use crate::audio::{self, Direction};
use crate::games;
use crate::install::Progress;
use crate::settings::Suppression;
use bukospeak_core::MAX_VOLUME;
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::ui::client::Client;
use crate::ui::kit::menu::{Item, Menu};
use crate::ui::kit::{self, InlineField, InlineFieldEvent, InputLevel, Slider, SliderEvent, Spinner, Switch, h_flex, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

#[derive(Clone, Debug, PartialEq)]
struct Choice {
    id: Option<String>,
    title: SharedString,
    missing: bool,
}

fn choices(devices: impl IntoIterator<Item = (String, String)>, kept: Option<&str>) -> Vec<Choice> {
    let standard = Choice { id: None, title: "Standard".into(), missing: false };
    let there = devices.into_iter().map(|(id, name)| Choice { id: Some(id), title: name.into(), missing: false });
    let mut choices: Vec<Choice> = std::iter::once(standard).chain(there).collect();
    if let Some(kept) = kept
        && !choices.iter().any(|choice| choice.id.as_deref() == Some(kept))
    {
        choices.push(Choice { id: Some(kept.to_owned()), title: "Nicht angeschlossen".into(), missing: true });
    }
    choices
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opened {
    Device(Direction),
    Suppression,
    Games,
}

const SUPPRESSIONS: [(Suppression, &str); 4] = [
    (Suppression::Off, "Aus"),
    (Suppression::Standard, "Standard"),
    (Suppression::NvidiaDereverbDenoiser, "Rauschen und Hall"),
    (Suppression::NvidiaStudioVoice, "Studio Voice"),
];

pub struct Settings {
    client: Entity<Client>,
    server: Entity<InlineField>,
    inputs: Vec<Choice>,
    outputs: Vec<Choice>,
    volume: Entity<Slider>,
    programs: Vec<String>,
    level: Entity<InputLevel>,
    menu: Option<(Opened, Entity<Menu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
    _listing: Task<()>,
}

impl Settings {
    pub fn new(client: Entity<Client>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let read = client.read(cx);
        let server = read.settings.server.clone();
        let kept = |direction| read.settings.device_id(direction).map(str::to_owned);
        let (kept_input, kept_output) = (kept(Direction::Input), kept(Direction::Output));
        let most = (MAX_VOLUME * 100.0) as u16;
        let percent = read.settings.volume.min(most);
        let meter = read.call.meter().clone();

        let server = cx.new(|cx| InlineField::new(server, "server.example:9988", window, cx));
        let level = cx.new(|cx| InputLevel::new(meter, cx));
        let volume = cx.new(|cx| Slider::new("Lautstärke", f32::from(percent), 0., f32::from(most), 5., cx));
        let subscriptions = vec![
            cx.observe(&client, |_, _, cx| cx.notify()),
            cx.subscribe(&server, |settings, _, InlineFieldEvent(server), cx| {
                settings.client.update(cx, |client, cx| client.set_server(server.clone(), cx));
            }),
            cx.subscribe(&volume, |settings, _, event: &SliderEvent, cx| {
                settings.client.update(cx, |client, cx| client.set_volume(event.value as u16, event.done, cx));
            }),
        ];
        let host = std::sync::Arc::clone(client.read(cx).host());
        let listing = cx.spawn(async move |settings, cx| {
            // Listing asks the sound server and the system: not on the window's thread.
            let (inputs, outputs, programs) = cx
                .background_spawn(async move {
                    let list = |direction| {
                        host.devices(direction).unwrap_or_else(|error| {
                            tracing::warn!("{error}");
                            Vec::new()
                        })
                    };
                    (list(Direction::Input), list(Direction::Output), host.programs())
                })
                .await;
            let _ = settings.update(cx, |settings, cx| {
                let named = |devices: Vec<audio::Device>| devices.into_iter().map(|device| (device.id.to_string(), device.name));
                let read = settings.client.read(cx);
                settings.inputs = choices(named(inputs), read.settings.device_id(Direction::Input));
                settings.outputs = choices(named(outputs), read.settings.device_id(Direction::Output));
                settings.programs = programs;
                cx.notify();
            });
        });
        Self {
            programs: Vec::new(),
            inputs: kept_choice(kept_input),
            outputs: kept_choice(kept_output),
            client,
            server,
            volume,
            level,
            menu: None,
            _subscriptions: subscriptions,
            _listing: listing,
        }
    }

    fn render_device(&self, direction: Direction, cx: &mut Context<Self>) -> AnyElement {
        let (id, choices) = match direction {
            Direction::Input => ("input", &self.inputs),
            Direction::Output => ("output", &self.outputs),
        };
        let kept = self.client.read(cx).settings.device_id(direction);
        let chosen =
            choices.iter().find(|choice| choice.id.as_deref() == kept).map_or_else(|| "Standard".into(), |choice| choice.title.clone());
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::Device(direction)).map(|(_, menu, _)| menu);
        kit::menu_row(
            id,
            "Gerät",
            chosen,
            open,
            cx.listener(move |settings, _, window, cx| settings.open_devices(direction, window, cx)),
            cx,
        )
    }

    fn open_devices(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let choices = match direction {
            Direction::Input => self.inputs.clone(),
            Direction::Output => self.outputs.clone(),
        };
        let kept = self.client.read(cx).settings.device_id(direction).map(str::to_owned);
        let client = self.client.downgrade();
        let menu = Menu::open(window, cx, |menu| {
            let item = |choice: &Choice| {
                let (client, id) = (client.clone(), choice.id.clone());
                Item::new(choice.title.clone(), move |_, cx| {
                    let _ = client.update(cx, |client, cx| client.set_device(direction, id.clone(), cx));
                })
                .checked(choice.id == kept)
                .disabled(choice.missing)
            };
            let (standard, devices) = choices.split_first().expect("the default is a choice");
            let menu = menu.item(item(standard));
            let menu = if devices.is_empty() { menu } else { menu.separator() };
            devices.iter().fold(menu, |menu, choice| menu.item(item(choice)))
        });
        self.show(Opened::Device(direction), menu, cx);
    }

    fn show(&mut self, opened: Opened, menu: Entity<Menu>, cx: &mut Context<Self>) {
        let closed = cx.subscribe(&menu, |settings, _, _: &DismissEvent, cx| {
            settings.menu = None;
            cx.notify();
        });
        self.menu = Some((opened, menu, closed));
        cx.notify();
    }

    fn render_suppression(&self, cx: &mut Context<Self>) -> AnyElement {
        let client = self.client.read(cx);
        let chosen =
            SUPPRESSIONS.iter().find(|(suppression, _)| *suppression == client.settings.suppression).map_or("", |(_, title)| title);
        let installing = client.nvidia.as_ref().and_then(|nvidia| nvidia.installing).map(|progress| match progress {
            Progress::Preparing { done, total } | Progress::Fetching { done, total } => (done * 100).checked_div(total).unwrap_or(0),
        });
        let tokens = cx.tokens();
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::Suppression).map(|(_, menu, _)| menu);
        let value = h_flex().gap(theme::TIGHT).child(chosen).children(installing.map(|percent| {
            h_flex()
                .gap(theme::TIGHT)
                .font_family(theme::MONO)
                .typeset(theme::TEXT_SM)
                .text_color(tokens.text_faint)
                .child(Spinner::new(theme::ICON_SM).color(tokens.speaking))
                .child(format!("{percent} %"))
        }));
        kit::menu_row(
            "suppression",
            "Rauschfilter",
            value,
            open,
            cx.listener(|settings, _, window, cx| settings.open_suppressions(window, cx)),
            cx,
        )
    }

    fn open_suppressions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let read = self.client.read(cx);
        let (chosen, nvidia) = (read.settings.suppression, read.nvidia.is_some());
        let client = self.client.downgrade();
        let menu = Menu::open(window, cx, |menu| {
            let item = |&(suppression, title): &(Suppression, &'static str)| {
                let client = client.clone();
                Item::new(title, move |_, cx| {
                    let _ = client.update(cx, |client, cx| client.set_suppression(suppression, cx));
                })
                .checked(suppression == chosen)
            };
            let (built_in, nvidias) = SUPPRESSIONS.split_at(2);
            let menu = built_in.iter().fold(menu, |menu, choice| menu.item(item(choice)));
            match nvidia {
                true => nvidias.iter().fold(menu.separator().heading("NVIDIA"), |menu, choice| menu.item(item(choice))),
                false => menu,
            }
        });
        self.show(Opened::Suppression, menu, cx);
    }

    fn render_games(&self, cx: &mut Context<Self>) -> AnyElement {
        let games = &self.client.read(cx).settings.games;
        let chosen = match games.as_slice() {
            [] => "Keinem".to_owned(),
            games => games.iter().map(|game| games::title(game)).collect::<Vec<_>>().join(", "),
        };
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::Games).map(|(_, menu, _)| menu);
        kit::menu_row("games", "Beitreten bei", chosen, open, cx.listener(|settings, _, window, cx| settings.open_games(window, cx)), cx)
    }

    fn open_games(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kept = self.client.read(cx).settings.games.clone();
        let running: Vec<String> =
            self.programs.iter().filter(|program| !kept.iter().any(|game| game.eq_ignore_ascii_case(program))).cloned().collect();
        let client = self.client.downgrade();
        let menu = Menu::open(window, cx, |menu| {
            let item = |program: &String, on: bool| {
                let (client, program) = (client.clone(), program.clone());
                Item::new(games::title(&program).to_owned(), move |_, cx| {
                    let _ = client.update(cx, |client, cx| client.set_game(&program, !on, cx));
                })
                .checked(on)
            };
            let menu = kept.iter().fold(menu, |menu, game| menu.item(item(game, true)));
            match running.is_empty() {
                true => menu,
                false => running.iter().fold(menu.separator().heading("Läuft gerade"), |menu, program| menu.item(item(program, false))),
            }
        });
        self.show(Opened::Games, menu, cx);
    }

    fn render_level(&self, cx: &App) -> AnyElement {
        kit::row("Pegel", cx).child(self.level.clone()).into_any_element()
    }

    fn render_volume(&self, cx: &mut Context<Self>) -> AnyElement {
        let percent = self.client.read(cx).settings.volume;
        kit::row("Lautstärke", cx)
            // The rail, not the knob's room around it, starts a gap after the label.
            .child(div().flex_1().ml(-theme::SLIDER_KNOB / 2.).child(self.volume.clone()))
            .child(
                div()
                    .w(theme::VOLUME_VALUE)
                    .text_right()
                    .font_family(theme::MONO)
                    .typeset(theme::TEXT_SM)
                    .text_color(cx.tokens().text_muted)
                    .child(format!("{percent} %")),
            )
            .into_any_element()
    }

    fn render_switch(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        set: fn(&mut Client, bool, &mut Context<Client>),
        cx: &App,
    ) -> AnyElement {
        let client = self.client.downgrade();
        kit::row(label, cx)
            .child(Switch::new(id, label, on).on_change(move |on, _, cx| {
                let _ = client.update(cx, |client, cx| set(client, on, cx));
            }))
            .into_any_element()
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let echo = self.client.read(cx).settings.echo_cancellation;
        let microphone = [
            self.render_device(Direction::Input, cx),
            self.render_level(cx),
            self.render_suppression(cx),
            self.render_switch("echo", "Echounterdrückung", echo, Client::set_echo_cancellation, cx),
        ];
        let output = [self.render_device(Direction::Output, cx), self.render_volume(cx)];
        let start = self.client.read(cx).settings.start_with_system;
        let automatic =
            [self.render_switch("start", "Mit dem System starten", start, Client::set_start_with_system, cx), self.render_games(cx)];
        let connection = [kit::row("Server", cx).child(self.server.clone()).into_any_element()];
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .px(theme::GUTTER)
                .pt(theme::GAP)
                .pb(theme::GUTTER)
                .gap(theme::SECTION)
                .child(kit::group("Mikrofon", microphone, cx))
                .child(kit::group("Ausgabe", output, cx))
                .child(kit::group("Automatisch", automatic, cx))
                .child(kit::group("Verbindung", connection, cx)),
        )
    }
}

fn kept_choice(kept: Option<String>) -> Vec<Choice> {
    let title = kept.clone().unwrap_or_else(|| "Standard".into()).into();
    vec![Choice { id: kept, title, missing: false }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_comes_first_and_a_kept_device_not_there_last() {
        let ids = |kept: Option<&str>| {
            let choices = choices([("first".to_owned(), "First".to_owned())], kept);
            choices.into_iter().map(|choice| (choice.id, choice.missing)).collect::<Vec<_>>()
        };
        let there = [(None, false), (Some("first".to_owned()), false)];
        assert_eq!(ids(None), there);
        assert_eq!(ids(Some("first")), there);
        assert_eq!(ids(Some("second")), [there[0].clone(), there[1].clone(), (Some("second".to_owned()), true)]);
    }
}

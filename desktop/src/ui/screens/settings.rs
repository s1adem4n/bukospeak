use std::path::Path;
use std::sync::Arc;

use crate::audio::{self, Direction};
use crate::games::{self, Chosen, Listing, Program};
use crate::install::Progress;
use crate::settings::Suppression;
use bukospeak_core::MAX_VOLUME;
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::ui::client::Client;
use crate::ui::glyph::Glyph;
use crate::ui::kit::menu::{Entries, Item, Menu, Place, Popover};
use crate::ui::kit::{
    self, AppIcon, Button, Icon, InlineField, InlineFieldEvent, InputLevel, Size, Slider, SliderEvent, Spinner, Switch, Variant, h_flex,
    v_flex,
};
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
    AddGame,
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
    programs: Listing,
    level: Entity<InputLevel>,
    menu: Option<(Opened, Entity<Menu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
    _devices: Task<()>,
    _programs: Task<()>,
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
        let host = Arc::clone(client.read(cx).host());
        let devices = cx.spawn(async move |settings, cx| {
            // Listing asks the sound server: not on the window's thread.
            let (inputs, outputs) = cx
                .background_spawn(async move {
                    let list = |direction| {
                        host.devices(direction).unwrap_or_else(|error| {
                            tracing::warn!("{error}");
                            Vec::new()
                        })
                    };
                    (list(Direction::Input), list(Direction::Output))
                })
                .await;
            let _ = settings.update(cx, |settings, cx| {
                let named = |devices: Vec<audio::Device>| devices.into_iter().map(|device| (device.id.to_string(), device.name));
                let read = settings.client.read(cx);
                settings.inputs = choices(named(inputs), read.settings.device_id(Direction::Input));
                settings.outputs = choices(named(outputs), read.settings.device_id(Direction::Output));
                cx.notify();
            });
        });
        let mut settings = Self {
            programs: Listing::default(),
            inputs: kept_choice(kept_input),
            outputs: kept_choice(kept_output),
            client,
            server,
            volume,
            level,
            menu: None,
            _subscriptions: subscriptions,
            _devices: devices,
            _programs: Task::ready(()),
        };
        settings._programs = settings.watch_programs(cx);
        settings
    }

    /// Lists the programs now and then every few seconds, so what runs shows as it starts.
    fn watch_programs(&self, cx: &mut Context<Self>) -> Task<()> {
        let host = Arc::clone(self.client.read(cx).host());
        cx.spawn(async move |settings, cx| {
            loop {
                let Ok(chosen) = settings.read_with(cx, |settings, cx| settings.client.read(cx).settings.games.clone()) else {
                    return;
                };
                let host = host.clone();
                // Listing asks the system and reads programs' files: not on the window's thread.
                let programs = cx.background_spawn(async move { host.programs(&chosen) }).await;
                if settings.update(cx, |settings, cx| settings.listed(programs, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(games::LOOK).await;
            }
        })
    }

    fn listed(&mut self, programs: Listing, cx: &mut Context<Self>) {
        self.programs = programs;
        if let Some((Opened::AddGame, menu, _)) = &self.menu {
            let entries = self.games_to_add(cx);
            menu.update(cx, |menu, cx| menu.set_entries(entries, cx));
        }
        cx.notify();
    }

    /// Chooses `game` or lets it go, shown at once; the listing that follows confirms it.
    fn set_game(&mut self, game: &str, on: bool, cx: &mut Context<Self>) {
        self.client.update(cx, |client, cx| client.set_game(game, on, cx));
        if on {
            self.programs.choose(game)
        } else {
            self.programs.unchoose(game)
        }
        self._programs = self.watch_programs(cx);
        cx.notify();
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
        let menu = Menu::open(window, cx, {
            let item = |choice: &Choice| {
                let (client, id) = (client.clone(), choice.id.clone());
                Item::new(choice.title.clone(), move |_, cx| {
                    let _ = client.update(cx, |client, cx| client.set_device(direction, id.clone(), cx));
                })
                .checked(choice.id == kept)
                .disabled(choice.missing)
            };
            let (standard, devices) = choices.split_first().expect("the default is a choice");
            devices.iter().fold(Entries::default().item(item(standard)).separator(), |entries, choice| entries.item(item(choice)))
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
        let menu = Menu::open(window, cx, {
            let item = |&(suppression, title): &(Suppression, &'static str)| {
                let client = client.clone();
                Item::new(title, move |_, cx| {
                    let _ = client.update(cx, |client, cx| client.set_suppression(suppression, cx));
                })
                .checked(suppression == chosen)
            };
            let (built_in, nvidias) = SUPPRESSIONS.split_at(2);
            let entries = built_in.iter().fold(Entries::default(), |entries, choice| entries.item(item(choice)));
            match nvidia {
                true => nvidias.iter().fold(entries.separator().heading("NVIDIA"), |entries, choice| entries.item(item(choice))),
                false => entries,
            }
        });
        self.show(Opened::Suppression, menu, cx);
    }

    /// The games chosen, one row each, and the row that adds one.
    fn render_games(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> =
            self.programs.chosen.iter().enumerate().map(|(index, chosen)| self.render_chosen(index, chosen, cx)).collect();
        rows.push(self.render_add_game(cx));
        rows
    }

    fn render_chosen(&self, index: usize, chosen: &Chosen, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let Program { game, title, icon, .. } = &chosen.program;
        let row: SharedString = format!("game-{index}").into();
        let game = game.clone();
        let remove = Button::new(("remove-game", index), Variant::Ghost)
            .size(Size::Small)
            .square()
            .icon(Glyph::Close)
            .name("Entfernen")
            .on_click(cx.listener(move |settings, _, _, cx| settings.set_game(&game, false, cx)));
        // Where the pointer is, the button that removes the game takes the dot's place at the end.
        let end = h_flex()
            .flex_none()
            .relative()
            .size(theme::CONTROL_SM)
            .justify_end()
            .children(chosen.runs.then(|| div().group_hover(row.clone(), |style| style.opacity(0.)).child(kit::dot(tokens.speaking))))
            .child(div().absolute().inset_0().opacity(0.).group_hover(row.clone(), |style| style.opacity(1.)).child(remove));
        kit::row_frame()
            .group(row)
            .gap(theme::INLINE)
            .child(AppIcon::new(title.clone(), icon.clone(), theme::app_icon::MD))
            .child(div().flex_1().min_w_0().truncate().text_color(tokens.text).child(title.clone()))
            .child(end)
            .into_any_element()
    }

    fn render_add_game(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::AddGame).map(|(_, menu, _)| menu);
        let mark = div()
            .flex_none()
            .size(theme::app_icon::MD.size)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::app_icon::MD.radius)
            .border_1()
            .border_dashed()
            .border_color(tokens.seat)
            .child(Icon::new(Glyph::Add).size(theme::ICON_SM));
        let row = kit::row_button("add-game")
            .gap(theme::INLINE)
            .child(mark)
            .child("Spiel hinzufügen")
            .on_click(cx.listener(|settings, _, window, cx| settings.open_games_to_add(window, cx)));
        div().relative().child(row).children(open.map(|menu| Popover::new(menu, Place::Under))).into_any_element()
    }

    fn open_games_to_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.games_to_add(cx);
        let menu = Menu::open_searchable("Suchen", window, cx, entries);
        self.show(Opened::AddGame, menu, cx);
    }

    /// The programs running and not chosen: the games in launchers' libraries, then the others
    /// with their files' names.
    fn games_to_add(&self, cx: &Context<Self>) -> Entries {
        let settings = cx.entity().downgrade();
        let item = |program: &Program| {
            let (settings, game) = (settings.clone(), program.game.clone());
            Item::new(program.title.clone(), move |_, cx| {
                let _ = settings.update(cx, |settings, cx| settings.set_game(&game, true, cx));
            })
            .icon(AppIcon::new(program.title.clone(), program.icon.clone(), theme::app_icon::SM))
        };
        let (in_library, others): (Vec<&Program>, Vec<&Program>) = self.programs.others.iter().partition(|program| program.in_library);
        let entries = in_library.into_iter().fold(Entries::default().heading("Spiele"), |entries, program| entries.item(item(program)));
        others.into_iter().fold(entries.separator().heading("Andere Programme"), |entries, program| {
            let file = Path::new(&program.game).file_name().unwrap_or_default().to_string_lossy().into_owned();
            entries.item(item(program).detail(file))
        })
    }

    fn render_level(&self, cx: &App) -> AnyElement {
        kit::row("Pegel", cx).child(self.level.clone()).into_any_element()
    }

    fn render_volume(&self, cx: &mut Context<Self>) -> AnyElement {
        let percent = self.client.read(cx).settings.volume;
        kit::row("Lautstärke", cx)
            // The rail, not the knob's room around it, starts a gap after the label.
            .child(div().flex_1().ml(-theme::slider::KNOB / 2.).child(self.volume.clone()))
            .child(
                div()
                    .w(theme::slider::VALUE)
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
        let speaker = [self.render_device(Direction::Output, cx), self.render_volume(cx)];
        let games = self.render_games(cx);
        let start = self.client.read(cx).settings.start_with_system;
        let app = [
            kit::row("Server", cx).child(self.server.clone()).into_any_element(),
            self.render_switch("start", "Mit dem System starten", start, Client::set_start_with_system, cx),
        ];
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .px(theme::GUTTER)
                .pt(theme::GAP)
                .pb(theme::GUTTER)
                .gap(theme::SECTION)
                .child(kit::group("Mikrofon", microphone, cx))
                .child(kit::group("Lautsprecher", speaker, cx))
                .child(kit::group("Beitreten, wenn läuft", games, cx))
                .child(kit::group("App", app, cx)),
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

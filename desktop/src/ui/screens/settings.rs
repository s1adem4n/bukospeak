//! The settings: the microphone, the output, and who one is where, each kept as soon as it is
//! changed: the fields once Enter is pressed or the focus goes, the rest at once.

use crate::audio::{self, Direction};
use crate::nvidia::Progress;
use crate::settings::Suppression;
use bukospeak_core::MAX_VOLUME;
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::ui::client::Client;
use crate::ui::kit::menu::{Item, Menu, Place, Popover};
use crate::ui::kit::{self, InlineField, InlineFieldEvent, InputLevel, Look, Slider, SliderEvent, Spinner, Switch, h_flex, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

/// A device to choose: the system's default for None.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    id: Option<String>,
    title: SharedString,
    /// Kept but not there now: shown as chosen, not to be chosen again.
    missing: bool,
}

/// The choices for a direction from its devices' ids and names: the default first, then the
/// devices there are, then the one kept if it isn't among them.
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

/// A menu of the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opened {
    Device(Direction),
    Suppression,
}

/// The choices of noise suppression, by their titles: the app's own, then NVIDIA's, which an RTX
/// card is offered under NVIDIA's name.
const SUPPRESSIONS: [(Suppression, &str); 4] = [
    (Suppression::Off, "Aus"),
    (Suppression::Standard, "Standard"),
    (Suppression::NvidiaDereverbDenoiser, "Rauschen und Hall"),
    (Suppression::NvidiaStudioVoice, "Studio Voice"),
];

pub struct Settings {
    client: Entity<Client>,
    name: Entity<InlineField>,
    server: Entity<InlineField>,
    room_key: Entity<InlineField>,
    /// The choices of microphones and of outputs; until the system listed its devices, only
    /// the one kept.
    inputs: Vec<Choice>,
    outputs: Vec<Choice>,
    volume: Entity<Slider>,
    level: Entity<InputLevel>,
    /// The menu open, and which it is.
    menu: Option<(Opened, Entity<Menu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
    _listing: Task<()>,
}

impl Settings {
    pub fn new(client: Entity<Client>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let read = client.read(cx);
        let (name, server, room_key) = (read.settings.name.clone(), read.settings.server.clone(), read.room_key().to_owned());
        let kept = |direction| read.settings.device_id(direction).map(str::to_owned);
        let (kept_input, kept_output) = (kept(Direction::Input), kept(Direction::Output));
        let most = (MAX_VOLUME * 100.0) as u16;
        let percent = read.settings.volume.min(most);
        let meter = read.call.meter().clone();

        let name = cx.new(|cx| InlineField::new(name, "Name", Look::Plain, window, cx).required());
        let server = cx.new(|cx| InlineField::new(server, "server.example:9988", Look::Mono, window, cx));
        let room_key = cx.new(|cx| InlineField::new(room_key, "Nicht gesetzt", Look::Secret, window, cx));
        let level = cx.new(|cx| InputLevel::new(meter, cx));
        let volume = cx.new(|cx| Slider::new("Lautstärke", f32::from(percent), 0., f32::from(most), 5., cx));
        let subscriptions = vec![
            cx.observe(&client, |_, _, cx| cx.notify()),
            keep(&name, &client, Client::set_name, cx),
            keep(&server, &client, Client::set_server, cx),
            keep(&room_key, &client, Client::set_room_key, cx),
            cx.subscribe(&volume, |settings, _, event: &SliderEvent, cx| {
                settings.client.update(cx, |client, cx| client.set_volume(event.value as u16, event.done, cx));
            }),
        ];
        let host = std::sync::Arc::clone(client.read(cx).host());
        let listing = cx.spawn(async move |settings, cx| {
            let list = move |direction| {
                host.devices(direction).unwrap_or_else(|error| {
                    tracing::warn!("{error}");
                    Vec::new()
                })
            };
            // Listing asks the sound server: not on the window's thread.
            let (inputs, outputs) = cx.background_spawn(async move { (list(Direction::Input), list(Direction::Output)) }).await;
            // The settings may be closed before the system answered.
            let _ = settings.update(cx, |settings, cx| {
                let named = |devices: Vec<audio::Device>| devices.into_iter().map(|device| (device.id.to_string(), device.name));
                let read = settings.client.read(cx);
                settings.inputs = choices(named(inputs), read.settings.device_id(Direction::Input));
                settings.outputs = choices(named(outputs), read.settings.device_id(Direction::Output));
                cx.notify();
            });
        });
        Self {
            inputs: kept_choice(kept_input),
            outputs: kept_choice(kept_output),
            client,
            name,
            server,
            room_key,
            volume,
            level,
            menu: None,
            _subscriptions: subscriptions,
            _listing: listing,
        }
    }

    /// Shows the room key the keyring kept, which came after the settings opened.
    pub fn read_room_key(&mut self, cx: &mut Context<Self>) {
        let key = self.client.read(cx).room_key().to_owned();
        self.room_key.update(cx, |field, cx| field.set_value(key, cx));
    }

    /// A row that chooses a device from a menu, which the row opens below its end: the device's
    /// name, which its group titles, and for the microphone how loud it is at the end.
    fn render_device(&self, direction: Direction, cx: &mut Context<Self>) -> AnyElement {
        let (id, choices) = match direction {
            Direction::Input => ("input", &self.inputs),
            Direction::Output => ("output", &self.outputs),
        };
        let kept = self.client.read(cx).settings.device_id(direction);
        let chosen =
            choices.iter().find(|choice| choice.id.as_deref() == kept).map_or_else(|| "Standard".into(), |choice| choice.title.clone());
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::Device(direction)).map(|(_, menu, _)| menu);
        // The level sits where other rows show their value: the row reads as they do.
        let level = (direction == Direction::Input).then(|| self.level.clone().into_any_element());
        div()
            .relative()
            .child(
                kit::menu_row(id, div().truncate().child(chosen), level, open.is_some(), cx)
                    .on_click(cx.listener(move |settings, _, window, cx| settings.open_devices(direction, window, cx))),
            )
            .children(open.map(|menu| Popover::new(menu, Place::BelowEnd)))
            .into_any_element()
    }

    /// Opens the menu of `direction`'s devices, the one kept checked.
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
                    // The window may be gone with the choice.
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

    /// Shows `menu` until it is dismissed.
    fn show(&mut self, opened: Opened, menu: Entity<Menu>, cx: &mut Context<Self>) {
        let closed = cx.subscribe(&menu, |settings, _, _: &DismissEvent, cx| {
            settings.menu = None;
            cx.notify();
        });
        self.menu = Some((opened, menu, closed));
        cx.notify();
    }

    /// The row that chooses the noise suppression from a menu, and tells how far installing
    /// NVIDIA's effects got.
    fn render_suppression(&self, cx: &mut Context<Self>) -> AnyElement {
        let client = self.client.read(cx);
        let chosen =
            SUPPRESSIONS.iter().find(|(suppression, _)| *suppression == client.settings.suppression).map_or("", |(_, title)| title);
        let installing = client.nvidia.as_ref().and_then(|nvidia| nvidia.installing).map(|progress| match progress {
            Progress::Preparing { done, total } | Progress::Fetching { done, total } => (done * 100).checked_div(total).unwrap_or(0),
        });
        let tokens = cx.tokens();
        let open = self.menu.as_ref().filter(|(shown, ..)| *shown == Opened::Suppression).map(|(_, menu, _)| menu);
        let value = h_flex().gap_1p5().text_color(tokens.text_muted).child(chosen).children(installing.map(|percent| {
            h_flex()
                .gap_1()
                .font_family(theme::MONO)
                .typeset(theme::TEXT_SM)
                .text_color(tokens.text_faint)
                .child(Spinner::new(theme::ICON_SM).color(tokens.speaking))
                .child(format!("{percent} %"))
        }));
        div()
            .relative()
            .child(
                kit::menu_row("suppression", "Rauschfilter", Some(value.into_any_element()), open.is_some(), cx)
                    .on_click(cx.listener(|settings, _, window, cx| settings.open_suppressions(window, cx))),
            )
            .children(open.map(|menu| Popover::new(menu, Place::BelowEnd)))
            .into_any_element()
    }

    /// Opens the menu of noise suppressions, the one chosen checked.
    fn open_suppressions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let read = self.client.read(cx);
        let (chosen, nvidia) = (read.settings.suppression, read.nvidia.is_some());
        let client = self.client.downgrade();
        let menu = Menu::open(window, cx, |menu| {
            let item = |&(suppression, title): &(Suppression, &'static str)| {
                let client = client.clone();
                Item::new(title, move |_, cx| {
                    // The window may be gone with the choice.
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

    /// How loud the others are: the slider from the panel's inset, its value at the end.
    fn render_volume(&self, cx: &mut Context<Self>) -> AnyElement {
        let percent = self.client.read(cx).settings.volume;
        kit::row_frame()
            // The rail, not the knob's room around it, starts where the labels do.
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
                // The window may be gone with the click.
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
            self.render_suppression(cx),
            self.render_switch("echo", "Echounterdrückung", echo, Client::set_echo_cancellation, cx),
        ];
        let output = [self.render_device(Direction::Output, cx), self.render_volume(cx)];
        let account = [
            kit::row("Name", cx).child(self.name.clone()).into_any_element(),
            kit::row("Server", cx).child(self.server.clone()).into_any_element(),
            kit::row("Raumschlüssel", cx).child(self.room_key.clone()).into_any_element(),
        ];
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .px(theme::GUTTER)
                .pt_3()
                .pb(theme::GUTTER)
                .gap_5()
                .child(kit::group("Mikrofon", microphone, cx))
                .child(kit::group("Ausgabe", output, cx))
                .child(kit::group("Konto", account, cx)),
        )
    }
}

/// Until the system listed its devices, only the device kept is known, by its id.
fn kept_choice(kept: Option<String>) -> Vec<Choice> {
    let title = kept.clone().unwrap_or_else(|| "Standard".into()).into();
    vec![Choice { id: kept, title, missing: false }]
}

/// Keeps what `field` was changed to, by `set`.
fn keep(
    field: &Entity<InlineField>,
    client: &Entity<Client>,
    set: fn(&mut Client, String, &mut Context<Client>),
    cx: &mut Context<Settings>,
) -> Subscription {
    let client = client.clone();
    cx.subscribe(field, move |_, _, InlineFieldEvent(value), cx| client.update(cx, |client, cx| set(client, value.clone(), cx)))
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

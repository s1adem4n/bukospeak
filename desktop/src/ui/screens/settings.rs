//! The settings: who one is and where, the audio, and the server, each kept as soon as it is
//! changed: the fields once Enter is pressed or the focus goes, the rest at once.

use bukospeak_core::MAX_VOLUME;
use bukospeak_desktop::audio::{self, Direction};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::ui::client::Client;
use crate::ui::glyph::Glyph;
use crate::ui::kit::{self, Avatar, Button, InlineField, InlineFieldEvent, InputLevel, Look, Variant};
use crate::ui::theme::{self, ActiveTokens};

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

pub struct Settings {
    client: Entity<Client>,
    name: Entity<InlineField>,
    server: Entity<InlineField>,
    room_key: Entity<InlineField>,
    /// The choices of microphones and of outputs; until the system listed its devices, only
    /// the one kept.
    inputs: Vec<Choice>,
    outputs: Vec<Choice>,
    volume: Entity<SliderState>,
    level: Entity<InputLevel>,
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

        let name = cx.new(|cx| InlineField::new(name, "Name", Look::Title, window, cx).required());
        let server = cx.new(|cx| InlineField::new(server, "server.example:9988", Look::Mono, window, cx));
        let room_key = cx.new(|cx| InlineField::new(room_key, "Nicht gesetzt", Look::Secret, window, cx));
        let level = cx.new(|cx| InputLevel::new(meter, cx));
        let volume = cx.new(|_| SliderState::new().min(0.0).max(f32::from(most)).step(5.0).default_value(f32::from(percent)));
        let subscriptions = vec![
            cx.observe(&client, |_, _, cx| cx.notify()),
            keep(&name, &client, Client::set_name, cx),
            keep(&server, &client, Client::set_server, cx),
            keep(&room_key, &client, Client::set_room_key, cx),
            cx.subscribe(&volume, |settings, _, event: &SliderEvent, cx| {
                let (value, done) = match event {
                    SliderEvent::Change(value) => (value, false),
                    SliderEvent::Release(value) => (value, true),
                };
                settings.client.update(cx, |client, cx| client.set_volume(value.end() as u16, done, cx));
            }),
        ];
        let listing = cx.spawn(async move |settings, cx| {
            let list = |direction| {
                audio::devices(direction).unwrap_or_else(|error| {
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
            _subscriptions: subscriptions,
            _listing: listing,
        }
    }

    /// Shows the room key the keyring kept, which came after the settings opened.
    pub fn read_room_key(&mut self, cx: &mut Context<Self>) {
        let key = self.client.read(cx).room_key().to_owned();
        self.room_key.update(cx, |field, cx| field.set_value(key, cx));
    }

    fn render_profile(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client.read(cx);
        let tokens = cx.tokens();
        kit::panel(cx)
            .flex()
            .items_center()
            .gap_3()
            .p(theme::PANEL_INSET)
            .child(Avatar::new(client.settings.name.clone(), theme::AVATAR_LG))
            .child(
                v_flex().flex_1().min_w_0().child(self.name.clone()).child(
                    div()
                        .truncate()
                        .font_family(theme::MONO)
                        .text_size(theme::TEXT_SM)
                        .text_color(tokens.text_faint)
                        .child(client.settings.server.clone()),
                ),
            )
    }

    /// A row that chooses a device from a menu: the row opens it, below its end.
    fn render_device(&self, direction: Direction, cx: &mut Context<Self>) -> AnyElement {
        let (id, label, choices) = match direction {
            Direction::Input => ("input", "Mikrofon", self.inputs.clone()),
            Direction::Output => ("output", "Ausgabe", self.outputs.clone()),
        };
        let kept = self.client.read(cx).settings.device_id(direction).map(str::to_owned);
        let chosen = choices.iter().find(|choice| choice.id == kept).map_or_else(|| "Standard".into(), |choice| choice.title.clone());
        let tokens = cx.tokens();
        let client = self.client.clone();
        Button::new(id, Variant::Ghost)
            .w_full()
            .h(theme::ROW)
            .px(theme::PANEL_INSET - theme::PANEL_PAD)
            .justify_between()
            .gap_3()
            .font_weight(FontWeight::NORMAL)
            .child(div().flex_none().text_color(tokens.text).child(label))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1p5()
                    .when(direction == Direction::Input, |value| value.child(div().mr_1().child(self.level.clone())))
                    .child(div().min_w_0().truncate().child(chosen))
                    .child(Icon::from(Glyph::Choose).size(theme::ICON_SM).text_color(tokens.text_faint)),
            )
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let item = |choice: &Choice| {
                    let client = client.clone();
                    let id = choice.id.clone();
                    PopupMenuItem::new(choice.title.clone()).checked(choice.id == kept).disabled(choice.missing).on_click(
                        move |_, _, cx| {
                            client.update(cx, |client, cx| client.set_device(direction, id.clone(), cx));
                        },
                    )
                };
                let (standard, devices) = choices.split_first().expect("the default is a choice");
                let menu = menu.min_w(theme::MENU).item(item(standard));
                let menu = if devices.is_empty() { menu } else { menu.separator() };
                devices.iter().fold(menu, |menu, choice| menu.item(item(choice)))
            })
            .into_any_element()
    }

    fn render_audio(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = &self.client.read(cx).settings;
        let tokens = cx.tokens();
        let (percent, noise, echo) = (settings.volume, settings.noise_suppression, settings.echo_cancellation);
        let volume = kit::row("Lautstärke", cx).child(
            h_flex().flex_none().w(theme::VOLUME).gap_3().child(div().flex_1().child(Slider::new(&self.volume))).child(
                div()
                    .w(theme::VOLUME_VALUE)
                    .text_right()
                    .font_family(theme::MONO)
                    .text_size(theme::TEXT_SM)
                    .text_color(tokens.text_muted)
                    .child(format!("{percent} %")),
            ),
        );
        kit::list(
            [
                self.render_device(Direction::Input, cx),
                self.render_device(Direction::Output, cx),
                volume.into_any_element(),
                self.render_switch("noise", "Rauschunterdrückung", noise, Client::set_noise_suppression, cx),
                self.render_switch("echo", "Echounterdrückung", echo, Client::set_echo_cancellation, cx),
            ],
            cx,
        )
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
            .child(Switch::new(id).checked(on).on_change(move |on, _, cx| {
                // The window may be gone with the click.
                let _ = client.update(cx, |client, cx| set(client, *on, cx));
            }))
            .into_any_element()
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let server = kit::list(
            [
                kit::row("Adresse", cx).child(self.server.clone()).into_any_element(),
                kit::row("Raumschlüssel", cx).child(self.room_key.clone()).into_any_element(),
            ],
            cx,
        );
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .px(theme::GAP)
                .pb(theme::GAP)
                .gap(theme::GAP)
                .child(self.render_profile(cx))
                .child(kit::section("Audio", cx))
                .child(self.render_audio(cx))
                .child(kit::section("Server", cx))
                .child(server),
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

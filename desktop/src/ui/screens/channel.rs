//! The channel: a panel of who is in it, from outside or in the call, and beneath it the dock,
//! which tells how the call is and holds the button to join or, in the call, to mute and leave.
//! Between them a release waiting for the call to end, which can be installed at once.

use std::time::Duration;

use bukospeak_desktop::call::Phase;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};

use crate::ui::client::{Client, Missing};
use crate::ui::glyph::Glyph;
use crate::ui::kit::{self, Avatar, Button, Variant};
use crate::ui::text;
use crate::ui::theme::{self, ActiveTokens};
use crate::update::{State, Updater};

/// Where the channel sends the window.
pub enum ChannelEvent {
    Settings,
    /// Something is missing to join, which the welcome asks for.
    Setup(Missing),
}

pub struct Channel {
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    /// Shows the call's time anew each second, while a call is up.
    clock: Option<Task<()>>,
    _client: Subscription,
    _updater: Option<Subscription>,
}

impl EventEmitter<ChannelEvent> for Channel {}

impl Channel {
    pub fn new(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |channel, _, cx| {
            channel.follow_clock(cx);
            cx.notify();
        });
        let updated = updater.as_ref().map(|updater| cx.observe(updater, |_, _, cx| cx.notify()));
        let mut channel = Self { client, updater, clock: None, _client: observed, _updater: updated };
        channel.follow_clock(cx);
        channel
    }

    /// Runs the clock while a call is up, and only then.
    fn follow_clock(&mut self, cx: &mut Context<Self>) {
        let up = self.client.read(cx).call.since().is_some();
        if !up {
            self.clock = None;
        } else if self.clock.is_none() {
            self.clock = Some(cx.spawn(async move |channel, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if channel.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
    }

    fn join(&mut self, cx: &mut Context<Self>) {
        match self.client.read(cx).missing() {
            Some(missing) => cx.emit(ChannelEvent::Setup(missing)),
            None => self.client.update(cx, |client, cx| client.join(cx)),
        }
    }

    fn render_members(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client.read(cx);
        let call = &client.call;
        let members: Vec<Member> = if call.is_in_call() {
            let you = call.you();
            call.members()
                .iter()
                .map(|member| {
                    Member {
                        id: member.id.0,
                        name: member.name.clone().into(),
                        speaks: call.speaking().contains(&member.id),
                        // One's own as set here, before the server tells it back.
                        muted: if Some(member.id) == you { call.is_muted() } else { member.muted },
                        reconnecting: member.reconnecting,
                    }
                })
                .collect()
        } else {
            call.present()
                .iter()
                .map(|member| Member {
                    id: member.id.0,
                    name: member.name.clone().into(),
                    speaks: false,
                    muted: member.muted,
                    reconnecting: member.reconnecting,
                })
                .collect()
        };
        let tokens = cx.tokens();
        let head = h_flex()
            .flex_none()
            .h(theme::HEAD)
            .px(theme::PANEL_INSET)
            .gap_2()
            .border_b_1()
            .border_color(tokens.hairline)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Kanal"))
            .child(kit::chip(members.len().to_string(), cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_right()
                    .truncate()
                    .font_family(theme::MONO)
                    .text_size(theme::TEXT_SM)
                    .text_color(tokens.text_faint)
                    .child(client.settings.server.clone()),
            );
        let body = if members.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2p5()
                .text_color(tokens.text_muted)
                .child(kit::tile(Glyph::Users, cx))
                .child("Niemand im Kanal")
                .into_any_element()
        } else {
            let reconnecting = *call.phase() == Phase::Reconnecting;
            div()
                .id("members")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(theme::PANEL_PAD)
                .flex()
                .flex_col()
                .when(reconnecting, |list| list.opacity(theme::RECONNECTING_OPACITY))
                .children(members.into_iter().map(|member| member.render(cx)))
                .into_any_element()
        };
        kit::panel(cx).flex_1().min_h_0().flex().flex_col().child(head).child(body)
    }

    /// One line of what went wrong beside the call: its notice, else the settings' trouble.
    fn render_notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let client = self.client.read(cx);
        let message = match (client.call.notice(), client.trouble) {
            (Some(notice), _) => text::notice(notice),
            (None, Some(trouble)) => text::trouble(trouble),
            (None, None) => return None,
        };
        let tokens = cx.tokens();
        Some(
            kit::panel(cx)
                .flex_none()
                .flex()
                .items_center()
                .gap_2p5()
                .h(theme::ROW)
                .px(theme::PANEL_INSET)
                .child(Icon::from(Glyph::Warning).size(theme::ICON_SM).text_color(tokens.warning))
                .child(div().min_w_0().truncate().child(message)),
        )
    }

    /// A release downloaded, which waits for the call to end, unless installed at once.
    fn render_update(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let updater = self.updater.clone()?;
        let State::Ready { version } = updater.read(cx).state() else {
            return None;
        };
        let tokens = cx.tokens();
        Some(
            kit::panel(cx)
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .p(theme::PANEL_PAD)
                .pl(theme::PANEL_INSET)
                .child(Icon::from(Glyph::Download).size(theme::ICON).text_color(tokens.speaking))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().font_weight(FontWeight::MEDIUM).child(format!("Update {version} bereit")))
                        .child(div().truncate().text_size(theme::TEXT_SM).text_color(tokens.text_muted).child("Nach dem Verlassen")),
                )
                .child(
                    Button::new("update", Variant::Raised)
                        .label("Jetzt updaten")
                        .on_click(move |_, _, cx| updater.update(cx, |updater, cx| updater.install(cx))),
                ),
        )
    }

    /// How the call is, and what to do with it.
    fn render_dock(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client.read(cx);
        let call = &client.call;
        let tokens = cx.tokens();
        let (status, color, label): (SharedString, _, _) = match call.phase() {
            Phase::Idle => ("Nicht verbunden".into(), tokens.text_faint, tokens.text_muted),
            Phase::Joining => ("Verbinde…".into(), tokens.text_faint, tokens.text_muted),
            Phase::Connected => ("Verbunden".into(), tokens.speaking, tokens.text),
            Phase::Reconnecting => ("Verbinde neu…".into(), tokens.warning, tokens.text),
            Phase::Failed(failure) => (text::failure(failure).into(), tokens.danger, tokens.danger),
        };
        let time = call.since().map(|since| clock(since.elapsed()));
        let status = h_flex()
            .h(theme::STATUS)
            .px(theme::PANEL_INSET - theme::PANEL_PAD)
            .gap_2p5()
            .child(kit::dot(color))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).text_color(label).child(status))
            .children(time.map(|time| div().font_family(theme::MONO).text_size(theme::TEXT_SM).text_color(tokens.text_faint).child(time)));
        let actions: Vec<AnyElement> = match call.phase() {
            Phase::Connected | Phase::Reconnecting => {
                let mute = if call.is_muted() {
                    Button::new("mute", Variant::Off).icon(Glyph::MicOff).label("Stumm").tooltip("Stummschaltung aufheben")
                } else {
                    Button::new("mute", Variant::Raised).icon(Glyph::Mic).label("Stummschalten")
                };
                vec![
                    mute.flex_1()
                        .keys(client.mute_keys)
                        .on_click(cx.listener(|channel, _, _, cx| channel.client.update(cx, |client, cx| client.toggle_mute(cx))))
                        .into_any_element(),
                    Button::new("leave", Variant::Danger)
                        .square()
                        .icon(Glyph::PhoneOff)
                        .tooltip("Verlassen")
                        .on_click(cx.listener(|channel, _, _, cx| channel.client.update(cx, |client, cx| client.leave(cx))))
                        .into_any_element(),
                ]
            }
            Phase::Failed(_) => vec![
                Button::new("settings", Variant::Raised)
                    .flex_1()
                    .label("Einstellungen")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ChannelEvent::Settings)))
                    .into_any_element(),
                Button::new("join", Variant::Primary)
                    .flex_1()
                    .label("Erneut beitreten")
                    .on_click(cx.listener(|channel, _, _, cx| channel.join(cx)))
                    .into_any_element(),
            ],
            Phase::Idle | Phase::Joining => vec![
                Button::new("join", Variant::Primary)
                    .flex_1()
                    .label("Beitreten")
                    .loading(*call.phase() == Phase::Joining)
                    .on_click(cx.listener(|channel, _, _, cx| channel.join(cx)))
                    .into_any_element(),
            ],
        };
        kit::panel(cx)
            .flex_none()
            .p(theme::PANEL_PAD)
            .flex()
            .flex_col()
            .gap(theme::PANEL_PAD)
            .child(status)
            .child(h_flex().gap(theme::PANEL_PAD).children(actions))
    }
}

impl Render for Channel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .px(theme::GAP)
            .pb(theme::GAP)
            .gap(theme::GAP)
            .child(self.render_members(cx))
            .children(self.render_notice(cx))
            .children(self.render_update(cx))
            .child(self.render_dock(cx))
    }
}

/// A member in the channel: the avatar in its speaking ring, the name, and at the end a mark
/// while muted or reconnecting, when the row is faded.
struct Member {
    id: u16,
    name: SharedString,
    speaks: bool,
    muted: bool,
    reconnecting: bool,
}

impl Member {
    fn render(self, cx: &App) -> impl IntoElement {
        let tokens = cx.tokens();
        let mark = match (self.reconnecting, self.muted) {
            (true, _) => Some((Glyph::Reconnecting, tokens.text_faint)),
            (false, true) => Some((Glyph::MicOff, tokens.danger)),
            (false, false) => None,
        };
        h_flex()
            .flex_none()
            .h(theme::MEMBER)
            .px(theme::PANEL_INSET - theme::PANEL_PAD)
            .gap_2p5()
            .when(self.reconnecting, |row| row.opacity(theme::RECONNECTING_OPACITY))
            .child(Avatar::new(self.name.clone(), theme::MEMBER_AVATAR).ring(u64::from(self.id), self.speaks))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(self.name))
            .children(mark.map(|(mark, color)| Icon::from(mark).size(theme::ICON_SM).text_color(color)))
    }
}

/// How long a call is up, as a clock shows it: minutes and seconds, the hours before once there
/// are any.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes:02}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_shows_hours_only_once_there_are_any() {
        assert_eq!(clock(Duration::from_millis(999)), "00:00");
        assert_eq!(clock(Duration::from_secs(42 * 60 + 17)), "42:17");
        assert_eq!(clock(Duration::from_secs(3600 + 12 * 60 + 4)), "1:12:04");
    }
}

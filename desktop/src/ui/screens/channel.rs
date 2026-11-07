use std::collections::HashSet;
use std::time::Duration;

use crate::call::Phase;
use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Context, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, SpringAnimation, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, ease_out_quint,
    pulsating_between,
};

use crate::ui::client::Client;
use crate::ui::glyph::Glyph;
use crate::ui::kit::{self, Avatar, Button, Icon, Size, Variant, h_flex, v_flex};
use crate::ui::marks::Mark;
use crate::ui::text;
use crate::ui::theme::{self, ActiveTokens, Seats, Tokens, Typeset};
use crate::update::{State, Updater};

const PULSE: Duration = Duration::from_millis(1400);

pub struct ChannelEvent;

pub struct Channel {
    client: Entity<Client>,
    updater: Option<Entity<Updater>>,
    clock: Option<Task<()>>,
    settled: HashSet<u16>,
    update_was_ready: bool,
    _client: Subscription,
    _updater: Option<Subscription>,
}

impl EventEmitter<ChannelEvent> for Channel {}

impl Channel {
    pub fn new(client: Entity<Client>, updater: Option<Entity<Updater>>, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |channel, _, cx| {
            channel.follow_clock(cx);
            let present = channel.member_ids(cx);
            channel.settled.retain(|id| present.contains(id));
            cx.notify();
        });
        let updated = updater.as_ref().map(|updater| cx.observe(updater, |_, _, cx| cx.notify()));
        let update_was_ready = updater.as_ref().is_some_and(|updater| matches!(updater.read(cx).state(), State::Ready { .. }));
        let mut channel =
            Self { client, updater, clock: None, settled: HashSet::new(), update_was_ready, _client: observed, _updater: updated };
        channel.settled = channel.member_ids(cx);
        channel.follow_clock(cx);
        channel
    }

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
        match self.client.read(cx).settings.is_complete() {
            true => self.client.update(cx, |client, cx| client.join(cx)),
            false => cx.emit(ChannelEvent),
        }
    }

    fn seats(&self, cx: &App) -> Vec<Seat> {
        let client = self.client.read(cx);
        let call = &client.call;
        if call.is_in_call() {
            let you = call.you();
            let mut seats: Vec<Seat> = call
                .members()
                .iter()
                .map(|member| {
                    Seat::Member(Member {
                        id: member.id.0,
                        name: member.name.clone().into(),
                        speaks: call.speaking().contains(&member.id),
                        // One's own as set here, before the server tells it back.
                        muted: if Some(member.id) == you { call.is_muted() } else { member.muted },
                        reconnecting: member.reconnecting,
                        arrives: !self.settled.contains(&member.id.0),
                    })
                })
                .collect();
            if seats.len() == 1 {
                seats.push(Seat::Empty);
            }
            seats
        } else {
            let present = call.present().iter().map(|member| {
                Seat::Member(Member {
                    id: member.id.0,
                    name: member.name.clone().into(),
                    speaks: false,
                    muted: member.muted,
                    reconnecting: member.reconnecting,
                    arrives: !self.settled.contains(&member.id.0),
                })
            });
            let you = Seat::Waiting { joining: *call.phase() == Phase::Joining };
            present.chain([you]).collect()
        }
    }

    fn member_ids(&self, cx: &App) -> HashSet<u16> {
        self.seats(cx)
            .into_iter()
            .filter_map(|seat| match seat {
                Seat::Member(member) => Some(member.id),
                Seat::Waiting { .. } | Seat::Empty => None,
            })
            .collect()
    }

    fn render_stage(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let seats = self.seats(cx);
        let layout = match seats.len() {
            0..=2 => theme::SEATS_TWO,
            3..=4 => theme::SEATS_FOUR,
            _ => theme::SEATS_MANY,
        };
        let centered = seats.len() <= 4;
        let full_rows = seats.len() > layout.columns;
        let rows: Vec<AnyElement> = seats
            .chunks(layout.columns)
            .map(|row| {
                // The last row's tiles keep the width of those above it.
                let spacers = if full_rows { layout.columns - row.len() } else { 0 };
                h_flex()
                    .w_full()
                    .justify_center()
                    .gap(theme::GAP)
                    .children(row.iter().map(|seat| seat.render(layout, cx)))
                    .children((0..spacers).map(|_| div().flex_1().max_w(layout.width)))
                    .into_any_element()
            })
            .collect();
        let reconnecting = *self.client.read(cx).call.phase() == Phase::Reconnecting;
        div()
            .id("stage")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(theme::GUTTER)
            .child(v_flex().py(theme::GAP).gap(theme::GAP).when(centered, |rows| rows.size_full().justify_center()).children(rows))
            .with_spring("reconnecting", SpringAnimation::new(theme::DIM).to(reconnecting), |stage, dim| {
                stage.opacity(dimmed(dim.clamp(0. ..=1.).0))
            })
    }

    fn render_update(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let updater = self.updater.clone()?;
        let State::Ready { version } = updater.read(cx).state() else {
            return None;
        };
        let tokens = cx.tokens();
        let callout = kit::callout(Icon::new(Glyph::Download).size(theme::ICON_SM).color(tokens.speaking), cx)
            .relative()
            .py(theme::PANEL_PAD)
            .pr(theme::PANEL_PAD)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(format!("Update {version} bereit")))
                    .child(div().truncate().typeset(theme::TEXT_SM).text_color(tokens.text_muted).child("Nach dem Verlassen")),
            )
            .child(
                Button::new("update", Variant::Raised)
                    .label("Jetzt updaten")
                    .on_click(move |_, _, cx| updater.update(cx, |updater, cx| updater.install(cx))),
            );
        if self.update_was_ready {
            return Some(callout.into_any_element());
        }
        Some(
            callout
                .with_animation("update", Animation::new(theme::APPEAR).with_easing(ease_out_quint()), |callout, delta| {
                    callout.opacity(delta).top(theme::RISE * (1. - delta))
                })
                .into_any_element(),
        )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let client = self.client.read(cx);
        let call = &client.call;
        let tokens = cx.tokens();
        let notice = match (call.notice(), client.trouble) {
            (Some(notice), _) => Some(text::notice(notice)),
            (None, Some(trouble)) => Some(text::trouble(trouble)),
            (None, None) => None,
        };
        let (status, dot, color): (SharedString, _, _) = match (call.phase(), notice) {
            (Phase::Failed(failure), _) => (text::failure(failure).into(), tokens.danger, tokens.danger),
            (_, Some(notice)) => (notice.into(), tokens.warning, tokens.text),
            (Phase::Idle, None) => return div().h(theme::TEXT_SM.line).into_any_element(),
            (Phase::Joining, None) => ("Verbinde…".into(), tokens.text_faint, tokens.text_muted),
            (Phase::Connected, None) => ("Verbunden".into(), tokens.speaking, tokens.text_muted),
            (Phase::Reconnecting, None) => ("Verbinde neu…".into(), tokens.warning, tokens.text),
        };
        let time = call.since().map(|since| clock(since.elapsed()));
        kit::led(kit::dot(dot))
            .typeset(theme::TEXT_SM)
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).text_color(color).child(status))
            .children(time.map(|time| div().font_family(theme::MONO).text_color(tokens.text_faint).child(time)))
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let client = self.client.read(cx);
        let call = &client.call;
        let actions: Vec<AnyElement> = match call.phase() {
            Phase::Connected | Phase::Reconnecting => {
                let mute = if call.is_muted() {
                    Button::new("mute", Variant::Off).icon(Glyph::MicOff).label("Stumm").tooltip("Stummschaltung aufheben")
                } else {
                    Button::new("mute", Variant::Raised).icon(Glyph::Mic).label("Stummschalten")
                };
                vec![
                    mute.size(Size::Large)
                        .flex_1()
                        .keys(client.mute_keys)
                        .on_click(cx.listener(|channel, _, _, cx| channel.client.update(cx, |client, cx| client.toggle_mute(cx))))
                        .into_any_element(),
                    Button::new("leave", Variant::Danger)
                        .size(Size::Large)
                        .square()
                        .icon(Glyph::PhoneOff)
                        .tooltip("Verlassen")
                        .on_click(cx.listener(|channel, _, _, cx| channel.client.update(cx, |client, cx| client.leave(cx))))
                        .into_any_element(),
                ]
            }
            phase @ (Phase::Idle | Phase::Joining | Phase::Failed(_)) => vec![
                Button::new("join", Variant::Primary)
                    .size(Size::Large)
                    .flex_1()
                    .label(if matches!(phase, Phase::Failed(_)) { "Erneut beitreten" } else { "Beitreten" })
                    .loading(*call.phase() == Phase::Joining)
                    .on_click(cx.listener(|channel, _, _, cx| channel.join(cx)))
                    .into_any_element(),
            ],
        };
        h_flex().gap(theme::GAP).children(actions)
    }
}

impl Render for Channel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().flex_1().min_h_0().child(self.render_stage(cx)).child(
            v_flex()
                .flex_none()
                .px(theme::GUTTER)
                .pb(theme::GUTTER)
                .gap(theme::GAP)
                .children(self.render_update(cx))
                .child(self.render_status(cx))
                .child(self.render_actions(cx)),
        )
    }
}

enum Seat {
    Member(Member),
    Waiting { joining: bool },
    Empty,
}

#[derive(Clone)]
struct Member {
    id: u16,
    name: SharedString,
    speaks: bool,
    muted: bool,
    reconnecting: bool,
    arrives: bool,
}

impl Seat {
    fn render(&self, layout: Seats, cx: &App) -> AnyElement {
        let tokens = *cx.tokens();
        let place = div().relative().flex_1().min_w_0().max_w(layout.width).h(layout.height);
        let outline = move || {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(layout.gap)
                .rounded(layout.radius)
                .border_1()
                .border_dashed()
                .border_color(tokens.seat)
        };
        match self {
            Self::Empty => place.child(outline()).into_any_element(),
            Self::Waiting { joining } => {
                let you = v_flex().items_center().gap(layout.gap).child(Avatar::unnamed(layout.avatar)).child(name_line(
                    "Du".into(),
                    layout,
                    tokens.text_muted,
                ));
                let you = if *joining {
                    let breathe = pulsating_between(theme::WAITING_OPACITY, 1.);
                    you.with_animation(
                        "joining",
                        // pulsating_between starts halfway up its range; from its low point the tile leaves its resting dimness without a jump.
                        Animation::new(PULSE).repeat().with_easing(move |delta| breathe((delta + 0.75) % 1.)),
                        |you, delta| you.opacity(delta),
                    )
                    .into_any_element()
                } else {
                    you.opacity(theme::WAITING_OPACITY).into_any_element()
                };
                place.id("you").mark("you").child(outline().child(you)).into_any_element()
            }
            Self::Member(member) => member.render(place, layout, tokens),
        }
    }
}

impl Member {
    fn render(&self, place: gpui::Div, layout: Seats, tokens: Tokens) -> AnyElement {
        let Self { id, name, speaks, muted, reconnecting, arrives } = self.clone();
        let badge = match (reconnecting, muted) {
            (true, _) => Some((Glyph::Reconnecting, tokens.text_muted)),
            (false, true) => Some((Glyph::MicOff, tokens.danger)),
            (false, false) => None,
        };
        let avatar = Avatar::new(name.clone(), layout.avatar);
        let avatar = match badge {
            Some((glyph, color)) => avatar.badge(glyph, color, tokens.tile[1]),
            None => avatar,
        };
        // Over the tile's hairline, so the thicker edge doesn't move what is inside.
        let spring = SpringAnimation::new(if speaks { theme::SPEAK_ON } else { theme::SPEAK_OFF }).to(speaks);
        let ring =
            div().absolute().inset_0().rounded(layout.radius).border(theme::SPEAKING_EDGE).border_color(tokens.speaking).with_spring(
                ("speaking", u64::from(id)),
                spring,
                |ring, lit| ring.opacity(lit.clamp(0. ..=1.).0),
            );
        let tile = v_flex()
            .relative()
            .size_full()
            .items_center()
            .justify_center()
            .gap(layout.gap)
            .rounded(layout.radius)
            .bg(theme::gradient(tokens.tile))
            .border_1()
            .border_color(tokens.hairline)
            .shadow(theme::depth::tile(&tokens))
            .child(avatar)
            .child(name_line(name, layout, tokens.text_muted))
            .child(ring)
            .with_spring(("reconnecting", u64::from(id)), SpringAnimation::new(theme::DIM).to(reconnecting), |tile, dim| {
                tile.opacity(dimmed(dim.clamp(0. ..=1.).0))
            });
        let place = place.id(("seat", u64::from(id))).mark(self.name.clone()).child(tile);
        if !arrives {
            return place.into_any_element();
        }
        place
            .with_animation(("appear", u64::from(id)), Animation::new(theme::APPEAR).with_easing(ease_out_quint()), |place, delta| {
                place.opacity(delta).top(theme::RISE * (1. - delta))
            })
            .into_any_element()
    }
}

fn name_line(name: SharedString, layout: Seats, color: gpui::Hsla) -> impl IntoElement {
    div().max_w_full().px(theme::TILE_INSET).truncate().typeset(layout.name).font_weight(FontWeight::MEDIUM).text_color(color).child(name)
}

fn dimmed(dim: f32) -> f32 {
    1. - (1. - theme::RECONNECTING_OPACITY) * dim
}

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

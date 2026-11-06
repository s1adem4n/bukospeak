//! The dialog when a server shows another certificate than the one pinned for it: both
//! fingerprints to compare, and the choice to trust the new one.

use bukospeak_core::proto::Fingerprint;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Window, div, ease_out_quint,
};

use crate::ui::chrome;
use crate::ui::client::Client;
use crate::ui::glyph::Glyph;
use crate::ui::kit::{self, Button, Icon, Size, Variant, h_flex, v_flex};
use crate::ui::theme::{self, ActiveTokens, Typeset};

pub fn dialog(client: &Entity<Client>, pinned: Fingerprint, presented: Fingerprint, window: &Window, cx: &App) -> AnyElement {
    let tokens = cx.tokens();
    let fingerprint = |label: &'static str, fingerprint: Fingerprint| {
        v_flex()
            .gap_1()
            // Its text on the dialog's inset, inside the dialog's padding and both edges.
            .px(theme::PANEL_INSET - theme::DIALOG_PAD - theme::HAIRLINE * 2.)
            .py_2p5()
            .rounded(theme::RADIUS_CONTROL_LG)
            .bg(tokens.inset)
            .border_1()
            .border_color(tokens.hairline)
            .child(div().typeset(theme::TEXT_SM).font_weight(FontWeight::MEDIUM).text_color(tokens.text_faint).child(label))
            .child(v_flex().font_family(theme::MONO).typeset(theme::TEXT_XS).children(lines(fingerprint).into_iter().map(|line| {
                // The ends of a fingerprint are where the eye compares it first.
                let edge = matches!(line.position, Position::First | Position::Last);
                h_flex()
                    .child(
                        div()
                            .font_weight(if edge { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                            .text_color(if edge { tokens.text } else { tokens.text_muted })
                            .child(line.head),
                    )
                    .child(div().text_color(tokens.text_muted).child(line.tail))
            })))
    };
    let (dismiss, trust) = (client.clone(), client.clone());
    chrome::rounded(div(), window)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .bg(tokens.scrim)
        .child(
            v_flex()
                .id("certificate")
                .relative()
                .w_full()
                .max_w(theme::DIALOG)
                .p(theme::DIALOG_PAD)
                .gap(theme::DIALOG_PAD)
                .rounded(theme::RADIUS_DIALOG)
                .bg(tokens.raised)
                .border_1()
                .border_color(tokens.edge)
                .shadow(theme::depth::overlay(tokens))
                .occlude()
                .child(
                    kit::led(Icon::new(Glyph::Warning).size(theme::ICON_SM).color(tokens.warning))
                        .h(theme::CONTROL)
                        .px(theme::PANEL_INSET - theme::DIALOG_PAD - theme::HAIRLINE)
                        .typeset(theme::TEXT_MD)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Zertifikat geändert"),
                )
                .child(fingerprint("Bisher", pinned))
                .child(fingerprint("Jetzt", presented))
                .child(
                    h_flex()
                        .gap(theme::DIALOG_PAD)
                        .child(
                            Button::new("cancel", Variant::Raised)
                                .size(Size::Large)
                                .flex_1()
                                .label("Abbrechen")
                                .on_click(move |_, _, cx| dismiss.update(cx, |client, cx| client.dismiss(cx))),
                        )
                        .child(
                            Button::new("trust", Variant::Danger)
                                .size(Size::Large)
                                .flex_1()
                                .label("Vertrauen")
                                .on_click(move |_, _, cx| trust.update(cx, |client, cx| client.trust(cx))),
                        ),
                )
                .with_animation("certificate", Animation::new(theme::APPEAR).with_easing(ease_out_quint()), |card, delta| {
                    card.opacity(delta).top(theme::RISE * (1. - delta))
                }),
        )
        .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    First,
    Middle,
    Last,
}

/// A line of a fingerprint, its first four pairs apart from the rest.
#[derive(Debug, PartialEq, Eq)]
struct Line {
    position: Position,
    head: SharedString,
    tail: SharedString,
}

/// A fingerprint in four lines of eight pairs, which the eye compares more easily than one.
fn lines(fingerprint: Fingerprint) -> Vec<Line> {
    let pairs: Vec<String> = fingerprint.0.iter().map(|byte| format!("{byte:02x}")).collect();
    let count = pairs.len().div_ceil(8);
    pairs
        .chunks(8)
        .enumerate()
        .map(|(index, line)| {
            let position = match index {
                0 => Position::First,
                index if index + 1 == count => Position::Last,
                _ => Position::Middle,
            };
            let (head, tail) = line.split_at(line.len().min(4));
            let tail = if tail.is_empty() { String::new() } else { format!(":{}", tail.join(":")) };
            Line { position, head: head.join(":").into(), tail: tail.into() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_shows_in_four_lines_of_eight_pairs_split_after_four() {
        let lines = lines(Fingerprint(std::array::from_fn(|index| index as u8)));
        let positions: Vec<Position> = lines.iter().map(|line| line.position).collect();
        assert_eq!(positions, [Position::First, Position::Middle, Position::Middle, Position::Last]);
        assert_eq!(lines[0].head, "00:01:02:03");
        assert_eq!(lines[0].tail, ":04:05:06:07");
        assert_eq!(lines[3].head, "18:19:1a:1b");
    }
}

//! The dialog when a server shows another certificate than the one pinned for it: both
//! fingerprints to compare, and the choice to trust the new one.

use bukospeak_core::proto::Fingerprint;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt, AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px,
};

use crate::ui::glyph::Glyph;
use crate::ui::{Main, theme};

impl Main {
    pub(super) fn render_certificate(&self, pinned: Fingerprint, presented: Fingerprint, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fingerprint = |label: &'static str, fingerprint: Fingerprint| {
            v_flex()
                .gap_1()
                .p_2p5()
                .rounded(theme.radius)
                .bg(theme.background)
                .child(div().text_size(theme::TEXT_XS).font_weight(FontWeight::MEDIUM).text_color(theme.muted_foreground).child(label))
                .child(div().text_size(theme::TEXT_XS).font_family(theme.mono_font_family.clone()).child(lines(fingerprint)))
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(theme.overlay)
            .child(
                v_flex()
                    .id("certificate")
                    .relative()
                    .w_full()
                    .max_w(px(340.))
                    .p_5()
                    .gap_4()
                    .rounded(theme.radius_lg)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_xl()
                    .occlude()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_size(theme::TEXT_LG)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Icon::from(Glyph::Warning).size(theme::ICON).text_color(theme.warning))
                            .child("Zertifikat geändert"),
                    )
                    .child(v_flex().gap_2().child(fingerprint("Bisher", pinned)).child(fingerprint("Jetzt", presented)))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .secondary()
                                    .rounded(ButtonRounded::Medium)
                                    .label("Abbrechen")
                                    .on_click(cx.listener(|main, _, _, cx| main.client.update(cx, |client, cx| client.dismiss(cx)))),
                            )
                            .child(
                                Button::new("trust")
                                    .danger()
                                    .rounded(ButtonRounded::Medium)
                                    .label("Vertrauen")
                                    .on_click(cx.listener(|main, _, _, cx| main.client.update(cx, |client, cx| client.trust(cx)))),
                            ),
                    )
                    .with_animation("certificate", Animation::new(theme::APPEAR).with_easing(gpui_kit::ease_out_quint()), |card, delta| {
                        card.opacity(delta).top(theme::SLIDE * (1. - delta))
                    }),
            )
            .into_any_element()
    }
}

/// A fingerprint in four lines of eight pairs, which the eye compares more easily than one.
fn lines(fingerprint: Fingerprint) -> SharedString {
    let pairs: Vec<String> = fingerprint.0.iter().map(|byte| format!("{byte:02x}")).collect();
    pairs.chunks(8).map(|line| line.join(":")).collect::<Vec<_>>().join("\n").into()
}

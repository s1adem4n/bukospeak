//! gpui-component's theme of one appearance, made from its tokens: the fields, menus, tooltips,
//! switches, sliders and scrollbars the app takes from gpui-component then look like its own
//! controls.

use std::rc::Rc;

use gpui_kit::Hsla;
use gpui_kit::component::{Colorize, ThemeConfig, ThemeConfigColors, ThemeMode};

use super::fonts::{MONO, SANS};
use super::{RADIUS_CONTROL, RADIUS_SMALL, TEXT, Tokens};

pub fn theme(tokens: &Tokens) -> Rc<ThemeConfig> {
    let color = |color: Hsla| Some(color.to_hex().into());
    // Some of its fields are gpui-component's own, so the colors are set one by one.
    let mut colors = ThemeConfigColors::default();
    colors.background = color(tokens.base);
    colors.foreground = color(tokens.text);
    colors.border = color(tokens.hairline);
    colors.input = color(tokens.edge);
    colors.ring = color(tokens.focus);
    colors.caret = color(tokens.text);
    colors.selection = color(tokens.selection);
    colors.muted = color(tokens.inset);
    colors.muted_foreground = color(tokens.text_muted);
    // A menu's hovered item, a list's hovered row.
    colors.accent = color(tokens.hover);
    colors.accent_foreground = color(tokens.text);
    colors.popover = color(tokens.floating);
    colors.popover_foreground = color(tokens.text);
    // A switch that is on, and the ghost buttons fields draw, such as a key's eye.
    colors.primary = color(tokens.text);
    colors.primary_hover = color(tokens.text);
    colors.primary_active = color(tokens.text);
    colors.primary_foreground = color(tokens.base);
    colors.secondary = color(tokens.inset);
    colors.secondary_hover = color(tokens.hover);
    colors.secondary_active = color(tokens.hover);
    colors.secondary_foreground = color(tokens.text);
    colors.switch = color(tokens.hover);
    colors.switch_thumb = color(tokens.raised);
    colors.slider_bar = color(tokens.text);
    colors.slider_thumb = color(tokens.primary[0]);
    colors.danger = color(tokens.danger);
    colors.danger_hover = color(tokens.danger_hover);
    colors.danger_active = color(tokens.danger);
    colors.danger_foreground = color(tokens.primary[0]);
    colors.warning = color(tokens.warning);
    colors.success = color(tokens.speaking);
    colors.title_bar = color(tokens.base);
    colors.title_bar_border = color(tokens.base.opacity(0.));
    colors.overlay = color(tokens.scrim);
    colors.scrollbar = color(tokens.base.opacity(0.));
    colors.scrollbar_thumb = color(tokens.edge);
    colors.scrollbar_thumb_hover = color(tokens.focus);
    Rc::new(ThemeConfig {
        name: if tokens.dark { "BukoSpeak Dark" } else { "BukoSpeak Light" }.into(),
        mode: if tokens.dark { ThemeMode::Dark } else { ThemeMode::Light },
        font_family: Some(SANS.into()),
        font_size: Some(f32::from(TEXT)),
        mono_font_family: Some(MONO.into()),
        mono_font_size: Some(f32::from(TEXT)),
        radius: Some(f32::from(RADIUS_SMALL) as usize),
        radius_lg: Some(f32::from(RADIUS_CONTROL) as usize),
        shadow: Some(false),
        colors,
        ..ThemeConfig::default()
    })
}

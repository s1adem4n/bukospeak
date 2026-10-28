//! A switch: on or off, flipped by a click, or by Enter or Space once it has the focus.

use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, BoxShadow, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement,
    Styled, Toggled, Window, div, px,
};

use crate::ui::theme::{self, ActiveTokens};

type ChangeHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    /// What it switches, for those who don't see the row it sits in.
    label: SharedString,
    on: bool,
    on_change: Option<ChangeHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Self {
        Self { id: id.into(), label: label.into(), on, on_change: None }
    }

    /// Called with what the switch is flipped to.
    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let on = self.on;
        let inset = (theme::SWITCH_HEIGHT - theme::SWITCH_KNOB) / 2.;
        div()
            .id(self.id)
            .role(Role::Switch)
            .aria_label(self.label)
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .tab_index(0)
            .flex_none()
            .flex()
            .items_center()
            .when(on, |track| track.justify_end())
            .w(theme::SWITCH_WIDTH)
            .h(theme::SWITCH_HEIGHT)
            .px(inset - px(1.))
            .rounded_full()
            .border_1()
            .border_color(tokens.hairline)
            .bg(if on { tokens.text } else { tokens.hover })
            .cursor_pointer()
            .focus_visible(|style| style.border_color(tokens.focus))
            .when_some(self.on_change, |track, on_change| track.on_click(move |_, window, cx| on_change(!on, window, cx)))
            .child(
                div()
                    .size(theme::SWITCH_KNOB)
                    .rounded_full()
                    .bg(tokens.raised)
                    .shadow(vec![BoxShadow::new(px(0.), px(1.), tokens.shadow).blur_radius(px(2.))]),
            )
    }
}

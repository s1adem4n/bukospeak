//! The field's line drawn: the text or its placeholder, what is selected and the cursor, the line
//! scrolled so the cursor stays in view; and the pointer on it, which, once pressed, goes on
//! selecting wherever it goes until released.

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ShapedLine, SharedString,
    Style, TextAlign, TextRun, UnderlineStyle, Window, fill, point, px, relative, size,
};

use super::{Layout, TextField, edit};
use crate::ui::theme::{self, ActiveTokens};

pub struct TextElement {
    field: Entity<TextField>,
}

impl TextElement {
    pub fn new(field: Entity<TextField>) -> Self {
        Self { field }
    }
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

pub struct Prepaint {
    line: ShapedLine,
    masked: bool,
    /// Where the line's start is drawn, and how far it is scrolled to get there.
    origin: gpui::Point<Pixels>,
    scroll: Pixels,
    line_height: Pixels,
    /// What of the field the text shows in, inside its inset.
    text: Bounds<Pixels>,
    selection: Option<Bounds<Pixels>>,
    cursor: Option<Bounds<Pixels>>,
    hitbox: Hitbox,
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style { flex_grow: 1., flex_shrink: 1., size: size(relative(1.).into(), relative(1.).into()), ..Style::default() };
        style.min_size.width = px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let field = self.field.read(cx);
        let tokens = cx.tokens();
        let style = window.text_style();
        let line_height = window.line_height();
        let content = field.text.content();
        let masked = field.masked() && !content.is_empty();
        let to_line = |offset: usize| if masked { edit::to_masked(content, offset) } else { offset };

        let (shown, color): (SharedString, _) = if content.is_empty() {
            (field.placeholder.clone(), tokens.text_faint)
        } else if masked {
            (edit::masked(content).into(), style.color)
        } else {
            (content.to_owned().into(), style.color)
        };
        let run = TextRun { len: shown.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        // What an input method composes is underlined until it is committed.
        let runs = match field.text.marked().filter(|_| !content.is_empty()) {
            Some(marked) => {
                let marked = to_line(marked.start)..to_line(marked.end);
                let underline = UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false };
                [
                    TextRun { len: marked.start, ..run.clone() },
                    TextRun { len: marked.len(), underline: Some(underline), ..run.clone() },
                    TextRun { len: shown.len() - marked.end, ..run },
                ]
                .into_iter()
                .filter(|run| run.len > 0)
                .collect()
            }
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(shown, font_size, &runs, None);

        let text = Bounds::new(
            point(bounds.left() + theme::FIELD_INSET, bounds.center().y - line_height / 2.),
            size((bounds.size.width - theme::FIELD_INSET * 2.).max(px(0.)), line_height),
        );
        let selection = field.text.selection();
        let cursor_x = if content.is_empty() { px(0.) } else { line.x_for_index(to_line(field.text.cursor())) };
        // Scrolled no further than the line reaches, and far enough that the cursor shows.
        let most = (line.width + theme::CURSOR - text.size.width).max(px(0.));
        let scroll = field.scroll.min(most).max(cursor_x + theme::CURSOR - text.size.width).min(cursor_x).max(px(0.));
        let origin = point(text.left() - scroll, text.top());

        let focused = field.focus.is_focused(window);
        let selection_bounds = (!selection.is_empty()).then(|| {
            let (start, end) = (line.x_for_index(to_line(selection.start)), line.x_for_index(to_line(selection.end)));
            Bounds::from_corners(point(origin.x + start, text.top()), point(origin.x + end, text.bottom()))
        });
        let cursor = (focused && field.cursor_shown && selection.is_empty())
            .then(|| Bounds::new(point(origin.x + cursor_x, text.top()), size(theme::CURSOR, line_height)));
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Prepaint { line, masked, origin, scroll, line_height, text, selection: selection_bounds, cursor, hitbox }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let tokens = cx.tokens();
        let (selection_color, cursor_color) = (tokens.selection, tokens.text);
        let focus = self.field.read(cx).focus.clone();
        let input_bounds = Bounds::new(prepaint.origin, size(prepaint.line.width.max(prepaint.text.size.width), prepaint.line_height));
        window.handle_input(&focus, ElementInputHandler::new(input_bounds, self.field.clone()), cx);
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        // The text is cut off at the inset, past which the line scrolls.
        window.with_content_mask(Some(gpui::ContentMask { bounds: prepaint.text }), |window| {
            if let Some(selection) = prepaint.selection {
                window.paint_quad(fill(selection, selection_color));
            }
            if let Err(error) = prepaint.line.paint(prepaint.origin, prepaint.line_height, TextAlign::Left, None, window, cx) {
                tracing::warn!("a field's text wasn't drawn: {error}");
            }
        });
        if let Some(cursor) = prepaint.cursor {
            window.paint_quad(fill(cursor, cursor_color));
        }

        let layout =
            Layout { line: prepaint.line.clone(), origin: prepaint.origin, line_height: prepaint.line_height, masked: prepaint.masked };
        let scroll = prepaint.scroll;
        self.field.update(cx, |field, _| {
            field.layout = Some(layout);
            field.scroll = scroll;
        });
        self.listen(&prepaint.hitbox, window);
    }
}

impl TextElement {
    /// Presses, drags and right clicks on the field; a press takes the pointer, so a drag goes
    /// on selecting past the field's edges.
    fn listen(&self, hitbox: &Hitbox, window: &mut Window) {
        window.on_mouse_event({
            let (field, hitbox) = (self.field.clone(), hitbox.clone());
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                    return;
                }
                match event.button {
                    MouseButton::Left => {
                        window.capture_pointer(hitbox.id);
                        field.update(cx, |field, cx| field.press(event.position, event.click_count, event.modifiers.shift, cx));
                    }
                    MouseButton::Right => field.update(cx, |field, cx| field.open_menu(event.position, window, cx)),
                    _ => {}
                }
            }
        });
        window.on_mouse_event({
            let (field, hitbox) = (self.field.clone(), hitbox.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) && event.pressed_button == Some(MouseButton::Left) {
                    field.update(cx, |field, cx| field.drag(event.position, cx));
                }
            }
        });
        let field = self.field.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                field.update(cx, |field, _| field.selecting = false);
            }
        });
    }
}

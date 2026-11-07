mod edit;
mod element;

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Pixels, Point, Render, ShapedLine, SharedString, Styled, Subscription, Task, UTF16Selection, Window,
    actions,
};

use crate::ui::kit::h_flex;
use crate::ui::kit::menu::{Item, Menu, Place, Popover};
use crate::ui::marks::Mark;
use crate::ui::theme;
use edit::{Kind, Text};
use element::TextElement;

actions!(
    text_field,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        WordLeft,
        WordRight,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectHome,
        SelectEnd,
        SelectAll,
        Cut,
        Copy,
        Paste,
        Undo,
        Redo,
        Confirm,
        ShowCharacterPalette
    ]
);

const CONTEXT: &str = "TextField";

pub fn init(cx: &mut App) {
    let field = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, field),
        KeyBinding::new("shift-backspace", Backspace, field),
        KeyBinding::new("delete", Delete, field),
        KeyBinding::new("left", Left, field),
        KeyBinding::new("right", Right, field),
        KeyBinding::new("shift-left", SelectLeft, field),
        KeyBinding::new("shift-right", SelectRight, field),
        KeyBinding::new("home", Home, field),
        KeyBinding::new("end", End, field),
        KeyBinding::new("shift-home", SelectHome, field),
        KeyBinding::new("shift-end", SelectEnd, field),
        KeyBinding::new("enter", Confirm, field),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-a", SelectAll, field),
            KeyBinding::new("cmd-x", Cut, field),
            KeyBinding::new("cmd-c", Copy, field),
            KeyBinding::new("cmd-v", Paste, field),
            KeyBinding::new("cmd-z", Undo, field),
            KeyBinding::new("cmd-shift-z", Redo, field),
            KeyBinding::new("alt-left", WordLeft, field),
            KeyBinding::new("alt-right", WordRight, field),
            KeyBinding::new("alt-shift-left", SelectWordLeft, field),
            KeyBinding::new("alt-shift-right", SelectWordRight, field),
            KeyBinding::new("alt-backspace", DeleteWordLeft, field),
            KeyBinding::new("alt-delete", DeleteWordRight, field),
            KeyBinding::new("cmd-left", Home, field),
            KeyBinding::new("cmd-right", End, field),
            KeyBinding::new("cmd-shift-left", SelectHome, field),
            KeyBinding::new("cmd-shift-right", SelectEnd, field),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, field),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-a", SelectAll, field),
            KeyBinding::new("ctrl-x", Cut, field),
            KeyBinding::new("ctrl-c", Copy, field),
            KeyBinding::new("ctrl-v", Paste, field),
            KeyBinding::new("ctrl-z", Undo, field),
            KeyBinding::new("ctrl-shift-z", Redo, field),
            KeyBinding::new("ctrl-left", WordLeft, field),
            KeyBinding::new("ctrl-right", WordRight, field),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, field),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, field),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, field),
            KeyBinding::new("ctrl-delete", DeleteWordRight, field),
            KeyBinding::new("ctrl-y", Redo, field),
        ]);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldEvent {
    Changed,
    Confirmed,
    Left,
}

struct Layout {
    line: ShapedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
}

pub struct TextField {
    text: Text,
    focus: FocusHandle,
    placeholder: SharedString,
    scroll: Pixels,
    layout: Option<Layout>,
    selecting: bool,
    cursor_shown: bool,
    blinking: Option<Task<()>>,
    menu: Option<(Entity<Menu>, Point<Pixels>, Subscription)>,
    _focus: [Subscription; 2],
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn new(value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let focused = cx.on_focus_in(&focus, window, |field, _, cx| field.blink(cx));
        let left = cx.on_focus_out(&focus, window, |field, _, _, cx| {
            field.blinking = None;
            field.selecting = false;
            cx.emit(TextFieldEvent::Left);
            cx.notify();
        });
        Self {
            text: Text::new(value),
            focus,
            placeholder: placeholder.into(),
            scroll: Pixels::ZERO,
            layout: None,
            selecting: false,
            cursor_shown: true,
            blinking: None,
            menu: None,
            _focus: [focused, left],
        }
    }

    pub fn value(&self) -> &str {
        self.text.content()
    }

    pub fn set_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.text = Text::new(value);
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.text.select_all();
        self.moved(cx);
    }

    fn blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_shown = true;
        self.blinking = Some(cx.spawn(async move |field, cx| {
            loop {
                cx.background_executor().timer(theme::BLINK).await;
                let blinked = field.update(cx, |field, cx| {
                    field.cursor_shown = !field.cursor_shown;
                    cx.notify();
                });
                if blinked.is_err() {
                    break;
                }
            }
        }));
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        if self.blinking.is_some() {
            self.blink(cx);
        }
        cx.notify();
    }

    fn edited(&mut self, changed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if changed {
            window.notify_text_input_changed(&self.focus);
            cx.emit(TextFieldEvent::Changed);
        }
        self.moved(cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.text.move_to(offset);
        self.moved(cx);
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.text.select_to(offset);
        self.moved(cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.text.selection();
        let to = if selection.is_empty() { self.text.previous_boundary(selection.start) } else { selection.start };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.text.selection();
        let to = if selection.is_empty() { self.text.next_boundary(selection.end) } else { selection.end };
        self.move_to(to, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.previous_word(self.text.cursor()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.next_word(self.text.cursor()), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.content().len(), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.previous_boundary(self.text.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.next_boundary(self.text.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.previous_word(self.text.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.next_word(self.text.cursor()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.content().len(), cx);
    }

    fn select_all_action(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.delete_back_to(self.text.previous_boundary(self.text.cursor()));
        self.edited(changed, window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.delete_on_to(self.text.next_boundary(self.text.cursor()));
        self.edited(changed, window, cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.delete_back_to(self.text.previous_word(self.text.cursor()));
        self.edited(changed, window, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.delete_on_to(self.text.next_word(self.text.cursor()));
        self.edited(changed, window, cx);
    }

    fn selected_text(&self) -> Option<&str> {
        let selection = self.text.selection();
        (!selection.is_empty()).then(|| &self.text.content()[selection])
    }

    fn cut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
            let changed = self.text.replace(None, "", Kind::Other);
            self.edited(changed, window, cx);
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
        }
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pasted) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let line = pasted.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
            let changed = self.text.replace(None, &line, Kind::Other);
            self.edited(changed, window, cx);
        }
    }

    fn cut_action(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.cut(window, cx);
    }

    fn copy_action(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(cx);
    }

    fn paste_action(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        self.paste(window, cx);
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.undo();
        self.edited(changed, window, cx);
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.text.redo();
        self.edited(changed, window, cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldEvent::Confirmed);
    }

    fn show_character_palette(&mut self, _: &ShowCharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else {
            return 0;
        };
        layout.line.closest_index_for_x(position.x - layout.origin.x)
    }

    fn x_of(&self, layout: &Layout, offset: usize) -> Pixels {
        layout.line.x_for_index(offset)
    }

    fn press(&mut self, position: Point<Pixels>, clicks: usize, shift: bool, cx: &mut Context<Self>) {
        let offset = self.offset_at(position);
        self.selecting = true;
        match clicks {
            1 if shift => self.text.select_to(offset),
            1 => self.text.move_to(offset),
            2 => self.text.select(self.text.word_at(offset)),
            _ => self.text.select_all(),
        }
        self.moved(cx);
    }

    fn drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.offset_at(position), cx);
        }
    }

    fn open_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_at(position);
        if !self.text.selection().contains(&offset) {
            self.text.move_to(offset);
        }
        self.focus(window, cx);
        let field = cx.entity().downgrade();
        let takes = self.selected_text().is_some();
        let menu = Menu::open(window, cx, |menu| {
            let item = |label: &'static str, edit: fn(&mut TextField, &mut Window, &mut Context<TextField>)| {
                let field = field.clone();
                Item::new(label, move |window, cx| {
                    let _ = field.update(cx, |field, cx| edit(field, window, cx));
                })
            };
            menu.item(item("Ausschneiden", |field, window, cx| field.cut(window, cx)).disabled(!takes))
                .item(item("Kopieren", |field, _, cx| field.copy(cx)).disabled(!takes))
                .item(item("Einfügen", |field, window, cx| field.paste(window, cx)))
                .separator()
                .item(item("Alles auswählen", |field, _, cx| field.select_all(cx)))
        });
        let closed = cx.subscribe(&menu, |field, _, _: &gpui::DismissEvent, cx| {
            field.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, position, closed));
        self.moved(cx);
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.text.range_from_utf16(&range);
        actual.replace(self.text.range_to_utf16(&range));
        Some(self.text.content()[range].to_owned())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let selection = self.text.selection();
        Some(UTF16Selection { range: self.text.range_to_utf16(&selection), reversed: self.text.reversed() })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.text.marked().map(|marked| self.text.range_to_utf16(&marked))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.text.unmark();
        cx.notify();
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = range.map(|range| self.text.range_from_utf16(&range));
        let changed = self.text.replace(range, text, Kind::Typing);
        self.edited(changed, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.map(|range| self.text.range_from_utf16(&range));
        // The selection is counted in UTF-16 within the text composed.
        let selected = selected.map(|selected| edit::from_utf16(text, selected.start)..edit::from_utf16(text, selected.end));
        self.text.compose(range, text, selected);
        self.edited(true, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.text.range_from_utf16(&range);
        let (start, end) = (self.x_of(layout, range.start), self.x_of(layout, range.end));
        Some(Bounds::from_corners(
            gpui::point(layout.origin.x + start, bounds.top()),
            gpui::point(layout.origin.x + end, bounds.top() + layout.line_height),
        ))
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.layout.as_ref().map(|_| self.text.to_utf16(self.offset_at(point)))
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .mark(self.placeholder.clone())
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::cut_action))
            .on_action(cx.listener(Self::copy_action))
            .on_action(cx.listener(Self::paste_action))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::show_character_palette))
            .size_full()
            .child(TextElement::new(cx.entity()))
            .children(self.menu.as_ref().map(|(menu, at, _)| Popover::new(menu, Place::At(*at))))
    }
}

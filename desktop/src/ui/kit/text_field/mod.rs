//! A field of one line of text: typed into, also through the system's input methods, selected by
//! keys and the pointer, cut, copied and pasted, undone and redone, and with a right click the
//! menu of those. A secret field shows dots, unless its eye shows the text.

mod edit;
mod element;

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Pixels, Point, Render, ShapedLine, SharedString, Styled, Subscription, Task, UTF16Selection, Window,
    actions,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::menu::{Item, Menu, Place, Popover};
use crate::ui::kit::{Button, Size, Variant, h_flex, icon_inset};
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
        /// Enter: what was typed is meant.
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
    // The clipboard, undoing, words and the line's ends by the keys each system has for them.
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

/// What a field tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldEvent {
    /// Its text changed.
    Changed,
    /// Enter was pressed in it.
    Confirmed,
    /// The focus left it, and what it opened.
    Left,
}

/// The line as last drawn, which the pointer and the input methods find characters on.
struct Layout {
    line: ShapedLine,
    /// Where the line's start was drawn, left of the field's edge by as far as it is scrolled.
    origin: Point<Pixels>,
    line_height: Pixels,
    /// Whether the line is the text masked.
    masked: bool,
}

pub struct TextField {
    text: Text,
    focus: FocusHandle,
    placeholder: SharedString,
    /// Whether the text is a secret, shown as dots, and whether its eye shows it anyway.
    secret: bool,
    revealed: bool,
    /// How far the line is scrolled left, so the cursor stays in view.
    scroll: Pixels,
    layout: Option<Layout>,
    /// Whether the pointer, pressed on the text, selects as it moves.
    selecting: bool,
    /// Whether the cursor shows now, while it blinks.
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
            secret: false,
            revealed: false,
            scroll: Pixels::ZERO,
            layout: None,
            selecting: false,
            cursor_shown: true,
            blinking: None,
            menu: None,
            _focus: [focused, left],
        }
    }

    /// Shows the text as dots, with an eye that shows it.
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn value(&self) -> &str {
        self.text.content()
    }

    /// Holds `value` from now, the cursor at its end and nothing to undo.
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

    /// Whether the text shows as dots now.
    fn masked(&self) -> bool {
        self.secret && !self.revealed
    }

    /// Shows the cursor and blinks it from now: it holds still while one types or moves it.
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

    /// The selection or the cursor moved.
    fn moved(&mut self, cx: &mut Context<Self>) {
        if self.blinking.is_some() {
            self.blink(cx);
        }
        cx.notify();
    }

    /// The text changed, or didn't, by an edit.
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

    /// Where the cursor goes for an arrow: off a selection to its end on that side, else a
    /// step that way.
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

    /// A secret's words aren't to be told apart by the cursor: they go to the line's ends.
    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.masked() { 0 } else { self.text.previous_word(self.text.cursor()) };
        self.move_to(to, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.masked() { self.text.content().len() } else { self.text.next_word(self.text.cursor()) };
        self.move_to(to, cx);
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
        let to = if self.masked() { 0 } else { self.text.previous_word(self.text.cursor()) };
        self.select_to(to, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.masked() { self.text.content().len() } else { self.text.next_word(self.text.cursor()) };
        self.select_to(to, cx);
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
        let to = if self.masked() { 0 } else { self.text.previous_word(self.text.cursor()) };
        let changed = self.text.delete_back_to(to);
        self.edited(changed, window, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        let to = if self.masked() { self.text.content().len() } else { self.text.next_word(self.text.cursor()) };
        let changed = self.text.delete_on_to(to);
        self.edited(changed, window, cx);
    }

    /// What may be taken out of the field: the selection, unless it is a secret shown as dots.
    fn selected_text(&self) -> Option<&str> {
        let selection = self.text.selection();
        (!selection.is_empty() && !self.masked()).then(|| &self.text.content()[selection])
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

    /// Pastes the clipboard's text, its lines joined by spaces: the field holds one.
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

    /// The offset of the text under `position` in the window, by the line as last drawn.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else {
            return 0;
        };
        let index = layout.line.closest_index_for_x(position.x - layout.origin.x);
        if layout.masked { edit::from_masked(self.text.content(), index) } else { index }
    }

    /// Where `offset` of the text lies along the line as last drawn, from the line's start.
    fn x_of(&self, layout: &Layout, offset: usize) -> Pixels {
        let index = if layout.masked { edit::to_masked(self.text.content(), offset) } else { offset };
        layout.line.x_for_index(index)
    }

    /// A press on the text: the cursor goes there, or a selection is extended there with Shift;
    /// two clicks select a word, three the whole line.
    fn press(&mut self, position: Point<Pixels>, clicks: usize, shift: bool, cx: &mut Context<Self>) {
        let offset = self.offset_at(position);
        self.selecting = true;
        match clicks {
            1 if shift => self.text.select_to(offset),
            1 => self.text.move_to(offset),
            2 if !self.masked() => self.text.select(self.text.word_at(offset)),
            _ => self.text.select_all(),
        }
        self.moved(cx);
    }

    fn drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.offset_at(position), cx);
        }
    }

    /// Opens the menu of cut, copy, paste and select all at `position`, the cursor moved there
    /// unless it was pressed on the selection.
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
                    // The field may be gone with its window.
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

    fn toggle_revealed(&mut self, cx: &mut Context<Self>) {
        self.revealed = !self.revealed;
        cx.notify();
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
        let eye = self.secret.then(|| {
            Button::new("reveal", Variant::Ghost)
                .size(Size::Small)
                .square()
                .unfocusable()
                // Its icon as far in from the field's end as the text from its start.
                .mr(theme::FIELD_INSET - theme::HAIRLINE - icon_inset(Size::Small))
                .icon(if self.revealed { Glyph::Hide } else { Glyph::Show })
                .tooltip(if self.revealed { "Verbergen" } else { "Anzeigen" })
                .on_click(cx.listener(|field, _, _, cx| field.toggle_revealed(cx)))
        });
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
            .children(eye)
            .children(self.menu.as_ref().map(|(menu, at, _)| Popover::new(menu, Place::At(*at))))
    }
}

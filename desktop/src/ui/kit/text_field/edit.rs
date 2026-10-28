//! A line of text being edited, apart from how it is drawn: what it holds, what of it is
//! selected, what an input method is composing, and the edits to undo. Offsets are bytes of its
//! UTF-8, on the boundaries of what a reader sees as one character.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// How an edit joins the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Typed: typing right after joins it, so an undo takes back a word, not a letter.
    Typing,
    /// Anything else: pasted, cut, deleted, composed. An undo step of its own.
    Other,
}

/// What a step of the history goes back to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    content: String,
    selection: Range<usize>,
}

#[derive(Debug, Default)]
pub struct Text {
    content: String,
    selection: Range<usize>,
    /// Whether the cursor is at the selection's start, which was extended to the left.
    reversed: bool,
    /// What an input method is composing, until it commits it.
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Whether the last edit was typed, which typing goes on joining.
    typing: bool,
}

impl Text {
    /// `content`, with the cursor at its end and no history.
    pub fn new(content: String) -> Self {
        let end = content.len();
        Self { content, selection: end..end, ..Self::default() }
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Whether the cursor is at the selection's start.
    pub fn reversed(&self) -> bool {
        self.reversed
    }

    /// Where the cursor is: the end of the selection it extends.
    pub fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    /// Puts the cursor at `offset`, selecting nothing.
    pub fn move_to(&mut self, offset: usize) {
        self.selection = offset..offset;
        self.reversed = false;
        self.typing = false;
    }

    /// Extends the selection from where it began to `offset`.
    pub fn select_to(&mut self, offset: usize) {
        let anchor = if self.reversed { self.selection.end } else { self.selection.start };
        self.reversed = offset < anchor;
        self.selection = if self.reversed { offset..anchor } else { anchor..offset };
        self.typing = false;
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.selection = range;
        self.reversed = false;
        self.typing = false;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.content.len());
    }

    /// The boundary before `offset`, or the start.
    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().map(|(index, _)| index).find(|index| *index < offset).unwrap_or(0)
    }

    /// The boundary after `offset`, or the end.
    pub fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).map(|(index, _)| index).find(|index| *index > offset).unwrap_or(self.content.len())
    }

    /// The start of the word before `offset`, past the spaces between.
    pub fn previous_word(&self, offset: usize) -> usize {
        self.content.unicode_word_indices().map(|(index, _)| index).rev().find(|index| *index < offset).unwrap_or(0)
    }

    /// The end of the word after `offset`, past the spaces between.
    pub fn next_word(&self, offset: usize) -> usize {
        self.content.unicode_word_indices().map(|(index, word)| index + word.len()).find(|end| *end > offset).unwrap_or(self.content.len())
    }

    /// The word, or the run of spaces or marks between words, around `offset`: what a double
    /// click selects.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let mut last = 0..0;
        for (index, part) in self.content.split_word_bound_indices() {
            last = index..index + part.len();
            if last.contains(&offset) {
                break;
            }
        }
        last
    }

    /// Replaces `range`, else what is being composed, else the selection, with `text`, and puts
    /// the cursor after it; false if nothing changed.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str, kind: Kind) -> bool {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        if range.is_empty() && text.is_empty() && self.marked.is_none() {
            return false;
        }
        self.remember(kind);
        self.content.replace_range(range.clone(), text);
        let end = range.start + text.len();
        self.selection = end..end;
        self.reversed = false;
        self.marked = None;
        true
    }

    /// Replaces as [`Self::replace`], and marks `text` as being composed, its part `selected`,
    /// in bytes within it, selected.
    pub fn compose(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        self.remember(Kind::Other);
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        let selected = selected.map_or(text.len()..text.len(), |selected| selected.start.min(text.len())..selected.end.min(text.len()));
        self.selection = range.start + selected.start..range.start + selected.end;
        self.reversed = false;
    }

    /// Commits what is being composed as it stands.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Deletes the selection, else from the cursor back to `to`; false if there was nothing.
    pub fn delete_back_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let cursor = self.cursor();
            self.selection = to.min(cursor)..cursor;
        }
        self.replace(None, "", Kind::Other)
    }

    /// Deletes the selection, else from the cursor on to `to`; false if there was nothing.
    pub fn delete_on_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let cursor = self.cursor();
            self.selection = cursor..to.max(cursor);
        }
        self.replace(None, "", Kind::Other)
    }

    /// Goes back to before the last edit; false if there is none.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.redo.push(current);
        true
    }

    /// Does again what the last undo took back; false if there is nothing.
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.undo.push(current);
        true
    }

    /// Keeps how the text is before an edit of `kind`, unless the edit joins the one before:
    /// typing joins typing, and whatever goes on with or commits a composition joins it, whose
    /// start was kept.
    fn remember(&mut self, kind: Kind) {
        let joins = self.marked.is_some() || (kind == Kind::Typing && self.typing && self.selection.is_empty());
        if !joins {
            self.undo.push(self.snapshot());
        }
        self.redo.clear();
        self.typing = kind == Kind::Typing;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { content: self.content.clone(), selection: self.selection.clone() }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.content = snapshot.content;
        self.selection = snapshot.selection;
        self.reversed = false;
        self.marked = None;
        self.typing = false;
    }

    /// `offset` in UTF-16 code units, as input methods count.
    pub fn to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    /// The byte offset of `utf16` code units in.
    pub fn offset_at_utf16(&self, utf16: usize) -> usize {
        from_utf16(&self.content, utf16)
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_at_utf16(range.start)..self.offset_at_utf16(range.end)
    }
}

/// The byte offset in `text` of `utf16` code units in.
pub fn from_utf16(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

/// What a secret field shows for each character of its text.
pub const MASK: char = '•';

/// The text as a secret field shows it: a dot for each character.
pub fn masked(content: &str) -> String {
    content.graphemes(true).map(|_| MASK).collect()
}

/// Where `offset` into `content` lies in its masked form.
pub fn to_masked(content: &str, offset: usize) -> usize {
    content.grapheme_indices(true).take_while(|(index, _)| *index < offset).count() * MASK.len_utf8()
}

/// Where `index` into the masked form of `content` lies in `content`.
pub fn from_masked(content: &str, index: usize) -> usize {
    content.grapheme_indices(true).nth(index / MASK.len_utf8()).map_or(content.len(), |(offset, _)| offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_selection_and_undoes_as_one() {
        let mut text = Text::new("server".into());
        text.select_all();
        for letter in ["h", "o", "s", "t"] {
            text.replace(None, letter, Kind::Typing);
        }
        assert_eq!(text.content(), "host");
        assert_eq!(text.selection(), 4..4);
        assert!(text.undo());
        assert_eq!(text.content(), "server");
        assert_eq!(text.selection(), 0..6);
        assert!(text.redo());
        assert_eq!(text.content(), "host");
        assert!(!text.redo());
    }

    #[test]
    fn a_paste_after_typing_is_a_step_of_its_own() {
        let mut text = Text::new(String::new());
        text.replace(None, "a", Kind::Typing);
        text.replace(None, "bc", Kind::Other);
        text.undo();
        assert_eq!(text.content(), "a");
        text.undo();
        assert_eq!(text.content(), "");
        assert!(!text.undo());
    }

    #[test]
    fn moving_the_cursor_ends_the_typing_an_undo_takes_back() {
        let mut text = Text::new(String::new());
        text.replace(None, "a", Kind::Typing);
        text.move_to(0);
        text.replace(None, "b", Kind::Typing);
        text.undo();
        assert_eq!(text.content(), "a");
    }

    #[test]
    fn deleting_goes_by_what_a_reader_sees_as_one_character() {
        // An e and its combining accent, then a flag of two code points.
        let mut text = Text::new("e\u{301}🇩🇪".into());
        assert!(text.delete_back_to(text.previous_boundary(text.cursor())));
        assert_eq!(text.content(), "e\u{301}");
        assert!(text.delete_back_to(text.previous_boundary(text.cursor())));
        assert_eq!(text.content(), "");
        assert!(!text.delete_back_to(0));
    }

    #[test]
    fn words_skip_the_spaces_between() {
        let text = Text::new("one two  three".into());
        assert_eq!(text.previous_word(14), 9);
        assert_eq!(text.previous_word(9), 4);
        assert_eq!(text.next_word(3), 7);
        assert_eq!(text.next_word(7), 14);
        assert_eq!(text.word_at(5), 4..7);
        assert_eq!(text.word_at(8), 7..9);
    }

    #[test]
    fn a_selection_extends_either_way_from_where_it_began() {
        let mut text = Text::new("abcdef".into());
        text.move_to(3);
        text.select_to(1);
        assert_eq!((text.selection(), text.cursor()), (1..3, 1));
        text.select_to(5);
        assert_eq!((text.selection(), text.cursor()), (3..5, 5));
    }

    #[test]
    fn composing_marks_until_committed() {
        let mut text = Text::new("a".into());
        text.compose(None, "k", None);
        text.compose(None, "か", Some(3..3));
        assert_eq!(text.content(), "aか");
        assert_eq!(text.marked(), Some(1..4));
        assert_eq!(text.selection(), 4..4);
        text.replace(None, "家", Kind::Other);
        assert_eq!(text.content(), "a家");
        assert_eq!(text.marked(), None);
        text.undo();
        assert_eq!(text.content(), "a");
    }

    #[test]
    fn utf16_offsets_count_surrogate_pairs_twice() {
        let text = Text::new("a😀b".into());
        assert_eq!(text.to_utf16(5), 3);
        assert_eq!(text.offset_at_utf16(3), 5);
        assert_eq!(text.range_from_utf16(&(1..3)), 1..5);
    }

    #[test]
    fn a_mask_shows_a_dot_for_each_character() {
        let content = "ä😀b";
        assert_eq!(masked(content), "•••");
        assert_eq!(to_masked(content, 2), 3);
        assert_eq!(from_masked(content, 6), 6);
        assert_eq!(from_masked(content, 9), content.len());
    }
}

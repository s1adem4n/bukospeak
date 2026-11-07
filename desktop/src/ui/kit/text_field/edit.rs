use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Typing,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    content: String,
    selection: Range<usize>,
}

#[derive(Debug, Default)]
pub struct Text {
    content: String,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    typing: bool,
}

impl Text {
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

    pub fn reversed(&self) -> bool {
        self.reversed
    }

    pub fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    pub fn move_to(&mut self, offset: usize) {
        self.selection = offset..offset;
        self.reversed = false;
        self.typing = false;
    }

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

    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().map(|(index, _)| index).find(|index| *index < offset).unwrap_or(0)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).map(|(index, _)| index).find(|index| *index > offset).unwrap_or(self.content.len())
    }

    pub fn previous_word(&self, offset: usize) -> usize {
        self.content.unicode_word_indices().map(|(index, _)| index).rev().find(|index| *index < offset).unwrap_or(0)
    }

    pub fn next_word(&self, offset: usize) -> usize {
        self.content.unicode_word_indices().map(|(index, word)| index + word.len()).find(|end| *end > offset).unwrap_or(self.content.len())
    }

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

    pub fn compose(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection.clone());
        self.remember(Kind::Other);
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        let selected = selected.map_or(text.len()..text.len(), |selected| selected.start.min(text.len())..selected.end.min(text.len()));
        self.selection = range.start + selected.start..range.start + selected.end;
        self.reversed = false;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn delete_back_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let cursor = self.cursor();
            self.selection = to.min(cursor)..cursor;
        }
        self.replace(None, "", Kind::Other)
    }

    pub fn delete_on_to(&mut self, to: usize) -> bool {
        if self.selection.is_empty() {
            let cursor = self.cursor();
            self.selection = cursor..to.max(cursor);
        }
        self.replace(None, "", Kind::Other)
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.redo.push(current);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.undo.push(current);
        true
    }

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

    pub fn to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

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
}

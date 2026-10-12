//! The message being written: a small multi-line editor.
//!
//! Pasted text arrives whole (bracketed paste) and is inserted as text, so a
//! newline inside a paste never sends the message — only Enter typed by a
//! person does. Widths are terminal columns: Hangul and CJK take two.

use unicode_width::UnicodeWidthChar as _;

#[derive(Default)]
pub struct Composer {
    text: String,
    /// Byte index into `text`, always on a char boundary.
    cursor: usize,
    history: Vec<String>,
    /// Position in `history` while recalling earlier messages.
    recall: Option<usize>,
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }

    pub fn insert_char(&mut self, c: char) {
        self.recall = None;
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert pasted text as-is, normalising line endings and tabs.
    pub fn insert_str(&mut self, pasted: &str) {
        self.recall = None;
        let clean = pasted.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
            self.text.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    /// To the start of the current line.
    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
    }

    /// To the end of the current line.
    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i);
    }

    /// Delete the word before the cursor (Ctrl-W).
    pub fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches([' ', '\n']);
        let start = trimmed.rfind([' ', '\n']).map_or(0, |i| i + 1);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.recall = None;
    }

    /// Take the message to send, remembering it for Up-arrow recall.
    pub fn take(&mut self) -> String {
        let message = self.text.trim().to_owned();
        if !message.is_empty() && self.history.last() != Some(&message) {
            self.history.push(message.clone());
        }
        self.clear();
        message
    }

    /// Up: an earlier message. Only while the composer holds a single line, so
    /// Up still moves through a multi-line draft's lines in the future.
    pub fn recall_older(&mut self) -> bool {
        if self.history.is_empty() || (self.recall.is_none() && !self.is_empty()) {
            return false;
        }
        let index = match self.recall {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.show_recalled(index);
        true
    }

    /// Down: back toward the newest message, then an empty composer.
    pub fn recall_newer(&mut self) -> bool {
        let Some(i) = self.recall else {
            return false;
        };
        if i + 1 < self.history.len() {
            self.show_recalled(i + 1);
        } else {
            self.clear();
        }
        true
    }

    fn show_recalled(&mut self, index: usize) {
        self.text = self.history[index].clone();
        self.cursor = self.text.len();
        self.recall = Some(index);
    }

    /// Rows as they wrap at `width` columns, and the cursor's (row, column).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows = vec![String::new()];
        let (mut row, mut col) = (0, 0);
        let mut cursor_at = None;
        for (i, c) in self.text.char_indices() {
            if i == self.cursor {
                cursor_at = Some((row, col));
            }
            if c == '\n' {
                rows.push(String::new());
                row += 1;
                col = 0;
                continue;
            }
            let w = c.width().unwrap_or(0);
            if col + w > width {
                rows.push(String::new());
                row += 1;
                col = 0;
                if i == self.cursor {
                    cursor_at = Some((row, col));
                }
            }
            rows[row].push(c);
            col += w;
        }
        let cursor = cursor_at.unwrap_or(if col >= width { (row + 1, 0) } else { (row, col) });
        if cursor.0 >= rows.len() {
            rows.push(String::new());
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Composer {
        let mut c = Composer::default();
        text.chars().for_each(|ch| c.insert_char(ch));
        c
    }

    #[test]
    fn a_paste_keeps_its_newlines_as_text() {
        let mut c = Composer::default();
        c.insert_str("fix the login bug\r\nerror log:\r\n\tTypeError");
        assert_eq!(c.text(), "fix the login bug\nerror log:\n    TypeError");
        assert_eq!(c.line_count(), 3);
    }

    #[test]
    fn editing_moves_by_whole_characters() {
        let mut c = typed("안녕");
        c.left();
        c.insert_char('!');
        assert_eq!(c.text(), "안!녕");
        c.backspace();
        c.backspace();
        assert_eq!(c.text(), "녕");
    }

    #[test]
    fn home_and_end_stay_on_the_current_line() {
        let mut c = typed("one\ntwo");
        c.home();
        c.insert_char('>');
        assert_eq!(c.text(), "one\n>two");
        c.end();
        c.insert_char('<');
        assert_eq!(c.text(), "one\n>two<");
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut c = typed("summarize this repo  ");
        c.delete_word();
        assert_eq!(c.text(), "summarize this ");
    }

    #[test]
    fn taking_a_message_trims_and_remembers_it() {
        let mut c = typed("  hello  ");
        assert_eq!(c.take(), "hello");
        assert!(c.is_empty());
        assert!(c.recall_older());
        assert_eq!(c.text(), "hello");
        assert!(c.recall_newer());
        assert!(c.is_empty());
    }

    #[test]
    fn up_does_not_replace_a_draft() {
        let mut c = typed("first");
        c.take();
        "draft".chars().for_each(|ch| c.insert_char(ch));
        assert!(!c.recall_older());
        assert_eq!(c.text(), "draft");
    }

    #[test]
    fn layout_wraps_hangul_by_columns() {
        // Each syllable is two columns: three fit in six.
        let c = typed("가나다라");
        let (rows, cursor) = c.layout(6);
        assert_eq!(rows, ["가나다", "라"]);
        assert_eq!(cursor, (1, 2));
    }

    #[test]
    fn layout_places_the_cursor_inside_the_text() {
        let mut c = typed("ab\ncd");
        c.left();
        let (rows, cursor) = c.layout(10);
        assert_eq!(rows, ["ab", "cd"]);
        assert_eq!(cursor, (1, 1));
    }

    #[test]
    fn a_cursor_at_a_full_row_moves_to_the_next() {
        let c = typed("abcd");
        let (rows, cursor) = c.layout(4);
        assert_eq!(cursor, (1, 0));
        assert_eq!(rows.len(), 2);
    }
}

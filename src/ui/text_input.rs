use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Single-line text input tracking a UTF-8 buffer and a byte-offset cursor.
///
/// The cursor is always on a char boundary so `insert_char`, `backspace`,
/// `delete`, and the cursor-movement methods never panic on multibyte input.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    buffer: String,
    cursor: usize,
}

impl TextInput {
    pub fn value(&self) -> &str {
        &self.buffer
    }

    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }

    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.buffer[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.buffer.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buffer.len());
        self.buffer.drain(self.cursor..next);
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.buffer[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        self.cursor = self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buffer.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
    }
}

/// Render a TextInput into `area` on `frame`. The cursor is rendered as a
/// reverse-video cell at the cursor byte offset (approximating column for
/// ASCII input; multi-column chars like emoji will visually drift by one).
pub fn render(frame: &mut Frame, area: Rect, input: &TextInput) {
    let cursor = Style::default().add_modifier(Modifier::REVERSED);
    let value = input.value();
    let cursor_byte = input.cursor_byte();

    let (before, at_and_after) = value.split_at(cursor_byte);
    let mut chars = at_and_after.chars();
    let under_cursor = chars.next();
    let after: String = chars.collect();

    let mut spans = Vec::new();
    if !before.is_empty() {
        spans.push(Span::raw(before.to_string()));
    }
    match under_cursor {
        Some(c) => spans.push(Span::styled(c.to_string(), cursor)),
        None => spans.push(Span::styled(" ", cursor)),
    }
    if !after.is_empty() {
        spans.push(Span::raw(after));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_ascii_characters() {
        let mut input = TextInput::default();
        input.insert_char('h');
        input.insert_char('i');
        assert_eq!(input.value(), "hi");
        assert_eq!(input.cursor_byte(), 2);
    }

    #[test]
    fn insert_at_cursor_position() {
        let mut input = TextInput::default();
        for c in "hllo".chars() {
            input.insert_char(c);
        }
        input.home();
        input.move_right();
        input.insert_char('e');
        assert_eq!(input.value(), "hello");
    }

    #[test]
    fn backspace_across_multibyte_boundary() {
        let mut input = TextInput::default();
        input.insert_char('a');
        input.insert_char('😀'); // 4-byte UTF-8
        input.insert_char('b');
        assert_eq!(input.value(), "a😀b");
        input.backspace();
        assert_eq!(input.value(), "a😀");
        input.backspace();
        assert_eq!(input.value(), "a");
        input.backspace();
        assert_eq!(input.value(), "");
        input.backspace();
        assert_eq!(input.value(), "");
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut input = TextInput::default();
        input.insert_char('a');
        input.delete();
        assert_eq!(input.value(), "a");
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut input = TextInput::default();
        for c in "abc".chars() {
            input.insert_char(c);
        }
        input.home();
        input.delete();
        assert_eq!(input.value(), "bc");
        assert_eq!(input.cursor_byte(), 0);
    }

    #[test]
    fn move_left_right_stay_on_char_boundaries() {
        let mut input = TextInput::default();
        input.insert_char('a');
        input.insert_char('🌲'); // 4-byte UTF-8
        input.insert_char('b');
        // Cursor is at end (byte 6). Move left twice, right once.
        input.move_left();
        assert_eq!(input.cursor_byte(), 5); // after emoji
        input.move_left();
        assert_eq!(input.cursor_byte(), 1); // after 'a'
        input.move_right();
        assert_eq!(input.cursor_byte(), 5);
    }

    #[test]
    fn home_and_end_jump_to_boundaries() {
        let mut input = TextInput::default();
        for c in "hello".chars() {
            input.insert_char(c);
        }
        input.home();
        assert_eq!(input.cursor_byte(), 0);
        input.end();
        assert_eq!(input.cursor_byte(), 5);
    }

    #[test]
    fn cursor_clamped_at_bounds() {
        let mut input = TextInput::default();
        input.move_left();
        assert_eq!(input.cursor_byte(), 0);
        input.insert_char('a');
        input.move_right();
        assert_eq!(input.cursor_byte(), 1);
    }
}

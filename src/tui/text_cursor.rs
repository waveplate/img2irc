use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A byte cursor over a UTF-8 input. An unset cursor follows the end of a new value.
#[derive(Clone, Debug, Default)]
pub(super) struct TextCursor {
    byte: Option<usize>,
    column: Option<usize>,
}

impl TextCursor {
    pub(super) fn reset(&mut self) { *self = Self::default(); }

    pub(super) fn position(&self, text: &str) -> usize {
        let mut byte = self.byte.unwrap_or(text.len()).min(text.len());
        while !text.is_char_boundary(byte) { byte -= 1; }
        byte
    }

    pub(super) fn marked(&self, text: &str) -> String {
        let byte = self.position(text);
        format!("{}▏{}", &text[..byte], &text[byte..])
    }

    pub(super) fn insert(&mut self, text: &mut String, value: &str) {
        let byte = self.position(text);
        text.insert_str(byte, value);
        self.byte = Some(byte + value.len());
        self.column = None;
    }

    /// Some(true) means the text changed; Some(false) means cursor movement.
    pub(super) fn key(&mut self, text: &mut String, key: KeyEvent, multiline: bool) -> Option<bool> {
        let byte = self.position(text);
        let previous = || text[..byte].char_indices().next_back().map_or(0, |(i, _)| i);
        let next = || byte + text[byte..].chars().next().map_or(0, char::len_utf8);
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let mut changed = false;
        let mut vertical = false;
        let position = match key.code {
            KeyCode::Left => previous(),
            KeyCode::Right => next(),
            KeyCode::Home => if control { 0 } else { text[..byte].rfind('\n').map_or(0, |i| i + 1) },
            KeyCode::End => if control { text.len() } else { text[byte..].find('\n').map_or(text.len(), |i| byte + i) },
            KeyCode::Up | KeyCode::Down if multiline => {
                let start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
                let column = *self.column.get_or_insert_with(|| text[start..byte].chars().count());
                vertical = true;
                let target = if key.code == KeyCode::Up {
                    if start == 0 { return Some(false); }
                    let end = start - 1;
                    let begin = text[..end].rfind('\n').map_or(0, |i| i + 1);
                    begin..end
                } else {
                    let Some(end) = text[byte..].find('\n').map(|i| byte + i) else { return Some(false); };
                    let begin = end + 1;
                    let end = text[begin..].find('\n').map_or(text.len(), |i| begin + i);
                    begin..end
                };
                target.start + text[target.clone()].char_indices().nth(column).map_or(target.len(), |(i, _)| i)
            }
            KeyCode::Backspace => {
                let start = previous();
                changed = start != byte;
                text.replace_range(start..byte, "");
                start
            }
            KeyCode::Delete => {
                let end = next();
                changed = end != byte;
                text.replace_range(byte..end, "");
                byte
            }
            KeyCode::Enter if multiline => { self.insert(text, "\n"); return Some(true); }
            KeyCode::Char('u') if control => {
                changed = !text.is_empty();
                text.clear();
                0
            }
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.insert(text, &c.to_string());
                return Some(true);
            }
            _ => return None,
        };
        self.byte = Some(position);
        if !vertical { self.column = None; }
        Some(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(cursor: &mut TextCursor, text: &mut String, code: KeyCode) {
        assert!(cursor.key(text, KeyEvent::new(code, KeyModifiers::NONE), true).is_some());
    }

    #[test]
    fn insert_delete_and_backspace_edit_at_a_unicode_cursor() {
        let mut text = "aé猫z".to_string();
        let mut cursor = TextCursor::default();
        press(&mut cursor, &mut text, KeyCode::Left);
        press(&mut cursor, &mut text, KeyCode::Left);
        press(&mut cursor, &mut text, KeyCode::Char('X'));
        assert_eq!(text, "aéX猫z");
        assert_eq!(cursor.marked(&text), "aéX▏猫z");
        press(&mut cursor, &mut text, KeyCode::Delete);
        press(&mut cursor, &mut text, KeyCode::Backspace);
        assert_eq!(text, "aéz");
        cursor.insert(&mut text, "two words");
        assert_eq!(text, "aétwo wordsz");
    }

    #[test]
    fn vertical_movement_retains_column_across_short_and_blank_lines() {
        let mut text = "abcde\nx\n\n12345".to_string();
        let mut cursor = TextCursor::default();
        press(&mut cursor, &mut text, KeyCode::Left);
        for _ in 0..3 { press(&mut cursor, &mut text, KeyCode::Up); }
        assert_eq!(cursor.marked(&text), "abcd▏e\nx\n\n12345");
        for _ in 0..3 { press(&mut cursor, &mut text, KeyCode::Down); }
        assert_eq!(cursor.marked(&text), "abcde\nx\n\n1234▏5");
        press(&mut cursor, &mut text, KeyCode::Home);
        press(&mut cursor, &mut text, KeyCode::Enter);
        assert_eq!(text, "abcde\nx\n\n\n12345");
        press(&mut cursor, &mut text, KeyCode::End);
        assert_eq!(cursor.position(&text), text.len());
    }
}

//! Shared text input key handling for TUI text fields.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handle a key event for a plain text input field.
///
/// Returns `true` if the key was consumed, `false` if the caller should handle it.
///
/// Handled keys:
/// - `Backspace` → pop last char
/// - `Ctrl+U` → clear string
/// - `Ctrl+W` → delete word backward
/// - `Char(c)` with no CONTROL modifier → push char
pub fn handle_text_input(s: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => {
            s.pop();
            true
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.clear();
            true
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            delete_word_backward(s);
            true
        }
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if let Some(text) = crate::ui::clipboard::read_text() {
                s.push_str(&text);
            }
            true
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            s.push(c);
            true
        }
        _ => false,
    }
}

/// Convert a crossterm `KeyEvent` to a `tui_textarea::Input`.
pub fn to_textarea_input(key: crossterm::event::KeyEvent) -> tui_textarea::Input {
    use crossterm::event::{KeyCode, KeyModifiers};
    use tui_textarea::{Input as TaInput, Key as TaKey};
    let tkey = match key.code {
        KeyCode::Char(c) => TaKey::Char(c),
        KeyCode::Backspace => TaKey::Backspace,
        KeyCode::Enter => TaKey::Enter,
        KeyCode::Left => TaKey::Left,
        KeyCode::Right => TaKey::Right,
        KeyCode::Up => TaKey::Up,
        KeyCode::Down => TaKey::Down,
        KeyCode::Tab => TaKey::Tab,
        KeyCode::Delete => TaKey::Delete,
        KeyCode::Home => TaKey::Home,
        KeyCode::End => TaKey::End,
        KeyCode::PageUp => TaKey::PageUp,
        KeyCode::PageDown => TaKey::PageDown,
        KeyCode::Esc => TaKey::Esc,
        KeyCode::F(n) => TaKey::F(n),
        _ => TaKey::Null,
    };
    TaInput {
        key: tkey,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
}

/// Delete the word immediately before the cursor.
///
/// Trims trailing whitespace, then removes characters until start-of-string
/// or a whitespace character is reached.
pub fn delete_word_backward(s: &mut String) {
    // Trim trailing whitespace
    let trimmed_len = s.trim_end().len();
    s.truncate(trimmed_len);
    // Remove chars until whitespace or start
    while !s.is_empty() {
        if s.ends_with(char::is_whitespace) {
            break;
        }
        s.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delete_word_backward_simple() {
        let mut s = "hello world".to_string();
        delete_word_backward(&mut s);
        assert_eq!(s, "hello ");
    }

    #[test]
    fn test_delete_word_backward_trailing_spaces() {
        let mut s = "hello world   ".to_string();
        delete_word_backward(&mut s);
        assert_eq!(s, "hello ");
    }

    #[test]
    fn test_delete_word_backward_single_word() {
        let mut s = "hello".to_string();
        delete_word_backward(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn test_delete_word_backward_empty() {
        let mut s = String::new();
        delete_word_backward(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn test_handle_text_input_backspace() {
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "hell");
    }

    #[test]
    fn test_handle_text_input_ctrl_u() {
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "");
    }

    #[test]
    fn test_handle_text_input_ctrl_w() {
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        let mut s = "hello world".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "hello ");
    }

    #[test]
    fn test_handle_text_input_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "hellox");
    }

    #[test]
    fn test_handle_text_input_unhandled() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let mut s = "hello".to_string();
        assert!(!handle_text_input(&mut s, &key));
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_handle_text_input_ctrl_v_consumed() {
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
    }

    #[test]
    fn test_handle_text_input_ctrl_shift_v_consumed() {
        let key = KeyEvent::new(
            KeyCode::Char('V'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
    }

    #[test]
    fn test_handle_text_input_ctrl_uppercase_v_consumed() {
        let key = KeyEvent::new(KeyCode::Char('V'), KeyModifiers::CONTROL);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
    }

    #[test]
    fn test_handle_text_input_v_no_ctrl_inserts_char() {
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "hellov");
    }

    #[test]
    fn test_handle_text_input_shift_v_inserts_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT);
        let mut s = "hello".to_string();
        assert!(handle_text_input(&mut s, &key));
        assert_eq!(s, "helloV");
    }
}

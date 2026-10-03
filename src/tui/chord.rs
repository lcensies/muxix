//! Terminal key chords: a key code plus its (normalized) modifiers.
//!
//! A chord is the unit the binding registry matches against. Construction is
//! `const` so static binding tables allocate nothing, and `from_event`
//! normalizes incoming crossterm events so a binding written as `key('F')`
//! matches whether the terminal reports `Char('F')` with or without `SHIFT`.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single key chord (code + modifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyChord {
    pub const fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }

    /// A bare character key with no modifiers (case carried by the char).
    pub const fn key(c: char) -> Self {
        Self::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// A `Ctrl`+character chord.
    pub const fn ctrl(c: char) -> Self {
        Self::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// A non-character key (e.g. `KeyCode::Tab`, `KeyCode::Esc`) with no modifiers.
    pub const fn code(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }

    /// Normalize a crossterm event into a chord.
    ///
    /// For character keys the `SHIFT` modifier is dropped because the case is
    /// already encoded in the char (`Char('F')`), matching how bindings are
    /// authored. `CONTROL` and `ALT` are preserved. Non-character keys keep
    /// their modifiers verbatim (`Shift`+`Tab` is delivered as `BackTab`).
    pub fn from_event(ev: KeyEvent) -> Self {
        let mut mods = ev.modifiers;
        if matches!(ev.code, KeyCode::Char(_)) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Self::new(ev.code, mods)
    }

    /// Parse a chord from a string like `"ctrl+n"`, `"Tab"`, `"?"`, `"C-x"`.
    ///
    /// Modifier prefixes (case-insensitive): `ctrl`/`c`, `alt`/`a`/`meta`/`m`,
    /// `shift`/`s`, separated by `+` or `-`. The final segment is the key.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err("empty key chord".to_string());
        }
        // Split on the LAST separator so a literal `+`/`-` key works
        // (e.g. `"ctrl++"`, `"-"`). Walk segments accumulating modifiers.
        let mut mods = KeyModifiers::NONE;
        let mut rest = spec;
        loop {
            // Find the first separator; if the segment before it is a known
            // modifier, consume it. Otherwise the remainder is the key.
            let sep = rest.find(['+', '-']);
            let Some(idx) = sep else { break };
            // A leading separator (e.g. the key is literally `-`) is the key.
            if idx == 0 {
                break;
            }
            let (head, tail) = rest.split_at(idx);
            let modifier = match head.to_ascii_lowercase().as_str() {
                "ctrl" | "c" | "control" => Some(KeyModifiers::CONTROL),
                "alt" | "a" | "meta" | "m" | "opt" | "option" => Some(KeyModifiers::ALT),
                "shift" | "s" => Some(KeyModifiers::SHIFT),
                _ => None,
            };
            match modifier {
                Some(m) => {
                    mods |= m;
                    rest = &tail[1..];
                }
                None => break,
            }
        }

        let key = rest;
        let code = match key.to_ascii_lowercase().as_str() {
            "tab" => KeyCode::Tab,
            "backtab" | "btab" => KeyCode::BackTab,
            "enter" | "return" | "cr" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "space" | "spc" => KeyCode::Char(' '),
            "bksp" | "backspace" | "bs" => KeyCode::Backspace,
            "del" | "delete" => KeyCode::Delete,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            _ => {
                let mut chars = key.chars();
                let c = chars
                    .next()
                    .ok_or_else(|| format!("missing key in chord: {spec:?}"))?;
                if chars.next().is_some() {
                    return Err(format!("unknown key name in chord: {spec:?}"));
                }
                KeyCode::Char(c)
            }
        };

        // Normalize: a shift on a character key is folded into the char only
        // when it has an obvious uppercase form; otherwise keep it (rare).
        if let KeyCode::Char(c) = code
            && mods.contains(KeyModifiers::SHIFT)
            && c.is_ascii_lowercase()
        {
            mods.remove(KeyModifiers::SHIFT);
            return Ok(Self::new(KeyCode::Char(c.to_ascii_uppercase()), mods));
        }
        if matches!(code, KeyCode::Char(_)) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Ok(Self::new(code, mods))
    }

    /// A short human-readable hint (e.g. `"Ctrl+n"`, `"Tab"`, `"?"`).
    pub fn hint(&self) -> String {
        let mut s = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            s.push_str("Ctrl+");
        }
        if self.mods.contains(KeyModifiers::ALT) {
            s.push_str("Alt+");
        }
        match self.code {
            KeyCode::Char(' ') => s.push_str("Space"),
            KeyCode::Char(c) => s.push(c),
            KeyCode::Tab => s.push_str("Tab"),
            KeyCode::BackTab => s.push_str("S-Tab"),
            KeyCode::Enter => s.push_str("Enter"),
            KeyCode::Esc => s.push_str("Esc"),
            KeyCode::Backspace => s.push_str("Bksp"),
            KeyCode::Delete => s.push_str("Del"),
            KeyCode::Up => s.push('↑'),
            KeyCode::Down => s.push('↓'),
            KeyCode::Left => s.push('←'),
            KeyCode::Right => s.push('→'),
            KeyCode::PageUp => s.push_str("PgUp"),
            KeyCode::PageDown => s.push_str("PgDn"),
            KeyCode::Home => s.push_str("Home"),
            KeyCode::End => s.push_str("End"),
            other => s.push_str(&format!("{other:?}")),
        }
        s
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_char() {
        assert_eq!(KeyChord::parse("q").unwrap(), KeyChord::key('q'));
        assert_eq!(KeyChord::parse("?").unwrap(), KeyChord::key('?'));
    }

    #[test]
    fn parse_ctrl() {
        assert_eq!(KeyChord::parse("ctrl+n").unwrap(), KeyChord::ctrl('n'));
        assert_eq!(KeyChord::parse("C-n").unwrap(), KeyChord::ctrl('n'));
    }

    #[test]
    fn parse_named() {
        assert_eq!(
            KeyChord::parse("Tab").unwrap(),
            KeyChord::code(KeyCode::Tab)
        );
        assert_eq!(
            KeyChord::parse("Esc").unwrap(),
            KeyChord::code(KeyCode::Esc)
        );
        assert_eq!(
            KeyChord::parse("Enter").unwrap(),
            KeyChord::code(KeyCode::Enter)
        );
    }

    #[test]
    fn parse_literal_dash() {
        assert_eq!(KeyChord::parse("-").unwrap(), KeyChord::key('-'));
    }

    #[test]
    fn parse_shifted_letter_folds_to_uppercase() {
        assert_eq!(KeyChord::parse("shift+f").unwrap(), KeyChord::key('F'));
    }

    #[test]
    fn from_event_strips_shift_on_char() {
        let ev = KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT);
        assert_eq!(KeyChord::from_event(ev), KeyChord::key('F'));
    }

    #[test]
    fn from_event_keeps_ctrl() {
        let ev = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(KeyChord::from_event(ev), KeyChord::ctrl('n'));
    }

    #[test]
    fn hint_roundtrips_common() {
        assert_eq!(KeyChord::ctrl('n').hint(), "Ctrl+n");
        assert_eq!(KeyChord::code(KeyCode::Tab).hint(), "Tab");
        assert_eq!(KeyChord::key('?').hint(), "?");
    }
}

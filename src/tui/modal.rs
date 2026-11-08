//! One centered modal widget with three render modes.
//!
//! Generic over the action type, like [`super::palette`], so the sidebar and
//! (later) the dashboard can share it. Sized from the area it is handed rather
//! than from fixed clamps, because the sidebar pane is only 25-50 columns wide.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::ui::theme::ThemePalette;

use super::palette::PaletteState;

/// One field of a [`Modal::Form`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalField {
    Text {
        label: &'static str,
        value: String,
    },
    /// A value cycled with `←`/`→`, drawn as `◂ value ▸`.
    Choice {
        label: &'static str,
        options: Vec<String>,
        idx: usize,
    },
}

impl ModalField {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Text { label, .. } | Self::Choice { label, .. } => label,
        }
    }

    /// The field's current value, as the form reports it on submit.
    pub fn value(&self) -> String {
        match self {
            Self::Text { value, .. } => value.clone(),
            Self::Choice { options, idx, .. } => options.get(*idx).cloned().unwrap_or_default(),
        }
    }

    fn cycle(&mut self, forward: bool) {
        if let Self::Choice { options, idx, .. } = self
            && !options.is_empty()
        {
            let n = options.len();
            *idx = if forward {
                (*idx + 1) % n
            } else {
                (*idx + n - 1) % n
            };
        }
    }
}

/// A modal in one of three forms.
pub enum Modal<A: Clone + 'static> {
    /// A yes/no question. `accept_on_enter` decides what a bare Enter means.
    Confirm {
        title: String,
        lines: Vec<String>,
        accept_on_enter: bool,
    },
    Form {
        title: String,
        fields: Vec<ModalField>,
        focused: usize,
    },
    ActionList {
        title: &'static str,
        state: PaletteState<A>,
    },
}

/// What a key did to the modal.
pub enum ModalOutcome<A> {
    /// Key consumed; the modal stays open.
    Consumed,
    /// Close without doing anything.
    Cancel,
    /// A [`Modal::Confirm`] was accepted.
    Confirmed,
    /// A [`Modal::Form`] was submitted, with one value per field in order.
    Submit(Vec<String>),
    /// A [`Modal::ActionList`] entry was picked.
    Run(A),
}

impl<A: Clone + 'static> Modal<A> {
    pub fn confirm(title: impl Into<String>, lines: Vec<String>, accept_on_enter: bool) -> Self {
        Self::Confirm {
            title: title.into(),
            lines,
            accept_on_enter,
        }
    }

    pub fn form(title: impl Into<String>, fields: Vec<ModalField>) -> Self {
        Self::Form {
            title: title.into(),
            fields,
            focused: 0,
        }
    }

    /// Feed a key press to the modal.
    pub fn handle_key(&mut self, key: &KeyEvent) -> ModalOutcome<A> {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match self {
            Self::Confirm {
                accept_on_enter, ..
            } => match key.code {
                KeyCode::Enter if *accept_on_enter => ModalOutcome::Confirmed,
                KeyCode::Char('y') | KeyCode::Char('Y') => ModalOutcome::Confirmed,
                _ => ModalOutcome::Cancel,
            },
            Self::Form {
                fields, focused, ..
            } => {
                let count = fields.len().max(1);
                match key.code {
                    KeyCode::Esc => ModalOutcome::Cancel,
                    KeyCode::Enter => {
                        ModalOutcome::Submit(fields.iter().map(ModalField::value).collect())
                    }
                    KeyCode::Tab if !shift => {
                        *focused = (*focused + 1) % count;
                        ModalOutcome::Consumed
                    }
                    KeyCode::Tab | KeyCode::BackTab => {
                        *focused = (*focused + count - 1) % count;
                        ModalOutcome::Consumed
                    }
                    KeyCode::Down => {
                        *focused = (*focused + 1) % count;
                        ModalOutcome::Consumed
                    }
                    KeyCode::Up => {
                        *focused = (*focused + count - 1) % count;
                        ModalOutcome::Consumed
                    }
                    KeyCode::Right | KeyCode::Left => {
                        if let Some(f) = fields.get_mut(*focused) {
                            f.cycle(key.code == KeyCode::Right);
                        }
                        ModalOutcome::Consumed
                    }
                    _ => {
                        if let Some(ModalField::Text { value, .. }) = fields.get_mut(*focused) {
                            crate::ui::text_input::handle_text_input(value, key);
                        }
                        ModalOutcome::Consumed
                    }
                }
            }
            Self::ActionList { state, .. } => match key.code {
                KeyCode::Esc => ModalOutcome::Cancel,
                KeyCode::Enter => match state.selected_action() {
                    Some(a) => ModalOutcome::Run(a.clone()),
                    None => ModalOutcome::Consumed,
                },
                KeyCode::Down => {
                    state.move_down();
                    ModalOutcome::Consumed
                }
                KeyCode::Up => {
                    state.move_up();
                    ModalOutcome::Consumed
                }
                KeyCode::Backspace => {
                    state.backspace();
                    ModalOutcome::Consumed
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.append_char(c);
                    ModalOutcome::Consumed
                }
                _ => ModalOutcome::Consumed,
            },
        }
    }
}

/// Draw `modal` centered in `area`, over whatever is already there.
pub fn render<A: Clone + 'static>(
    f: &mut Frame,
    area: Rect,
    modal: &Modal<A>,
    theme: &ThemePalette,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let bold = |s: &str| {
        Span::styled(
            s.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )
    };
    let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(theme.dimmed));

    let (title, mut lines, footer) = match modal {
        Modal::ActionList { title, state } => {
            super::palette::render_in(f, area, state, theme, title);
            return;
        }
        Modal::Confirm { title, lines, .. } => (
            title.clone(),
            lines
                .iter()
                .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.text))))
                .collect::<Vec<_>>(),
            Line::from(vec![
                bold("Enter/y"),
                dim(" yes  "),
                bold("any"),
                dim(" no"),
            ]),
        ),
        Modal::Form {
            title,
            fields,
            focused,
        } => {
            let mut lines = Vec::new();
            for (i, field) in fields.iter().enumerate() {
                let is_focused = i == *focused;
                let marker = if is_focused { "▸ " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default().fg(if is_focused {
                            theme.accent
                        } else {
                            theme.dimmed
                        }),
                    ),
                    Span::styled(field.label(), Style::default().fg(theme.dimmed)),
                ]));
                let value_style =
                    Style::default().fg(if is_focused { theme.accent } else { theme.text });
                let shown = match field {
                    ModalField::Text { value, .. } => {
                        let cursor = if is_focused { "_" } else { "" };
                        format!("  {value}{cursor}")
                    }
                    ModalField::Choice { .. } => format!("  ◂ {} ▸", field.value()),
                };
                lines.push(Line::from(Span::styled(shown, value_style)));
            }
            (
                title.clone(),
                lines,
                Line::from(vec![
                    bold("Tab"),
                    dim(" field  "),
                    bold("Enter"),
                    dim(" ok  "),
                    bold("Esc"),
                    dim(" cancel"),
                ]),
            )
        }
    };

    lines.push(Line::from(""));
    lines.push(footer);

    // Leave a one-column margin when the pane can spare it, and never exceed
    // the area: the sidebar pane can be as narrow as 25 columns.
    let width = area.width.clamp(4, 60);
    let content_height = lines.len() as u16 + 2;
    let height = content_height.min(area.height);

    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };

    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.help_border))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.header)
                .add_modifier(Modifier::BOLD),
        )));

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ThemeMode};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn theme() -> ThemePalette {
        ThemePalette::from_config(&Config::default().theme, ThemeMode::Dark)
    }

    fn form() -> Modal<u8> {
        Modal::form(
            "New agent",
            vec![
                ModalField::Text {
                    label: "Branch",
                    value: String::new(),
                },
                ModalField::Choice {
                    label: "Placement",
                    options: vec!["window".into(), "session".into(), "pane".into()],
                    idx: 0,
                },
            ],
        )
    }

    #[test]
    fn a_form_renders_inside_a_narrow_pane() {
        let backend = TestBackend::new(25, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let m = form();
        let t = theme();
        // Drawing outside the buffer panics in ratatui, so a clean draw is the
        // in-bounds assertion.
        terminal
            .draw(|f| render(f, Rect::new(0, 0, 25, 10), &m, &t))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("New agent"), "title missing: {text}");
        assert!(text.contains("Branch"), "label missing: {text}");
    }

    #[test]
    fn a_confirm_renders_inside_a_narrow_pane() {
        let backend = TestBackend::new(25, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let m: Modal<u8> = Modal::confirm(
            "Delete",
            vec!["Delete agent feature-x and its worktree?".to_string()],
            true,
        );
        let t = theme();
        terminal
            .draw(|f| render(f, Rect::new(0, 0, 25, 10), &m, &t))
            .unwrap();
    }

    #[test]
    fn an_action_list_renders_inside_a_narrow_pane() {
        use crate::tui::palette::PaletteCommand;
        let backend = TestBackend::new(25, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let m: Modal<u8> = Modal::ActionList {
            title: "Actions",
            state: PaletteState::new(vec![PaletteCommand {
                label: "Delete agent",
                key_hint: "d",
                action: 1u8,
            }]),
        };
        let t = theme();
        terminal
            .draw(|f| render(f, Rect::new(0, 0, 25, 10), &m, &t))
            .unwrap();
    }

    #[test]
    fn focus_movement_wraps_both_ways() {
        let mut m = form();
        let focused = |m: &Modal<u8>| match m {
            Modal::Form { focused, .. } => *focused,
            _ => unreachable!(),
        };
        assert_eq!(focused(&m), 0);
        m.handle_key(&key(KeyCode::Tab));
        assert_eq!(focused(&m), 1);
        m.handle_key(&key(KeyCode::Tab));
        assert_eq!(focused(&m), 0, "Tab wraps round");
        m.handle_key(&key(KeyCode::BackTab));
        assert_eq!(focused(&m), 1, "Shift-Tab wraps back");
    }

    #[test]
    fn choice_cycling_wraps() {
        let mut m = form();
        m.handle_key(&key(KeyCode::Tab));
        let value = |m: &Modal<u8>| match m {
            Modal::Form { fields, .. } => fields[1].value(),
            _ => unreachable!(),
        };
        assert_eq!(value(&m), "window");
        m.handle_key(&key(KeyCode::Right));
        assert_eq!(value(&m), "session");
        m.handle_key(&key(KeyCode::Right));
        assert_eq!(value(&m), "pane");
        m.handle_key(&key(KeyCode::Right));
        assert_eq!(value(&m), "window", "forward wraps");
        m.handle_key(&key(KeyCode::Left));
        assert_eq!(value(&m), "pane", "backward wraps");
    }

    #[test]
    fn esc_cancels_and_enter_submits_the_field_values() {
        let mut m = form();
        assert!(matches!(
            m.handle_key(&key(KeyCode::Esc)),
            ModalOutcome::Cancel
        ));
        for c in "fix-1".chars() {
            m.handle_key(&key(KeyCode::Char(c)));
        }
        match m.handle_key(&key(KeyCode::Enter)) {
            ModalOutcome::Submit(values) => {
                assert_eq!(values, vec!["fix-1".to_string(), "window".to_string()]);
            }
            _ => panic!("Enter must submit"),
        }
    }

    #[test]
    fn a_confirm_accepts_enter_and_rejects_everything_else() {
        let mut m: Modal<u8> = Modal::confirm("t", vec!["q".into()], true);
        assert!(matches!(
            m.handle_key(&key(KeyCode::Enter)),
            ModalOutcome::Confirmed
        ));
        for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('j')] {
            assert!(matches!(m.handle_key(&key(code)), ModalOutcome::Cancel));
        }
        // With Enter not accepting, only an explicit `y` confirms.
        let mut m: Modal<u8> = Modal::confirm("t", vec!["q".into()], false);
        assert!(matches!(
            m.handle_key(&key(KeyCode::Enter)),
            ModalOutcome::Cancel
        ));
        assert!(matches!(
            m.handle_key(&key(KeyCode::Char('y'))),
            ModalOutcome::Confirmed
        ));
    }

    #[test]
    fn an_action_list_filters_and_runs() {
        use crate::tui::palette::PaletteCommand;
        let mut m: Modal<u8> = Modal::ActionList {
            title: "Actions",
            state: PaletteState::new(vec![
                PaletteCommand {
                    label: "delete agent",
                    key_hint: "d",
                    action: 1u8,
                },
                PaletteCommand {
                    label: "toggle layout",
                    key_hint: "v",
                    action: 2u8,
                },
            ]),
        };
        for c in "tog".chars() {
            m.handle_key(&key(KeyCode::Char(c)));
        }
        match m.handle_key(&key(KeyCode::Enter)) {
            ModalOutcome::Run(a) => assert_eq!(a, 2),
            _ => panic!("Enter must run the filtered entry"),
        }
    }
}

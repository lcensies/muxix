//! Reusable fuzzy command palette (VSCode-style `:` menu).
//!
//! Generic over the action type so the dashboard and sidebar share one palette
//! widget, state machine, and fuzzy matcher. Commands are sourced from the
//! binding registry; see [`super::binding`].

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::ui::theme::ThemePalette;

use super::binding::Binding;

/// A single command shown in the palette.
pub struct PaletteCommand<A> {
    pub label: &'static str,
    pub key_hint: &'static str,
    pub action: A,
}

/// Command palette modal state.
pub struct PaletteState<A> {
    pub commands: Vec<PaletteCommand<A>>,
    pub filter: String,
    pub cursor: usize,
}

impl<A: Clone> PaletteState<A> {
    pub fn new(commands: Vec<PaletteCommand<A>>) -> Self {
        Self {
            commands,
            filter: String::new(),
            cursor: 0,
        }
    }

    /// Build palette state from the binding registry for a context.
    pub fn from_bindings(bindings: &[Binding<A>], ctx: &str) -> Self {
        let commands = super::binding::palette_entries(bindings, ctx)
            .into_iter()
            .filter_map(|b| {
                b.action.clone().map(|action| PaletteCommand {
                    label: b.label,
                    key_hint: b.hint,
                    action,
                })
            })
            .collect();
        Self::new(commands)
    }
}

impl<A> PaletteState<A> {
    /// Indices into `commands` matching the current filter, best first.
    pub fn filtered(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.commands.len()).collect();
        }
        let query = self.filter.to_lowercase();
        let mut scored: Vec<(usize, i32)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, cmd)| {
                fuzzy_score(&query, &cmd.label.to_lowercase()).map(|score| (i, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(i, _)| i).collect()
    }

    /// The action currently under the cursor, if any.
    pub fn selected_action(&self) -> Option<&A> {
        let filtered = self.filtered();
        filtered
            .get(self.cursor)
            .map(|&idx| &self.commands[idx].action)
    }

    pub fn move_down(&mut self) {
        let count = self.filtered().len();
        if count > 0 {
            self.cursor = (self.cursor + 1).min(count - 1);
        }
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn append_char(&mut self, c: char) {
        self.filter.push(c);
        self.cursor = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.cursor = 0;
    }
}

/// Score a fuzzy match: higher is better, `None` means no match.
/// Exact prefix > word-boundary match > substring > subsequence.
pub fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    if target.starts_with(query) {
        return Some(1000 + query.len() as i32);
    }
    for word in target.split_whitespace() {
        if word.starts_with(query) {
            return Some(800 + query.len() as i32);
        }
    }
    if target.contains(query) {
        return Some(500 + query.len() as i32);
    }
    let mut target_chars = target.chars().peekable();
    let mut score = 0i32;
    let mut matched = 0;
    let mut prev_matched = false;
    for qc in query.chars() {
        let mut found = false;
        for tc in target_chars.by_ref() {
            if tc == qc {
                matched += 1;
                if prev_matched {
                    score += 5;
                }
                prev_matched = true;
                found = true;
                break;
            }
            prev_matched = false;
        }
        if !found {
            return None;
        }
    }
    if matched == query.len() {
        Some(score + matched as i32)
    } else {
        None
    }
}

/// Render the command palette modal centered on the frame.
pub fn render<A>(f: &mut Frame, state: &PaletteState<A>, theme: &ThemePalette) {
    let bold = |s: &str| {
        Span::styled(
            s.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )
    };
    let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(theme.dimmed));

    let filtered = state.filtered();

    let area = f.area();
    let width = (area.width * 3 / 5).clamp(40, 70);
    let height = (area.height * 2 / 5).clamp(10, 25);
    // overhead: filter + blank + blank_after_items + footer + borders(2)
    let overhead: u16 = 6;
    let max_visible: usize = height.saturating_sub(overhead) as usize;

    let popup_width = width.min(area.width);
    let popup_height = height.min(area.height);
    let inner_width = popup_width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Filter input line
    if state.filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(theme.dimmed)),
            Span::styled("_", Style::default().fg(theme.dimmed)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(theme.dimmed)),
            Span::styled(state.filter.clone(), Style::default().fg(theme.text)),
            Span::styled("_", Style::default().fg(theme.text)),
        ]));
    }

    lines.push(Line::from(""));

    if filtered.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No matching commands.",
            Style::default().fg(theme.dimmed),
        )]));
        for _ in 1..max_visible {
            lines.push(Line::from(""));
        }
    } else {
        let total = filtered.len();
        let start = if total <= max_visible || state.cursor < max_visible / 2 {
            0
        } else if state.cursor + max_visible / 2 >= total {
            total.saturating_sub(max_visible)
        } else {
            state.cursor - max_visible / 2
        };
        let end = (start + max_visible).min(total);

        for (fi, &idx) in filtered.iter().enumerate().take(end).skip(start) {
            let cmd = &state.commands[idx];
            let is_selected = fi == state.cursor;
            let cursor_str = if is_selected { "> " } else { "  " };

            let label_style = if is_selected {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.text)
            };

            let mut spans = vec![
                Span::styled(cursor_str, Style::default().fg(theme.text)),
                Span::styled(cmd.label, label_style),
            ];

            if !cmd.key_hint.is_empty() {
                let label_len = 2 + cmd.label.len() + 1 + cmd.key_hint.len();
                let pad = inner_width.saturating_sub(label_len);
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(
                    cmd.key_hint,
                    Style::default()
                        .fg(theme.dimmed)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            lines.push(Line::from(spans));
        }

        for _ in (end - start)..max_visible {
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw(" "),
        bold("Enter"),
        dim(" run  "),
        bold("Esc"),
        dim(" cancel"),
    ]));

    let popup_area = Rect {
        x: area.width.saturating_sub(popup_width) / 2,
        y: area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.help_border))
        .title(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                "Command Palette",
                Style::default()
                    .fg(theme.header)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]));

    let paragraph = Paragraph::new(Text::from(lines)).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(labels: &[&'static str]) -> PaletteState<usize> {
        PaletteState::new(
            labels
                .iter()
                .enumerate()
                .map(|(i, l)| PaletteCommand {
                    label: l,
                    key_hint: "",
                    action: i,
                })
                .collect(),
        )
    }

    #[test]
    fn empty_filter_lists_all_in_order() {
        let s = state(&["alpha", "beta", "gamma"]);
        assert_eq!(s.filtered(), vec![0, 1, 2]);
    }

    #[test]
    fn prefix_beats_subsequence() {
        let s = state(&["commit changes", "cycle theme"]);
        // "cy" is a prefix of "cycle theme" -> ranks first
        let mut s = s;
        s.filter = "cy".to_string();
        let f = s.filtered();
        assert_eq!(s.commands[f[0]].label, "cycle theme");
    }

    #[test]
    fn selected_action_follows_cursor() {
        let mut s = state(&["a", "b", "c"]);
        s.move_down();
        assert_eq!(s.selected_action(), Some(&1));
        s.move_up();
        assert_eq!(s.selected_action(), Some(&0));
    }

    #[test]
    fn no_match_returns_empty() {
        let mut s = state(&["alpha"]);
        s.filter = "zzz".to_string();
        assert!(s.filtered().is_empty());
        assert_eq!(s.selected_action(), None);
    }

    #[test]
    fn render_draws_title_and_labels() {
        use crate::config::{Config, ThemeMode};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let theme = ThemePalette::from_config(&Config::default().theme, ThemeMode::Dark);
        let st = state(&["Commit changes", "Merge branch"]);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, &st, &theme)).unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Command Palette"), "title missing");
        assert!(text.contains("Commit changes"), "first label missing");
        assert!(text.contains("Merge branch"), "second label missing");
    }
}

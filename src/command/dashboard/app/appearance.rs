//! Theme and appearance logic for the dashboard.

use super::super::ui::theme::ThemePalette;
use super::App;

impl App {
    /// Cycle to the next color scheme, re-render palette, and persist to config
    pub fn cycle_color_scheme(&mut self) {
        self.scheme = self.scheme.next();
        self.palette = ThemePalette::for_scheme(self.scheme, self.theme_mode);
        self.save_theme_scheme();
    }

    fn save_theme_scheme(&self) {
        let Some(ref path) = self.config_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        let result = update_theme_in_config(&contents, self.scheme, self.config.theme.mode);
        let _ = std::fs::write(path, result);
    }
}

/// Update the `theme:` entry in a YAML config string.
/// Preserves explicit mode override when present.
/// Prefers uncommented `theme:` over `# theme:`.
fn update_theme_in_config(
    contents: &str,
    scheme: crate::config::ThemeScheme,
    explicit_mode: Option<crate::config::ThemeMode>,
) -> String {
    use crate::config::{ThemeMode, ThemeScheme};

    let slug = scheme.slug();

    // Build replacement lines
    let new_lines_for_theme = if scheme == ThemeScheme::Default && explicit_mode.is_none() {
        vec!["# theme: default".to_string()]
    } else if let Some(mode) = explicit_mode {
        let mode_str = match mode {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        };
        vec![
            "theme:".to_string(),
            format!("  scheme: {}", slug),
            format!("  mode: {}", mode_str),
        ]
    } else {
        vec![format!("theme: {}", slug)]
    };

    let lines: Vec<&str> = contents.lines().collect();

    // First pass: find the best target line (prefer uncommented over commented)
    let mut uncommented_idx = None;
    let mut commented_idx = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("theme:") && uncommented_idx.is_none() {
            uncommented_idx = Some(i);
        } else if trimmed.starts_with("# theme:") && commented_idx.is_none() {
            commented_idx = Some(i);
        }
    }
    let target_idx = uncommented_idx.or(commented_idx);

    // Second pass: build output
    let mut result_lines: Vec<String> = Vec::new();
    let mut replaced = false;
    let mut iter = lines.iter().enumerate().peekable();

    while let Some((i, line)) = iter.next() {
        if !replaced && Some(i) == target_idx {
            replaced = true;
            result_lines.extend(new_lines_for_theme.clone());

            // Skip structured block sub-keys (including blank lines within)
            let trimmed = line.trim_start();
            let is_block = trimmed
                .strip_prefix("theme:")
                .is_some_and(|rest| rest.trim().is_empty());

            if is_block {
                let block_indent = line.len() - trimmed.len();
                while let Some(&(_, next_line)) = iter.peek() {
                    let next_trimmed = next_line.trim_start();
                    // Continue through blank lines and deeper-indented lines
                    if next_trimmed.is_empty() {
                        iter.next();
                        continue;
                    }
                    if (next_line.len() - next_trimmed.len()) > block_indent {
                        iter.next();
                    } else {
                        break;
                    }
                }
            }
        } else {
            result_lines.push(line.to_string());
        }
    }

    if !replaced && scheme != ThemeScheme::Default {
        result_lines.extend(new_lines_for_theme);
    }

    let mut result = result_lines.join("\n");
    if contents.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    if !contents.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod theme_persistence_tests {
    use super::update_theme_in_config;
    use crate::config::{ThemeMode, ThemeScheme};

    #[test]
    fn simple_theme_line() {
        let input = "agent: claude\ntheme: default\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::Emberforge, None);
        assert_eq!(result, "agent: claude\ntheme: emberforge\nmode: window\n");
    }

    #[test]
    fn no_theme_line_appends() {
        let input = "agent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::Lasergrid, None);
        assert_eq!(result, "agent: claude\ntheme: lasergrid\n");
    }

    #[test]
    fn no_theme_line_default_does_nothing() {
        let input = "agent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, None);
        assert_eq!(result, "agent: claude\n");
    }

    #[test]
    fn default_scheme_comments_out() {
        let input = "theme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, None);
        assert_eq!(result, "# theme: default\n");
    }

    #[test]
    fn structured_block_replaced() {
        let input = "agent: claude\ntheme:\n  scheme: emberforge\n  mode: dark\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::SlateGarden, None);
        assert_eq!(result, "agent: claude\ntheme: slate-garden\nmode: window\n");
    }

    #[test]
    fn structured_block_with_blank_lines() {
        let input = "agent: claude\ntheme:\n  scheme: emberforge\n\n  mode: dark\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::Mossfire, None);
        assert_eq!(result, "agent: claude\ntheme: mossfire\nmode: window\n");
    }

    #[test]
    fn preserves_explicit_mode() {
        let input = "theme: emberforge\n";
        let result =
            update_theme_in_config(input, ThemeScheme::GlacierSignal, Some(ThemeMode::Light));
        assert_eq!(result, "theme:\n  scheme: glacier-signal\n  mode: light\n");
    }

    #[test]
    fn preserves_explicit_dark_mode() {
        let input = "theme: default\n";
        let result = update_theme_in_config(input, ThemeScheme::ObsidianPop, Some(ThemeMode::Dark));
        assert_eq!(result, "theme:\n  scheme: obsidian-pop\n  mode: dark\n");
    }

    #[test]
    fn default_with_explicit_mode() {
        let input = "theme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, Some(ThemeMode::Light));
        assert_eq!(result, "theme:\n  scheme: default\n  mode: light\n");
    }

    #[test]
    fn prefers_uncommented_over_commented() {
        let input = "# theme: default\nagent: claude\ntheme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Lasergrid, None);
        assert_eq!(
            result,
            "# theme: default\nagent: claude\ntheme: lasergrid\n"
        );
    }

    #[test]
    fn falls_back_to_commented_if_no_active() {
        let input = "# theme: default\nagent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::NightSorbet, None);
        assert_eq!(result, "theme: night-sorbet\nagent: claude\n");
    }

    #[test]
    fn empty_file() {
        let result = update_theme_in_config("", ThemeScheme::Emberforge, None);
        assert_eq!(result, "theme: emberforge");
    }

    #[test]
    fn empty_file_default() {
        let result = update_theme_in_config("", ThemeScheme::Default, None);
        assert_eq!(result, "");
    }

    #[test]
    fn preserves_surrounding_content() {
        let input = "# my config\nagent: claude\ntheme: mossfire\nnerdfont: true\n# end\n";
        let result = update_theme_in_config(input, ThemeScheme::TealDrift, None);
        assert_eq!(
            result,
            "# my config\nagent: claude\ntheme: teal-drift\nnerdfont: true\n# end\n"
        );
    }

    #[test]
    fn structured_to_structured_preserves_mode() {
        let input = "theme:\n  scheme: emberforge\n  mode: light\n";
        let result =
            update_theme_in_config(input, ThemeScheme::FestivalCircuit, Some(ThemeMode::Light));
        assert_eq!(
            result,
            "theme:\n  scheme: festival-circuit\n  mode: light\n"
        );
    }

    #[test]
    fn no_trailing_newline_preserved() {
        let input = "theme: default";
        let result = update_theme_in_config(input, ThemeScheme::Emberforge, None);
        assert_eq!(result, "theme: emberforge");
    }
}

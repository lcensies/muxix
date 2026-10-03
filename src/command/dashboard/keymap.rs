//! Keymap definitions for dashboard contexts.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::actions::Action;
use super::bindings::BINDINGS;
use crate::tui::binding;
use crate::tui::chord::KeyChord;

/// Context for key handling - determines which keymap is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    DashboardNormal,
    DashboardInput,
    DashboardFilter,
    WorktreeNormal,
    WorktreeFilter,
    TasksNormal,
    TasksFilter,
    DiffNormal,
    Patch,
    Comment,
}

/// Map a Context to its canonical string name (used for widget spec matching).
pub fn context_name(ctx: Context) -> &'static str {
    match ctx {
        Context::DashboardNormal => "DashboardNormal",
        Context::DashboardInput => "DashboardInput",
        Context::DashboardFilter => "DashboardFilter",
        Context::WorktreeNormal => "WorktreeNormal",
        Context::WorktreeFilter => "WorktreeFilter",
        Context::TasksNormal => "TasksNormal",
        Context::TasksFilter => "TasksFilter",
        Context::DiffNormal => "DiffNormal",
        Context::Patch => "Patch",
        Context::Comment => "Comment",
    }
}

/// Map a key event to an action for the given context.
///
/// Discrete command bindings live in the binding registry (`super::bindings`);
/// this consults it first, then falls back to per-context dynamic handling for
/// text entry, PTY forwarding, and parameterized keys (quick-jump digits).
pub fn action_for_key(ctx: Context, key: KeyEvent) -> Option<Action> {
    let chord = KeyChord::from_event(key);
    if let Some(action) = binding::lookup(BINDINGS, context_name(ctx), chord) {
        return Some(action);
    }
    match ctx {
        Context::DashboardNormal => dashboard_normal_dynamic(key),
        Context::WorktreeNormal => worktree_normal_dynamic(key),
        Context::DashboardInput => dashboard_input_key(key),
        Context::DashboardFilter | Context::WorktreeFilter => dashboard_filter_key(key),
        Context::TasksFilter => tasks_filter_key(key),
        Context::Comment => comment_key(key),
        // TasksNormal, DiffNormal, Patch, ProjectNormal are fully registry-driven.
        _ => None,
    }
}

/// Quick-jump digits for the agents list (registry handles the rest).
fn dashboard_normal_dynamic(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(c @ '1'..='9') => Some(Action::JumpToIndex((c as u8 - b'1') as usize)),
        _ => None,
    }
}

/// Quick-jump digits for the worktree list (registry handles the rest).
fn worktree_normal_dynamic(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(c @ '1'..='9') => {
            Some(Action::WorktreeJumpToIndex((c as u8 - b'1') as usize))
        }
        _ => None,
    }
}

fn dashboard_filter_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::ClearFilter),
        KeyCode::Enter => Some(Action::AcceptFilter),
        KeyCode::Backspace => Some(Action::FilterDeleteChar),
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::FilterDeleteWord)
        }
        KeyCode::Char('?') => Some(Action::ShowHelp),
        KeyCode::Char(c) => Some(Action::FilterAppendChar(c)),
        _ => None,
    }
}

fn tasks_filter_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::TaskClearFilter),
        KeyCode::Enter => Some(Action::TaskAcceptFilter),
        KeyCode::Backspace => Some(Action::TaskFilterDeleteChar),
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::TaskFilterDeleteWord)
        }
        KeyCode::Char(c) => Some(Action::TaskFilterAppendChar(c)),
        _ => None,
    }
}

fn dashboard_input_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::ExitInputMode),
        KeyCode::Enter => Some(Action::SendKey("Enter".to_string())),
        KeyCode::Backspace => Some(Action::SendKey("BSpace".to_string())),
        KeyCode::Tab => Some(Action::SendKey("Tab".to_string())),
        KeyCode::Up => Some(Action::SendKey("Up".to_string())),
        KeyCode::Down => Some(Action::SendKey("Down".to_string())),
        KeyCode::Left => Some(Action::SendKey("Left".to_string())),
        KeyCode::Right => Some(Action::SendKey("Right".to_string())),
        KeyCode::Char(c) => Some(Action::SendKey(c.to_string())),
        _ => None,
    }
}

fn comment_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::CancelComment),
        KeyCode::Enter => Some(Action::SendComment),
        KeyCode::Backspace => Some(Action::DeleteChar),
        KeyCode::Char(c) => Some(Action::AppendChar(c)),
        _ => None,
    }
}





/// Get help rows for a context: (key, description) pairs.
///
/// Registry-driven contexts derive their rows from `super::bindings`; the
/// remaining text-entry/PTY contexts keep curated rows.
pub fn help_rows(ctx: Context) -> Vec<(&'static str, &'static str)> {
    if super::bindings::REGISTRY_CONTEXTS.contains(&context_name(ctx)) {
        return binding::help_rows(BINDINGS, context_name(ctx));
    }
    match ctx {
        Context::DashboardInput => vec![("Esc", "Exit input mode"), ("<keys>", "Send to agent")],
        Context::DashboardFilter | Context::WorktreeFilter => vec![
            ("Enter", "Accept filter"),
            ("Esc", "Clear filter"),
            ("<type>", "Filter text"),
        ],
        Context::TasksFilter => vec![
            ("Enter", "Accept filter"),
            ("Esc", "Clear filter"),
            ("<type>", "Filter text"),
        ],
        Context::Comment => vec![
            ("Esc", "Cancel"),
            ("Enter", "Send comment"),
            ("<type>", "Input text"),
        ],
        // Registry-driven contexts are handled by the early return above.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_each_context_has_help_rows() {
        assert!(!help_rows(Context::DashboardNormal).is_empty());
        assert!(!help_rows(Context::DashboardInput).is_empty());
        assert!(!help_rows(Context::DashboardFilter).is_empty());
        assert!(!help_rows(Context::WorktreeNormal).is_empty());
        assert!(!help_rows(Context::WorktreeFilter).is_empty());
        assert!(!help_rows(Context::TasksNormal).is_empty());
        assert!(!help_rows(Context::TasksFilter).is_empty());
        assert!(!help_rows(Context::DiffNormal).is_empty());
        assert!(!help_rows(Context::Patch).is_empty());
        assert!(!help_rows(Context::Comment).is_empty());
    }

    #[test]
    fn test_no_duplicate_keys_in_context() {
        for ctx in [
            Context::DashboardNormal,
            Context::DashboardInput,
            Context::DashboardFilter,
            Context::WorktreeNormal,
            Context::WorktreeFilter,
            Context::TasksNormal,
            Context::TasksFilter,
            Context::DiffNormal,
            Context::Patch,
            Context::Comment,
        ] {
            let rows = help_rows(ctx);
            let keys: Vec<_> = rows.iter().map(|(k, _)| *k).collect();
            let mut seen = std::collections::HashSet::new();
            for key in &keys {
                assert!(
                    seen.insert(*key),
                    "Duplicate key '{key}' in context {ctx:?}"
                );
            }
        }
    }

    #[test]
    fn test_dashboard_quit_keys() {
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(
            action_for_key(Context::DashboardNormal, q),
            Some(Action::Quit)
        );
        assert_eq!(
            action_for_key(Context::DashboardNormal, esc),
            Some(Action::Quit)
        );
        assert_eq!(
            action_for_key(Context::DashboardNormal, ctrl_c),
            Some(Action::Quit)
        );
    }

    #[test]
    fn test_diff_close_keys() {
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(
            action_for_key(Context::DiffNormal, q),
            Some(Action::CloseDiff)
        );
        assert_eq!(
            action_for_key(Context::DiffNormal, esc),
            Some(Action::CloseDiff)
        );
    }

    #[test]
    fn test_patch_stage_key() {
        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(
            action_for_key(Context::Patch, y),
            Some(Action::StageAndNext)
        );
    }

    #[test]
    fn test_scope_filter_key() {
        let shift_f = KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE);
        assert_eq!(
            action_for_key(Context::DashboardNormal, shift_f),
            Some(Action::ToggleScopeFilter)
        );
    }
}

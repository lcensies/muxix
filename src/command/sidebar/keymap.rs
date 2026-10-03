//! Sidebar key bindings, driven by the shared registry ([`crate::tui::binding`]).
//!
//! The sidebar and the dashboard each define their own action enum but share
//! the registry machinery, so shortcuts are declared once and stay consistent.

use crate::tui::Binding;
use crate::tui::chord::KeyChord as K;
use crossterm::event::KeyCode;

use super::app::SidebarApp;

/// Canonical context name for the sidebar (single context).
pub const SIDEBAR: &str = "sidebar";

/// Actions the sidebar can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAction {
    Quit,
    Next,
    Previous,
    JumpToSelected,
    SelectLast,
    ToggleLayout,
    ToggleSleeping,
    HalfPageDown,
    HalfPageUp,
}

pub const SIDEBAR_BINDINGS: &[Binding<SidebarAction>] = &[
    Binding {
        action: Some(SidebarAction::Quit),
        chords: &[K::key('q'), K::code(KeyCode::Esc), K::ctrl('c')],
        hint: "q/Esc",
        label: "Close sidebar",
        palette: true,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::Next),
        chords: &[K::key('j'), K::code(KeyCode::Down)],
        hint: "j/↓",
        label: "Next agent",
        palette: false,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::Previous),
        chords: &[K::key('k'), K::code(KeyCode::Up)],
        hint: "k/↑",
        label: "Previous agent",
        palette: false,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::JumpToSelected),
        chords: &[K::code(KeyCode::Enter)],
        hint: "Enter",
        label: "Jump to agent",
        palette: true,
        contexts: &[SIDEBAR],
    },
    // `g` is not in this table: it is a vim-style prefix handled in
    // `handle_key` (`gg` = first agent).
    Binding {
        action: Some(SidebarAction::HalfPageDown),
        chords: &[K::ctrl('d')],
        hint: "Ctrl+d",
        label: "Half page down",
        palette: false,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::HalfPageUp),
        chords: &[K::ctrl('u')],
        hint: "Ctrl+u",
        label: "Half page up",
        palette: false,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::SelectLast),
        chords: &[K::key('G')],
        hint: "G",
        label: "Last agent",
        palette: true,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::ToggleLayout),
        chords: &[K::key('v')],
        hint: "v",
        label: "Toggle layout",
        palette: true,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::ToggleSleeping),
        chords: &[K::key('z')],
        hint: "z",
        label: "Toggle sleeping",
        palette: true,
        contexts: &[SIDEBAR],
    },
];

/// Apply a resolved sidebar action to the app.
pub fn apply_action(app: &mut SidebarApp, action: SidebarAction) {
    match action {
        SidebarAction::Quit => {
            app.quit_reason = Some("user keypress".to_string());
            app.should_quit = true;
        }
        SidebarAction::Next => app.next(),
        SidebarAction::Previous => app.previous(),
        SidebarAction::JumpToSelected => app.jump_to_selected(),
        SidebarAction::SelectLast => app.select_last(),
        SidebarAction::ToggleLayout => app.toggle_layout_mode(),
        SidebarAction::ToggleSleeping => app.toggle_sleeping(),
        SidebarAction::HalfPageDown => app.half_page_down(),
        SidebarAction::HalfPageUp => app.half_page_up(),
    }
}

/// Full key entry point: vim-style pending state (`gg`, count prefixes)
/// resolved first, then the binding registry.
pub fn handle_key(app: &mut SidebarApp, chord: K) {
    // `g` pending: a second `g` selects first; anything else cancels the
    // prefix and is processed normally.
    if app.pending_g {
        app.pending_g = false;
        if chord == K::key('g') {
            app.pending_count = None;
            app.select_first();
            return;
        }
    } else if chord == K::key('g') {
        app.pending_g = true;
        return;
    }

    // Digits accumulate a count (no sidebar binding uses digits).
    if let crossterm::event::KeyCode::Char(c) = chord.code
        && chord.mods.is_empty()
        && c.is_ascii_digit()
        && (c != '0' || app.pending_count.is_some())
    {
        let d = c as usize - '0' as usize;
        app.pending_count = Some(app.pending_count.unwrap_or(0).saturating_mul(10) + d);
        return;
    }

    if let Some(n) = app.pending_count.take() {
        if chord == K::key('j') || chord == K::code(KeyCode::Down) {
            app.move_down(n);
            return;
        }
        if chord == K::key('k') || chord == K::code(KeyCode::Up) {
            app.move_up(n);
            return;
        }
        if chord == K::key('G') {
            app.select_nth(n);
            return;
        }
        // Any other key discards the count and proceeds normally.
    }

    if let Some(action) = crate::tui::binding::lookup(SIDEBAR_BINDINGS, SIDEBAR, chord) {
        apply_action(app, action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::binding::{assert_no_conflicts, lookup};
    use crossterm::event::{KeyEvent, KeyModifiers};

    #[test]
    fn no_conflicts() {
        assert_no_conflicts(SIDEBAR_BINDINGS);
    }

    #[test]
    fn resolves_core_keys() {
        let chord = KeyChord_from('q', KeyModifiers::NONE);
        assert_eq!(
            lookup(SIDEBAR_BINDINGS, SIDEBAR, chord),
            Some(SidebarAction::Quit)
        );
        assert_eq!(
            lookup(
                SIDEBAR_BINDINGS,
                SIDEBAR,
                KeyChord_from('G', KeyModifiers::SHIFT)
            ),
            Some(SidebarAction::SelectLast)
        );
        // `g` is a vim prefix handled in handle_key, not a binding
        assert_eq!(
            lookup(
                SIDEBAR_BINDINGS,
                SIDEBAR,
                KeyChord_from('g', KeyModifiers::NONE)
            ),
            None
        );
    }

    fn nav_app() -> SidebarApp {
        SidebarApp::test_with_agents(&["/tmp/x/a", "/tmp/x/a", "/tmp/x/a", "/tmp/x/a"])
    }

    #[test]
    fn gg_selects_first() {
        let mut app = nav_app();
        app.select_last();
        handle_key(&mut app, K::key('g'));
        assert!(app.pending_g);
        handle_key(&mut app, K::key('g'));
        assert!(!app.pending_g);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn g_then_other_key_cancels_prefix() {
        let mut app = nav_app();
        app.list_state.select(Some(0));
        handle_key(&mut app, K::key('g'));
        handle_key(&mut app, K::key('j'));
        assert!(!app.pending_g);
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn count_prefix_moves_n() {
        let mut app = nav_app();
        app.list_state.select(Some(0));
        handle_key(&mut app, K::key('2'));
        assert_eq!(app.pending_count, Some(2));
        handle_key(&mut app, K::key('j'));
        assert_eq!(app.pending_count, None);
        assert_eq!(app.list_state.selected(), Some(2));
        handle_key(&mut app, K::key('1'));
        handle_key(&mut app, K::key('0'));
        assert_eq!(app.pending_count, Some(10));
        handle_key(&mut app, K::key('k'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn count_g_selects_nth() {
        let mut app = nav_app();
        handle_key(&mut app, K::key('3'));
        handle_key(&mut app, K::key('G'));
        assert_eq!(app.list_state.selected(), Some(2));
    }

    #[test]
    fn count_cleared_by_unrelated_key() {
        let mut app = nav_app();
        let layout = app.layout_mode;
        handle_key(&mut app, K::key('3'));
        handle_key(&mut app, K::key('v'));
        assert_eq!(app.pending_count, None);
        assert_ne!(app.layout_mode, layout);
    }

    #[test]
    fn plain_j_still_wraps() {
        let mut app = nav_app();
        app.select_last();
        handle_key(&mut app, K::key('j'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[allow(non_snake_case)]
    fn KeyChord_from(c: char, mods: KeyModifiers) -> crate::tui::chord::KeyChord {
        crate::tui::chord::KeyChord::from_event(KeyEvent::new(KeyCode::Char(c), mods))
    }
}

//! Sidebar key bindings, driven by the shared registry ([`crate::tui::binding`]).
//!
//! The sidebar and the dashboard each define their own action enum but share
//! the registry machinery, so shortcuts are declared once and stay consistent.

use crate::tui::Binding;
use crate::tui::chord::KeyChord as K;
use crossterm::event::KeyCode;

use super::app::{ModalIntent, SidebarApp, SidebarMode};
use crate::tui::modal::ModalOutcome;

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
    DeleteSelected,
    NewAgent,
    ShowActions,
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
    Binding {
        action: Some(SidebarAction::DeleteSelected),
        chords: &[K::key('d')],
        hint: "d",
        label: "Delete agent",
        palette: true,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::NewAgent),
        chords: &[K::key('a')],
        hint: "a",
        label: "New agent",
        palette: true,
        contexts: &[SIDEBAR],
    },
    Binding {
        action: Some(SidebarAction::ShowActions),
        chords: &[K::key('?')],
        hint: "?",
        label: "Show actions",
        // Opening the action list from inside the action list buys nothing.
        palette: false,
        contexts: &[SIDEBAR],
    },
];

/// Apply a resolved sidebar action to the app.
pub fn apply_action(app: &mut SidebarApp, action: SidebarAction) {
    match action {
        SidebarAction::Quit => {
            app.quit = Some(super::app::Quit::All("user keypress".to_string()));
        }
        SidebarAction::Next => app.next(),
        SidebarAction::Previous => app.previous(),
        SidebarAction::JumpToSelected => app.jump_to_selected(),
        SidebarAction::SelectLast => app.select_last(),
        SidebarAction::ToggleLayout => app.toggle_layout_mode(),
        SidebarAction::ToggleSleeping => app.toggle_sleeping(),
        SidebarAction::HalfPageDown => app.half_page_down(),
        SidebarAction::HalfPageUp => app.half_page_up(),
        SidebarAction::DeleteSelected => app.begin_delete(),
        SidebarAction::NewAgent => app.open_create_form(),
        SidebarAction::ShowActions => app.open_action_list(),
    }
}

/// Key entry point for the event loop.
///
/// An open modal owns the keyboard, so it is served before the vim prefixes
/// and the binding registry: `Esc` must close it rather than quit the sidebar,
/// and a bound key must not also fire its action underneath.
pub fn handle_key_event(app: &mut SidebarApp, key: crossterm::event::KeyEvent) {
    if let SidebarMode::Modal { modal, .. } = &mut app.mode {
        let outcome = modal.handle_key(&key);
        apply_modal_outcome(app, outcome);
        return;
    }
    handle_key(app, K::from_event(key));
}

fn apply_modal_outcome(app: &mut SidebarApp, outcome: ModalOutcome<SidebarAction>) {
    match outcome {
        ModalOutcome::Consumed => {}
        ModalOutcome::Cancel => app.mode = SidebarMode::normal(),
        ModalOutcome::Confirmed => {
            if let SidebarMode::Modal {
                intent: ModalIntent::DeleteAgent(agent),
                ..
            } = std::mem::replace(&mut app.mode, SidebarMode::normal())
            {
                app.delete_agent_now(&agent);
            }
        }
        ModalOutcome::Submit(values) => {
            app.mode = SidebarMode::normal();
            // The worker reports for itself; the loop does not wait on it.
            let _ = app.submit_create(&values);
        }
        ModalOutcome::Run(action) => {
            app.mode = SidebarMode::normal();
            apply_action(app, action);
        }
    }
}

/// Full key entry point: vim-style pending state (`gg`, count prefixes)
/// resolved first, then the binding registry.
pub fn handle_key(app: &mut SidebarApp, chord: K) {
    // A pending delete owns the keyboard: only Enter confirms, and every other
    // key cancels — so a stray Enter typed after moving the selection can never
    // delete the wrong agent. Checked first because Esc is otherwise Quit.
    if app.mode.pending_delete().is_some() {
        if chord == K::code(KeyCode::Enter) {
            app.confirm_delete();
        } else {
            app.cancel_delete();
        }
        return;
    }

    // `g` pending: a second `g` selects first; anything else cancels the
    // prefix and is processed normally.
    let count = app.mode.count();
    if matches!(app.mode, SidebarMode::PendingG { .. }) {
        app.mode = SidebarMode::Normal { count };
        if chord == K::key('g') {
            app.mode = SidebarMode::normal();
            app.select_first();
            return;
        }
    } else if chord == K::key('g') {
        app.mode = SidebarMode::PendingG { count };
        return;
    }

    // Digits accumulate a count (no sidebar binding uses digits).
    if let crossterm::event::KeyCode::Char(c) = chord.code
        && chord.mods.is_empty()
        && c.is_ascii_digit()
        && (c != '0' || count.is_some())
    {
        let d = c as usize - '0' as usize;
        app.mode = SidebarMode::Normal {
            count: Some(count.unwrap_or(0).saturating_mul(10) + d),
        };
        return;
    }

    if let Some(n) = count {
        app.mode = SidebarMode::normal();
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
    fn a_pending_delete_is_cancelled_by_any_key_but_enter() {
        // Only Enter confirms. Every other key cancels and is consumed, so Esc and
        // q do not also close the sidebar, and j does not move the selection onto
        // an agent that a following Enter would then delete.
        for chord in [K::code(KeyCode::Esc), K::key('q'), K::key('j'), K::key('d')] {
            let mut app = nav_app();
            app.list_state.select(Some(0));
            let agent = app.agents[0].clone();
            app.mode = SidebarMode::PendingDelete(Box::new(agent), "prompt".to_string());
            handle_key(&mut app, chord);
            assert!(app.mode.pending_delete().is_none(), "{chord:?}");
            assert!(app.quit.is_none(), "{chord:?}");
            assert_eq!(app.list_state.selected(), Some(0), "{chord:?}");
        }
    }

    #[test]
    fn the_quit_key_shuts_every_sidebar_down() {
        let mut app = nav_app();
        handle_key(&mut app, K::key('q'));
        assert!(matches!(app.quit, Some(super::super::app::Quit::All(_))));
    }

    #[test]
    fn gg_selects_first() {
        let mut app = nav_app();
        app.select_last();
        handle_key(&mut app, K::key('g'));
        assert!(matches!(app.mode, SidebarMode::PendingG { .. }));
        handle_key(&mut app, K::key('g'));
        assert!(app.mode.is_normal());
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn g_then_other_key_cancels_prefix() {
        let mut app = nav_app();
        app.list_state.select(Some(0));
        handle_key(&mut app, K::key('g'));
        handle_key(&mut app, K::key('j'));
        assert!(app.mode.is_normal());
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn count_prefix_moves_n() {
        let mut app = nav_app();
        app.list_state.select(Some(0));
        handle_key(&mut app, K::key('2'));
        assert_eq!(app.mode.count(), Some(2));
        handle_key(&mut app, K::key('j'));
        assert_eq!(app.mode.count(), None);
        assert_eq!(app.list_state.selected(), Some(2));
        handle_key(&mut app, K::key('1'));
        handle_key(&mut app, K::key('0'));
        assert_eq!(app.mode.count(), Some(10));
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
        assert_eq!(app.mode.count(), None);
        assert_ne!(app.layout_mode, layout);
    }

    #[test]
    fn plain_j_still_wraps() {
        let mut app = nav_app();
        app.select_last();
        handle_key(&mut app, K::key('j'));
        assert_eq!(app.list_state.selected(), Some(0));
    }

    fn event_for(chord: crate::tui::chord::KeyChord) -> KeyEvent {
        KeyEvent::new(chord.code, chord.mods)
    }

    fn bound_chords() -> Vec<crate::tui::chord::KeyChord> {
        SIDEBAR_BINDINGS
            .iter()
            .flat_map(|b| b.chords.iter().copied())
            .collect()
    }

    #[test]
    fn the_new_bindings_resolve() {
        assert_eq!(
            lookup(SIDEBAR_BINDINGS, SIDEBAR, K::key('a')),
            Some(SidebarAction::NewAgent)
        );
        assert_eq!(
            lookup(SIDEBAR_BINDINGS, SIDEBAR, K::key('?')),
            Some(SidebarAction::ShowActions)
        );
    }

    #[test]
    fn an_open_modal_swallows_every_bound_key() {
        for chord in bound_chords() {
            // Enter submits the form rather than jumping to an agent; the
            // submit path is covered in `crate::tui::modal`.
            if chord == K::code(KeyCode::Enter) {
                continue;
            }
            let mut app = nav_app();
            app.list_state.select(Some(0));
            let layout = app.layout_mode;
            app.open_create_form();

            handle_key_event(&mut app, event_for(chord));

            assert!(app.quit.is_none(), "{chord:?} must not quit the sidebar");
            assert_eq!(
                app.list_state.selected(),
                Some(0),
                "{chord:?} must not move the selection"
            );
            assert_eq!(app.layout_mode, layout, "{chord:?} must not act underneath");
            if chord == K::code(KeyCode::Esc) {
                assert!(app.mode.is_normal(), "Esc closes the modal");
            } else {
                assert!(
                    matches!(app.mode, SidebarMode::Modal { .. }),
                    "{chord:?} must leave the modal open"
                );
            }
        }
    }

    #[test]
    fn the_action_list_shows_exactly_the_palette_bindings() {
        let mut app = nav_app();
        app.open_action_list();
        let SidebarMode::Modal {
            modal: crate::tui::modal::Modal::ActionList { state, .. },
            ..
        } = &app.mode
        else {
            panic!("expected the action list");
        };
        let shown: Vec<&str> = state.commands.iter().map(|c| c.label).collect();
        let expected: Vec<&str> = SIDEBAR_BINDINGS
            .iter()
            .filter(|b| b.palette && b.contexts.contains(&SIDEBAR))
            .map(|b| b.label)
            .collect();
        assert_eq!(shown, expected);
        assert!(
            !shown.contains(&"Show actions"),
            "the action list does not list itself"
        );
    }

    #[test]
    fn filtering_the_action_list_and_accepting_runs_that_action() {
        let mut app = nav_app();
        let layout = app.layout_mode;
        app.open_action_list();
        for c in "toggle la".chars() {
            handle_key_event(&mut app, event_for(K::key(c)));
        }
        handle_key_event(&mut app, event_for(K::code(KeyCode::Enter)));
        assert!(app.mode.is_normal(), "accepting closes the list");
        assert_ne!(app.layout_mode, layout, "the filtered action ran");
    }

    #[allow(non_snake_case)]
    fn KeyChord_from(c: char, mods: KeyModifiers) -> crate::tui::chord::KeyChord {
        crate::tui::chord::KeyChord::from_event(KeyEvent::new(KeyCode::Char(c), mods))
    }
}

//! A single source of truth for key bindings.
//!
//! Each [`Binding`] couples an action with the chords that trigger it, a
//! display hint, a human label, and the contexts it applies to. The keymap
//! lookup, the command palette, and the help overlay are all *derived* from one
//! binding table per TUI, eliminating the drift that comes from maintaining
//! those three lists by hand.
//!
//! Generic over the action type so the dashboard and the sidebar can each keep
//! their own action enum while sharing this machinery.

use super::chord::KeyChord;

/// One entry in a binding table.
///
/// Authored as a `const` so tables are fully static (no per-frame allocation).
pub struct Binding<A: 'static> {
    /// Action dispatched when a chord matches. `None` marks a *display-only*
    /// row (e.g. `1-9 → Quick jump`) whose keys are handled dynamically by the
    /// owning TUI but which should still appear in the help overlay.
    pub action: Option<A>,
    /// Chords that trigger `action`. Order is irrelevant; matching is exact.
    pub chords: &'static [KeyChord],
    /// Short hint shown in help/palette (e.g. `"j/k"`, `"Ctrl+u"`, `"1-9"`).
    /// Empty hides the entry from the help overlay.
    pub hint: &'static str,
    /// Human-readable label (e.g. `"Next agent"`).
    pub label: &'static str,
    /// Whether this entry is offered in the command palette.
    pub palette: bool,
    /// Canonical context names this binding applies in.
    pub contexts: &'static [&'static str],
}

impl<A: 'static> Binding<A> {
    fn applies_in(&self, ctx: &str) -> bool {
        self.contexts.contains(&ctx)
    }
}

/// Resolve a chord to an action within a context. Returns the first matching
/// binding's action (tables must not bind the same chord to two actions in one
/// context; [`assert_no_conflicts`] guards this in tests).
pub fn lookup<A: Clone + 'static>(
    bindings: &[Binding<A>],
    ctx: &str,
    chord: KeyChord,
) -> Option<A> {
    for b in bindings {
        if let Some(action) = &b.action
            && b.applies_in(ctx)
            && b.chords.contains(&chord)
        {
            return Some(action.clone());
        }
    }
    None
}

/// Bindings offered in the command palette for a context, in table order.
pub fn palette_entries<'a, A: 'static>(
    bindings: &'a [Binding<A>],
    ctx: &str,
) -> Vec<&'a Binding<A>> {
    bindings
        .iter()
        .filter(|b| b.palette && b.action.is_some() && b.applies_in(ctx))
        .collect()
}

/// Help rows `(hint, label)` for a context, in table order. Entries with an
/// empty hint are omitted.
pub fn help_rows<A: 'static>(
    bindings: &[Binding<A>],
    ctx: &str,
) -> Vec<(&'static str, &'static str)> {
    bindings
        .iter()
        .filter(|b| !b.hint.is_empty() && b.applies_in(ctx))
        .map(|b| (b.hint, b.label))
        .collect()
}

/// All distinct context names referenced by a table.
pub fn contexts_of<A: 'static>(bindings: &[Binding<A>]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for b in bindings {
        for c in b.contexts {
            if !out.contains(c) {
                out.push(*c);
            }
        }
    }
    out
}

/// Panic if any context binds one chord to two different actions, or repeats a
/// help hint within a context. Intended for use in unit tests to guarantee the
/// derived keymap/help stay unambiguous.
#[cfg(test)]
pub fn assert_no_conflicts<A: PartialEq + std::fmt::Debug + 'static>(bindings: &[Binding<A>]) {
    use std::collections::HashMap;
    for ctx in contexts_of(bindings) {
        // Chord -> action uniqueness.
        let mut by_chord: HashMap<KeyChord, &A> = HashMap::new();
        for b in bindings {
            let Some(action) = &b.action else { continue };
            if !b.applies_in(ctx) {
                continue;
            }
            for chord in b.chords {
                if let Some(prev) = by_chord.insert(*chord, action)
                    && prev != action
                {
                    panic!(
                        "chord {chord:?} bound to two actions in {ctx:?}: {prev:?} and {action:?}"
                    );
                }
            }
        }
        // Help hint uniqueness (the dashboard's `test_no_duplicate_keys`).
        let mut hints = std::collections::HashSet::new();
        for (hint, _) in help_rows(bindings, ctx) {
            assert!(
                hints.insert(hint),
                "duplicate help hint {hint:?} in context {ctx:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum A {
        Quit,
        Next,
        Prev,
    }

    const QUIT: &[KeyChord] = &[KeyChord::key('q'), KeyChord::code(KeyCode::Esc)];
    const NEXT: &[KeyChord] = &[KeyChord::key('j'), KeyChord::ctrl('n')];
    const PREV: &[KeyChord] = &[KeyChord::key('k')];

    const TABLE: &[Binding<A>] = &[
        Binding {
            action: Some(A::Quit),
            chords: QUIT,
            hint: "q",
            label: "Quit",
            palette: true,
            contexts: &["Normal", "Other"],
        },
        Binding {
            action: Some(A::Next),
            chords: NEXT,
            hint: "j",
            label: "Next",
            palette: true,
            contexts: &["Normal"],
        },
        Binding {
            action: Some(A::Prev),
            chords: PREV,
            hint: "k",
            label: "Prev",
            palette: false,
            contexts: &["Normal"],
        },
        Binding {
            action: None,
            chords: &[],
            hint: "1-9",
            label: "Quick jump",
            palette: false,
            contexts: &["Normal"],
        },
    ];

    #[test]
    fn lookup_matches_any_chord() {
        assert_eq!(lookup(TABLE, "Normal", KeyChord::key('j')), Some(A::Next));
        assert_eq!(lookup(TABLE, "Normal", KeyChord::ctrl('n')), Some(A::Next));
        assert_eq!(lookup(TABLE, "Normal", KeyChord::key('q')), Some(A::Quit));
        assert_eq!(lookup(TABLE, "Normal", KeyChord::key('z')), None);
    }

    #[test]
    fn lookup_respects_context() {
        assert_eq!(lookup(TABLE, "Other", KeyChord::key('j')), None);
        assert_eq!(lookup(TABLE, "Other", KeyChord::key('q')), Some(A::Quit));
    }

    #[test]
    fn palette_only_flagged() {
        let p = palette_entries(TABLE, "Normal");
        let labels: Vec<_> = p.iter().map(|b| b.label).collect();
        assert_eq!(labels, vec!["Quit", "Next"]);
    }

    #[test]
    fn help_includes_display_only() {
        let h = help_rows(TABLE, "Normal");
        assert!(h.contains(&("1-9", "Quick jump")));
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn no_conflicts() {
        assert_no_conflicts(TABLE);
    }
}

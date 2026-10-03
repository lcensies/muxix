//! Structured, greppable pipeline event tracing.
//!
//! The pipeline runner drives agents *out of band* — it sends prompts into tmux
//! panes, waits on filesystem signals written by agent hooks (in a *different*
//! process), and advances a DAG. When something is slow ("the prompt took ages
//! to appear") or stuck ("implement never handed off to test"), the only way to
//! see *why* is a timeline of the discrete events on both sides.
//!
//! # Model: spans carry context, events mark transitions
//!
//! Ambient context (which node, what kind, which pane, how long) lives on
//! **spans**, not on every call site. The scheduler enters a [`wm_span!`]
//! `"node"` span per node; the pane driver enters a nested `"turn"` span per
//! prompt. Because the fmt subscriber prints the active span scope on every
//! line, all events inside automatically carry `node{node=… kind=…}:turn{pane=…}:`
//! — so individual events stay lean (`wm_evt!("prompt.sent", bytes = n)`), and
//! span close lines report elapsed time (`time.busy`) for free.
//!
//! Every event/span uses the dedicated tracing target [`TARGET`], so they land
//! in the normal `muxix.log` (file, never the pane) and are trivially
//! greppable, and can be silenced wholesale in production:
//!
//! ```text
//! grep 'ev=' ~/.local/state/muxix.log               # whole event timeline
//! grep 'ev="turn.done"' ~/.local/state/muxix.log     # one event kind
//! grep 'node=implement' ~/.local/state/muxix.log     # one node (span context)
//! grep 'pane=%5' ~/.local/state/muxix.log            # one pane (span context)
//! ```
//!
//! Because `muxix signal …` (the agent hook subprocess) also initialises the
//! logger, its `signal.write` events interleave with the runner's observe
//! events in the *same* file — so signal-write→observe latency is directly
//! measurable from the timestamps (and the convenience `age_ms` field),
//! distinguishing "the hook fired late" from "the runner polled late".
//!
//! # Levels & disabling
//!
//! Lifecycle events (`node.run`, `prompt.sent`, `turn.done`, …) are `info` and
//! on by default. High-frequency per-poll/per-tick events are `debug` so the
//! default log isn't flooded. Control the *level* independently of everything
//! else via the `MUXIX_EVENTS` env var (`off` | `info` | `debug` | `trace`),
//! or with a standard `RUST_LOG=wm::event=debug` directive. See [`crate::logger`].
//!
//! # Per-kind filtering from config
//!
//! The level is all-or-nothing per level; to silence *specific* kinds or whole
//! groups, the `events:` section of `.muxix.yaml` installs an [`EventFilter`]
//! (via [`set_filter`]) that the emit macros consult through [`should_emit`]:
//!
//! ```yaml
//! events:
//!   enabled: true            # master switch; false silences all wm::event
//!   level: debug             # off|info|debug|trace (ignored if MUXIX_EVENTS set)
//!   disable: [pane.probe, turn.poll]   # group prefix or exact kind
//!   only: []                 # allowlist; non-empty => only these are emitted
//! ```
//!
//! Precedence: `enabled: false` wins over all; level is `MUXIX_EVENTS` env >
//! `events.level` > default `info`; per-kind `only` then `disable` are applied
//! on top of whatever the level lets through.

use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Tracing target every pipeline span and event shares. Filter or silence with
/// `MUXIX_EVENTS=off` / `RUST_LOG=wm::event=off`.
pub const TARGET: &str = "wm::event";

/// Runtime per-event-kind filter, configured from `.muxix.yaml` (`events:`).
///
/// The `MUXIX_EVENTS` env var / `RUST_LOG` directive controls the *level*
/// (`off|info|debug|trace`) at the tracing subscriber, but it cannot single out
/// individual event kinds. This filter does: it is consulted by the [`wm_evt!`]
/// / [`wm_evt_dbg!`] macros before they emit, so specific kinds or whole groups
/// can be silenced while the rest of that level keeps flowing.
///
/// Matching is group-aware: a pattern matches an event kind when it equals the
/// kind exactly *or* is a dotted prefix of it — `"pane.probe"` matches
/// `pane.probe.ok` and `pane.probe.retry` but not `pane.gate.ok`.
#[derive(Debug, Clone)]
pub struct EventFilter {
    /// Master switch. `false` silences every `wm::event` regardless of the rest.
    pub enabled: bool,
    /// Allowlist. When non-empty, only kinds matching one of these are emitted.
    pub only: Vec<String>,
    /// Denylist. Kinds matching one of these are never emitted (applied after
    /// `only`).
    pub disable: Vec<String>,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            enabled: true,
            only: Vec::new(),
            disable: Vec::new(),
        }
    }
}

impl EventFilter {
    /// Whether this filter imposes any restriction. An unrestricted filter lets
    /// every kind through, so emission can skip the lock entirely.
    fn is_restrictive(&self) -> bool {
        !self.enabled || !self.only.is_empty() || !self.disable.is_empty()
    }

    /// A pattern matches a kind when it equals the kind or is a dotted prefix of
    /// it (`"pane.probe"` ⊇ `pane.probe.ok`).
    fn pattern_matches(pat: &str, kind: &str) -> bool {
        kind == pat
            || (kind.len() > pat.len()
                && kind.as_bytes()[pat.len()] == b'.'
                && kind.starts_with(pat))
    }

    /// Whether an event of `kind` should be emitted under this filter.
    pub fn allows(&self, kind: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.only.is_empty() && !self.only.iter().any(|p| Self::pattern_matches(p, kind)) {
            return false;
        }
        if self.disable.iter().any(|p| Self::pattern_matches(p, kind)) {
            return false;
        }
        true
    }
}

/// Fast path: stays `false` while the installed filter is unrestricted, so the
/// hot emit path avoids touching the lock on every event.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static FILTER: RwLock<Option<EventFilter>> = RwLock::new(None);

/// Install the per-kind event filter (called after config load). Idempotent and
/// safe to call on every config load; the latest call wins.
pub fn set_filter(filter: EventFilter) {
    let restrictive = filter.is_restrictive();
    if let Ok(mut slot) = FILTER.write() {
        *slot = Some(filter);
    }
    ACTIVE.store(restrictive, Ordering::Release);
}

/// Whether an event of `kind` should be emitted. Returns `true` immediately
/// (no lock) when no restrictive filter is installed — the common case.
#[inline]
pub fn should_emit(kind: &str) -> bool {
    if !ACTIVE.load(Ordering::Acquire) {
        return true;
    }
    match FILTER.read() {
        Ok(slot) => slot.as_ref().map(|f| f.allows(kind)).unwrap_or(true),
        Err(_) => true,
    }
}

/// Open a pipeline context span on [`TARGET`]. Fields set here (`node`, `kind`,
/// `pane`, …) are printed on every event emitted while the span is entered, so
/// events themselves carry no repeated context. Returns a [`tracing::Span`];
/// hold its `.entered()` guard for the scope.
///
/// `let _g = wm_span!("node", node = %id, kind = node_kind_label(n)).entered();`
#[macro_export]
macro_rules! wm_span {
    ($($tt:tt)*) => {
        ::tracing::info_span!(target: $crate::signals::event::TARGET, $($tt)*)
    };
}

/// Emit a lifecycle pipeline event (`info` — on by default). The first argument
/// is the event kind (recorded as `ev="…"`); the rest are forwarded verbatim to
/// `tracing`, so `%`/`?` sigils and `key = value` pairs work as usual.
///
/// `wm_evt!("prompt.sent", bytes = n, multiline = ml)`
#[macro_export]
macro_rules! wm_evt {
    ($kind:expr) => {
        if $crate::signals::event::should_emit($kind) {
            ::tracing::info!(target: $crate::signals::event::TARGET, ev = $kind)
        }
    };
    ($kind:expr, $($tt:tt)*) => {
        if $crate::signals::event::should_emit($kind) {
            ::tracing::info!(target: $crate::signals::event::TARGET, ev = $kind, $($tt)*)
        }
    };
}

/// Emit a high-frequency pipeline event (`debug` — enable with
/// `MUXIX_EVENTS=debug`). Use for per-poll / per-tick traces that would
/// otherwise flood the default log.
#[macro_export]
macro_rules! wm_evt_dbg {
    ($kind:expr) => {
        if $crate::signals::event::should_emit($kind) {
            ::tracing::debug!(target: $crate::signals::event::TARGET, ev = $kind)
        }
    };
    ($kind:expr, $($tt:tt)*) => {
        if $crate::signals::event::should_emit($kind) {
            ::tracing::debug!(target: $crate::signals::event::TARGET, ev = $kind, $($tt)*)
        }
    };
}

/// Age, in milliseconds, of a signal file (now − its mtime). Best-effort: `None`
/// if the file is gone or its mtime is unreadable. A large value points at the
/// *writer* (hook fired long ago, runner only just noticed) rather than at the
/// poll interval.
#[allow(dead_code)]
pub fn age_ms(path: &Path) -> Option<u128> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified.elapsed().ok().map(|d| d.as_millis())
}

/// Milliseconds elapsed since `start`, as a `u64` (saturating). For the explicit
/// `elapsed_ms` on transition events where a span's own close timing isn't the
/// quantity of interest (e.g. signal-observe latency within a turn).
#[allow(dead_code)]
pub fn since_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(only: &[&str], disable: &[&str]) -> EventFilter {
        EventFilter {
            enabled: true,
            only: only.iter().map(|s| s.to_string()).collect(),
            disable: disable.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_filter_passes_everything() {
        let f = EventFilter::default();
        assert!(!f.is_restrictive());
        assert!(f.allows("turn.done"));
        assert!(f.allows("pane.probe.ok"));
    }

    #[test]
    fn exact_kind_disable() {
        let f = filter(&[], &["turn.poll"]);
        assert!(f.is_restrictive());
        assert!(!f.allows("turn.poll"));
        assert!(f.allows("turn.done"));
    }

    #[test]
    fn group_prefix_disable() {
        let f = filter(&[], &["pane.probe"]);
        // whole group silenced
        assert!(!f.allows("pane.probe.ok"));
        assert!(!f.allows("pane.probe.retry"));
        assert!(!f.allows("pane.probe")); // exact match too
        // sibling group untouched
        assert!(f.allows("pane.gate.ok"));
        // not a dotted-prefix match: must not silence `pane.probexyz`
        assert!(f.allows("pane.probexyz"));
    }

    #[test]
    fn only_allowlist() {
        let f = filter(&["node", "turn.done"], &[]);
        assert!(f.allows("node.run")); // group prefix in allowlist
        assert!(f.allows("turn.done")); // exact in allowlist
        assert!(!f.allows("pane.probe.ok")); // not in allowlist
    }

    #[test]
    fn only_then_disable() {
        // allowlist admits the group, disable carves out one kind
        let f = filter(&["pane"], &["pane.probe"]);
        assert!(f.allows("pane.gate.ok"));
        assert!(!f.allows("pane.probe.ok"));
    }

    #[test]
    fn set_filter_drives_should_emit() {
        // Exercises the real runtime global the config install path writes to.
        // Default install (no restriction) — fast path, everything passes.
        set_filter(EventFilter::default());
        assert!(should_emit("config.load"));
        assert!(should_emit("turn.done"));

        // Install a restrictive filter (as `events.disable: [config.load, pane.probe]`).
        set_filter(filter(&[], &["config.load", "pane.probe"]));
        assert!(!should_emit("config.load"));
        assert!(!should_emit("pane.probe.ok")); // group prefix
        assert!(should_emit("cmd.dispatch")); // untouched

        // Reset so other tests see the unrestricted fast path.
        set_filter(EventFilter::default());
        assert!(should_emit("config.load"));
    }

    #[test]
    fn master_disabled_blocks_all() {
        let f = EventFilter {
            enabled: false,
            only: vec![],
            disable: vec![],
        };
        assert!(f.is_restrictive());
        assert!(!f.allows("turn.done"));
        assert!(!f.allows("app.start"));
    }
}

//! Data structures for the per-project runtime state store.
//!
//! This is runtime FACTS — tri-state capability flags and facts discovered or
//! produced by setup (e.g. `test_command`, `build_command`). It is *not*
//! workflow config; the workflow itself lives in `.muxix/workflows/harness.yaml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Schema version written into new `project.json` files.
pub const CURRENT_VERSION: u32 = 1;

/// Tri-state status of a capability discovered/produced by setup.
///
/// The lifecycle is `Absent` → `InProgress` → `Done`. Only one actor may hold a
/// capability in `InProgress` at a time; the transition out of `Absent` is a
/// compare-and-swap guarded by the project lock (see [`super::store`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Not yet attempted by anyone.
    #[default]
    Absent,
    /// An actor currently holds the slot and is working on it.
    InProgress,
    /// Completed successfully; no further work needed.
    Done,
}

impl CapabilityStatus {
    /// Lowercase wire name, matching the serde representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityStatus::Absent => "absent",
            CapabilityStatus::InProgress => "in_progress",
            CapabilityStatus::Done => "done",
        }
    }
}

/// A single capability slot: tri-state status plus lock/ownership metadata.
///
/// `owner`/`started_at`/`heartbeat` are only meaningful while the slot is
/// `InProgress` (and `owner` is retained on `Done` for provenance). They are
/// cleared when the slot returns to `Absent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capability {
    /// Tri-state lifecycle flag.
    pub status: CapabilityStatus,

    /// Opaque identifier of the actor that owns the slot (the one that
    /// transitioned it out of `Absent`). `None` when `Absent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Unix seconds when the current owner acquired the slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,

    /// Unix seconds of the owner's most recent heartbeat.
    ///
    /// Used for stale-lock reclaim: an `InProgress` slot whose heartbeat is
    /// older than the configured TTL is considered abandoned by a crashed owner
    /// and may be reclaimed by another actor's `acquire`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<u64>,
}

impl Capability {
    /// A capability that has not been attempted.
    pub fn absent() -> Self {
        Self::default()
    }

    /// Whether this `InProgress` slot's heartbeat has gone stale relative to
    /// `now` and `ttl_secs`. Always false for `Absent`/`Done` slots.
    pub fn is_stale(&self, now: u64, ttl_secs: u64) -> bool {
        if self.status != CapabilityStatus::InProgress {
            return false;
        }
        // A missing heartbeat falls back to `started_at`; if neither exists the
        // slot is treated as stale so it can never deadlock setup.
        let last = self.heartbeat.or(self.started_at);
        match last {
            Some(ts) => now.saturating_sub(ts) > ttl_secs,
            None => true,
        }
    }
}

/// One agent session observed in a worktree.
///
/// This is a *journal entry*, not the truth: the agent's own session store owns
/// the conversation and a user may delete it at any time. Anything acting on a
/// recorded id (e.g. `muxix resurrect`) must verify it against that store
/// first — launching a resume for a session that no longer exists makes the
/// agent exit immediately and takes its pane down with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Canonical agent name that owned the session (e.g. "claude", "opencode").
    /// Recorded per session because a worktree outlives any single agent choice.
    pub agent: String,

    /// Session identifier as the agent's own store knows it.
    pub id: String,

    /// Unix seconds when the session was first observed.
    pub started_at: u64,

    /// Unix seconds when the session was last observed as finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
}

/// What muxix knows about a worktree it created.
///
/// Holds the worktree/agent/session facts outright. `parent` in particular used
/// to live in git config (`muxix.worktree.<handle>.parent`); keeping one owner
/// avoids the two stores disagreeing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRecord {
    /// Branch checked out in the worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Handle of the worktree whose agent spawned this one. `None` for a
    /// top-level worktree created from the main worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    /// Agent the worktree was most recently launched with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,

    /// Sessions observed in this worktree, oldest first. Survives agent
    /// switches, so a worktree that ran opencode and now runs claude keeps both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionRecord>,

    /// Unix seconds when muxix created the worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_at: Option<u64>,

    /// Unix seconds of the most recent activity muxix observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

impl WorktreeRecord {
    /// Most recently started session for `agent`, if any.
    ///
    /// Scoped by agent because a session belonging to a previously configured
    /// agent is unreachable by the current one — the exact case that makes a
    /// naive "resume the last session" resurrect kill the window it restored.
    pub fn latest_session_for(&self, agent: &str) -> Option<&SessionRecord> {
        self.sessions
            .iter()
            .filter(|s| s.agent == agent)
            .max_by_key(|s| s.started_at)
    }
}

/// Per-project runtime state: tri-state capability flags plus free-form facts
/// discovered or produced by setup (e.g. `test_command`, `build_command`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectState {
    /// Schema version for forward-compatible migrations.
    #[serde(default)]
    pub version: u32,

    /// Tri-state capability flags keyed by capability name (e.g. "build", "test").
    #[serde(default)]
    pub capabilities: BTreeMap<String, Capability>,

    /// Free-form facts keyed by name (e.g. "test_command" -> "cargo test").
    #[serde(default)]
    pub facts: BTreeMap<String, String>,

    /// Worktrees muxix created in this project, keyed by handle.
    ///
    /// `default` keeps files written before this field existed readable, so no
    /// schema version bump is needed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worktrees: BTreeMap<String, WorktreeRecord>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            capabilities: BTreeMap::new(),
            facts: BTreeMap::new(),
            worktrees: BTreeMap::new(),
        }
    }
}

impl ProjectState {
    /// Read a capability, defaulting to `Absent` when never recorded.
    pub fn capability(&self, name: &str) -> Capability {
        self.capabilities.get(name).cloned().unwrap_or_default()
    }
}

/// Outcome of an [`super::store::ProjectStateStore::acquire`] call.
///
/// Mirrors the compare-and-swap semantics: at most one actor walks away owning
/// the slot for a given `Absent`/stale state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// Caller transitioned the slot `Absent` → `InProgress` and now owns it.
    Acquired,
    /// Caller reclaimed a stale `InProgress` slot abandoned by a crashed owner.
    Reclaimed { previous_owner: Option<String> },
    /// Another live actor holds the slot (its heartbeat is still fresh).
    Held { owner: Option<String> },
    /// The capability is already `Done`; no work is needed.
    AlreadyDone,
}

impl AcquireOutcome {
    /// Whether the caller now owns the slot and should proceed with setup.
    #[allow(dead_code)] // public API for gate consumers; exercised by tests
    pub fn owned(&self) -> bool {
        matches!(
            self,
            AcquireOutcome::Acquired | AcquireOutcome::Reclaimed { .. }
        )
    }

    /// Stable lowercase word for CLI/bash-gate consumption.
    pub fn as_str(&self) -> &'static str {
        match self {
            AcquireOutcome::Acquired => "acquired",
            AcquireOutcome::Reclaimed { .. } => "reclaimed",
            AcquireOutcome::Held { .. } => "held",
            AcquireOutcome::AlreadyDone => "done",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_and_done_are_never_stale() {
        let absent = Capability::absent();
        assert!(!absent.is_stale(1_000_000, 1));

        let done = Capability {
            status: CapabilityStatus::Done,
            owner: Some("a".into()),
            started_at: Some(1),
            heartbeat: Some(1),
        };
        assert!(!done.is_stale(1_000_000, 1));
    }

    #[test]
    fn in_progress_staleness_tracks_heartbeat_and_ttl() {
        let cap = Capability {
            status: CapabilityStatus::InProgress,
            owner: Some("a".into()),
            started_at: Some(100),
            heartbeat: Some(100),
        };
        assert!(!cap.is_stale(150, 100), "55s elapsed < 100s ttl");
        assert!(cap.is_stale(250, 100), "150s elapsed > 100s ttl");
    }

    #[test]
    fn in_progress_without_heartbeat_falls_back_to_started_at() {
        let cap = Capability {
            status: CapabilityStatus::InProgress,
            owner: Some("a".into()),
            started_at: Some(100),
            heartbeat: None,
        };
        assert!(cap.is_stale(300, 100));

        // Neither timestamp present → treated as stale so it can't deadlock.
        let cap = Capability {
            status: CapabilityStatus::InProgress,
            owner: Some("a".into()),
            started_at: None,
            heartbeat: None,
        };
        assert!(cap.is_stale(300, 100));
    }

    #[test]
    fn outcome_owned_classification() {
        assert!(AcquireOutcome::Acquired.owned());
        assert!(
            AcquireOutcome::Reclaimed {
                previous_owner: None
            }
            .owned()
        );
        assert!(!AcquireOutcome::Held { owner: None }.owned());
        assert!(!AcquireOutcome::AlreadyDone.owned());
    }

    #[test]
    fn project_state_default_is_versioned() {
        let state = ProjectState::default();
        assert_eq!(state.version, CURRENT_VERSION);
    }

    #[test]
    fn json_uses_snake_case_status() {
        let json = serde_json::to_string(&CapabilityStatus::InProgress).unwrap();
        assert_eq!(json, "\"in_progress\"");
    }
}

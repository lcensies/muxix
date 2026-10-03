//! Filesystem-backed read/write API for the per-project runtime state store.
//!
//! Layout (relative to the project's `.workmux/` directory):
//! ```text
//! .workmux/state/
//! ├── project.json        # the ProjectState document
//! └── project.lock        # O_EXCL mutex guarding read-modify-write
//! ```
//!
//! All mutations go through [`ProjectStateStore::mutate`], which holds the file
//! lock across read → modify → atomic-write so concurrent actors can't both win
//! the `Absent` → `InProgress` compare-and-swap.

use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing::warn;

use super::lock::{FileLock, now_secs};
use super::types::{
    AcquireOutcome, Capability, CapabilityStatus, ProjectState, SessionRecord, WorktreeRecord,
};

/// Default heartbeat TTL: an `InProgress` capability whose owner has not
/// heartbeat within this window is reclaimable by another actor.
pub const DEFAULT_CAPABILITY_TTL_SECS: u64 = 300;

/// Read/write handle for a project's runtime state store.
pub struct ProjectStateStore {
    /// The `.workmux/state` directory.
    dir: PathBuf,
}

impl ProjectStateStore {
    /// Open the store for `project_dir`, locating the project's `.workmux/`
    /// directory and ensuring `.workmux/state/` exists.
    pub fn open(project_dir: &Path) -> Result<Self> {
        let dir = crate::config::find_workmux_dir(project_dir).join("state");
        Self::with_dir(dir)
    }

    /// Open the store at an explicit `state` directory, creating it if needed.
    /// Open the single project-wide store, regardless of which worktree the
    /// caller is in.
    ///
    /// [`Self::open`] resolves the *nearest* `.workmux/`, and since every
    /// worktree is a full checkout carrying its own `.workmux.yaml`, calling it
    /// from a worktree yields that worktree's private store. Anything shared
    /// across worktrees — the worktree/session journal above all — must go
    /// through here so there is exactly one journal per project.
    pub fn open_project() -> Result<Self> {
        Self::open(&crate::git::get_main_worktree_root()?)
    }

    pub fn with_dir(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).context("Failed to create project state directory")?;
        Ok(Self { dir })
    }

    /// Path to the state document.
    pub fn json_path(&self) -> PathBuf {
        self.dir.join("project.json")
    }

    /// Path to the lock file guarding mutations.
    fn lock_path(&self) -> PathBuf {
        self.dir.join("project.lock")
    }

    /// Read the current state. Returns the default (empty) state when the file
    /// is missing, and a warned default when it is corrupt (the file is left in
    /// place for debugging rather than deleted).
    pub fn read(&self) -> Result<ProjectState> {
        let path = self.json_path();
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(state) => Ok(state),
                Err(e) => {
                    warn!(?path, error = %e, "corrupted project state, using defaults");
                    Ok(ProjectState::default())
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ProjectState::default()),
            Err(e) => Err(e).context("Failed to read project state"),
        }
    }

    /// Atomically persist `state` (temp file + rename).
    fn write(&self, state: &ProjectState) -> Result<()> {
        let path = self.json_path();
        let content = serde_json::to_string_pretty(state)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, content.as_bytes()).context("Failed to write project state temp file")?;
        fs::rename(&tmp, &path).context("Failed to rename project state temp file")?;
        Ok(())
    }

    /// Run `f` against the state while holding the lock, then persist the
    /// (possibly mutated) state. The lock makes the whole read-modify-write
    /// sequence atomic with respect to other processes.
    pub fn mutate<T>(&self, f: impl FnOnce(&mut ProjectState) -> T) -> Result<T> {
        let _guard = FileLock::acquire(&self.lock_path())?;
        let mut state = self.read()?;
        let out = f(&mut state);
        self.write(&state)?;
        Ok(out)
    }

    // ── Capability API ──────────────────────────────────────────────────────

    /// Read a capability's current slot (defaults to `Absent`).
    pub fn get_capability(&self, name: &str) -> Result<Capability> {
        Ok(self.read()?.capability(name))
    }

    /// Compare-and-swap acquire of `name` for `owner`.
    ///
    /// Transitions `Absent` → `InProgress`, or reclaims an `InProgress` slot
    /// whose heartbeat has gone stale past `ttl_secs`. Two racing actors can
    /// never both win because the read-modify-write runs under the file lock.
    pub fn acquire(&self, name: &str, owner: &str, ttl_secs: u64) -> Result<AcquireOutcome> {
        let now = now_secs();
        self.mutate(|state| {
            let cap = state.capability(name);
            match cap.status {
                CapabilityStatus::Done => AcquireOutcome::AlreadyDone,
                CapabilityStatus::Absent => {
                    state
                        .capabilities
                        .insert(name.to_string(), in_progress(owner, now));
                    AcquireOutcome::Acquired
                }
                CapabilityStatus::InProgress => {
                    // Re-acquiring your own slot is idempotent (refreshes it).
                    if cap.owner.as_deref() == Some(owner) {
                        state
                            .capabilities
                            .insert(name.to_string(), in_progress(owner, now));
                        return AcquireOutcome::Acquired;
                    }
                    if cap.is_stale(now, ttl_secs) {
                        let previous_owner = cap.owner.clone();
                        state
                            .capabilities
                            .insert(name.to_string(), in_progress(owner, now));
                        AcquireOutcome::Reclaimed { previous_owner }
                    } else {
                        AcquireOutcome::Held { owner: cap.owner }
                    }
                }
            }
        })
    }

    /// Refresh the heartbeat for an `InProgress` slot owned by `owner`.
    ///
    /// Returns `true` if refreshed, `false` if the caller no longer owns the
    /// slot (it was reclaimed, completed, or released by someone else) — a
    /// signal to the caller that it has lost the lock and should stop.
    pub fn heartbeat(&self, name: &str, owner: &str) -> Result<bool> {
        let now = now_secs();
        self.mutate(|state| match state.capabilities.get_mut(name) {
            Some(cap)
                if cap.status == CapabilityStatus::InProgress
                    && cap.owner.as_deref() == Some(owner) =>
            {
                cap.heartbeat = Some(now);
                true
            }
            _ => false,
        })
    }

    /// Mark `name` as `Done`. Succeeds when the caller owns the slot or the slot
    /// is unowned (`Absent`); returns `false` if a *different* live owner holds
    /// it (the caller lost the lock and must not clobber).
    pub fn complete(&self, name: &str, owner: &str) -> Result<bool> {
        let now = now_secs();
        self.mutate(|state| {
            let cap = state.capability(name);
            let mine = cap.owner.is_none() || cap.owner.as_deref() == Some(owner);
            if !mine && cap.status == CapabilityStatus::InProgress {
                return false;
            }
            state.capabilities.insert(
                name.to_string(),
                Capability {
                    status: CapabilityStatus::Done,
                    owner: Some(owner.to_string()),
                    started_at: cap.started_at.or(Some(now)),
                    heartbeat: Some(now),
                },
            );
            true
        })
    }

    /// Reset `name` back to `Absent`. Succeeds when the caller owns the slot or
    /// the slot is unowned; returns `false` if a different live owner holds it.
    pub fn release(&self, name: &str, owner: &str) -> Result<bool> {
        self.mutate(|state| {
            let cap = state.capability(name);
            let mine = cap.owner.is_none() || cap.owner.as_deref() == Some(owner);
            if !mine && cap.status == CapabilityStatus::InProgress {
                return false;
            }
            state
                .capabilities
                .insert(name.to_string(), Capability::absent());
            true
        })
    }

    // ── Facts API ───────────────────────────────────────────────────────────

    /// Read a fact (e.g. `test_command`), if recorded.
    pub fn get_fact(&self, key: &str) -> Result<Option<String>> {
        Ok(self.read()?.facts.get(key).cloned())
    }

    /// Record a fact discovered/produced by setup.
    pub fn set_fact(&self, key: &str, value: &str) -> Result<()> {
        self.mutate(|state| {
            state.facts.insert(key.to_string(), value.to_string());
        })
    }

    // ── Worktree / session journal ──────────────────────────────────────────

    /// What workmux knows about `handle`, if it created it.
    pub fn get_worktree(&self, handle: &str) -> Result<Option<WorktreeRecord>> {
        Ok(self.read()?.worktrees.get(handle).cloned())
    }

    /// Record a worktree at creation time.
    ///
    /// Idempotent: re-recording an existing handle refreshes its fields and
    /// `last_seen` but never drops the session history, so reopening a worktree
    /// does not erase which sessions ran in it.
    pub fn record_worktree(
        &self,
        handle: &str,
        branch: Option<&str>,
        parent: Option<&str>,
        agent: Option<&str>,
    ) -> Result<()> {
        let now = now_secs();
        self.mutate(|state| {
            let entry = state
                .worktrees
                .entry(handle.to_string())
                .or_insert_with(|| WorktreeRecord {
                    branch: None,
                    parent: None,
                    agent: None,
                    sessions: Vec::new(),
                    spawned_at: Some(now),
                    last_seen: None,
                });
            if let Some(branch) = branch {
                entry.branch = Some(branch.to_string());
            }
            if let Some(parent) = parent {
                entry.parent = Some(parent.to_string());
            }
            if let Some(agent) = agent {
                entry.agent = Some(agent.to_string());
            }
            entry.last_seen = Some(now);
        })
    }

    /// Note that `session_id` (belonging to `agent`) was seen in `handle`.
    ///
    /// Appends on first sight and refreshes `last_seen` thereafter, so the
    /// journal accumulates one entry per session rather than per observation.
    pub fn record_session(&self, handle: &str, agent: &str, session_id: &str) -> Result<()> {
        let now = now_secs();
        self.mutate(|state| {
            let entry = state
                .worktrees
                .entry(handle.to_string())
                .or_insert_with(|| WorktreeRecord {
                    branch: None,
                    parent: None,
                    agent: Some(agent.to_string()),
                    sessions: Vec::new(),
                    spawned_at: Some(now),
                    last_seen: None,
                });
            if !entry
                .sessions
                .iter()
                .any(|s| s.id == session_id && s.agent == agent)
            {
                entry.sessions.push(SessionRecord {
                    agent: agent.to_string(),
                    id: session_id.to_string(),
                    started_at: now,
                    ended_at: None,
                });
            }
            entry.agent = Some(agent.to_string());
            entry.last_seen = Some(now);
        })
    }

    /// Drop a worktree's record. Called when the worktree is removed, so the
    /// journal does not accumulate entries for worktrees that no longer exist.
    pub fn forget_worktree(&self, handle: &str) -> Result<()> {
        self.mutate(|state| {
            state.worktrees.remove(handle);
        })
    }

    /// Pretty-printed JSON of the whole document (for `show` / debugging).
    pub fn show(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.read()?)?)
    }
}

/// Build an `InProgress` slot owned by `owner` as of `now`.
fn in_progress(owner: &str, now: u64) -> Capability {
    Capability {
        status: CapabilityStatus::InProgress,
        owner: Some(owner.to_string()),
        started_at: Some(now),
        heartbeat: Some(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    fn test_store() -> (ProjectStateStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = ProjectStateStore::with_dir(dir.path().join("state")).unwrap();
        (store, dir)
    }

    #[test]
    fn worktree_journal_round_trips_and_preserves_sessions() {
        let (store, _d) = test_store();

        store
            .record_worktree("auth", Some("feat/auth"), Some("coordinator"), Some("claude"))
            .unwrap();
        store.record_session("auth", "claude", "sess-1").unwrap();

        // Re-recording (e.g. reopening the worktree) must not wipe history.
        store
            .record_worktree("auth", Some("feat/auth"), None, Some("claude"))
            .unwrap();

        let rec = store.get_worktree("auth").unwrap().unwrap();
        assert_eq!(rec.parent.as_deref(), Some("coordinator"));
        assert_eq!(rec.branch.as_deref(), Some("feat/auth"));
        assert_eq!(rec.sessions.len(), 1);
        assert_eq!(rec.latest_session_for("claude").unwrap().id, "sess-1");

        // Recording the same session twice appends nothing.
        store.record_session("auth", "claude", "sess-1").unwrap();
        assert_eq!(store.get_worktree("auth").unwrap().unwrap().sessions.len(), 1);

        // A different agent's session coexists rather than replacing.
        store.record_session("auth", "opencode", "sess-2").unwrap();
        let rec = store.get_worktree("auth").unwrap().unwrap();
        assert_eq!(rec.sessions.len(), 2);
        assert_eq!(rec.latest_session_for("claude").unwrap().id, "sess-1");

        store.forget_worktree("auth").unwrap();
        assert!(store.get_worktree("auth").unwrap().is_none());
    }

    #[test]
    fn worktree_journal_survives_files_written_before_it_existed() {
        let (store, _d) = test_store();
        // A pre-journal document has no `worktrees` key at all.
        std::fs::create_dir_all(store.json_path().parent().unwrap()).unwrap();
        std::fs::write(
            store.json_path(),
            r#"{"version":1,"capabilities":{},"facts":{"test_command":"cargo test"}}"#,
        )
        .unwrap();

        let state = store.read().unwrap();
        assert!(state.worktrees.is_empty());
        assert_eq!(state.facts.get("test_command").unwrap(), "cargo test");

        // And it can still be journalled into without losing the old facts.
        store.record_worktree("x", None, None, None).unwrap();
        assert_eq!(
            store.read().unwrap().facts.get("test_command").unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn missing_state_reads_as_default() {
        let (store, _dir) = test_store();
        let state = store.read().unwrap();
        assert!(state.capabilities.is_empty());
        assert!(state.facts.is_empty());
        assert_eq!(
            store.get_capability("build").unwrap().status,
            CapabilityStatus::Absent
        );
    }

    #[test]
    fn acquire_transitions_absent_to_in_progress() {
        let (store, _dir) = test_store();
        let outcome = store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        assert_eq!(outcome, AcquireOutcome::Acquired);

        let cap = store.get_capability("build").unwrap();
        assert_eq!(cap.status, CapabilityStatus::InProgress);
        assert_eq!(cap.owner.as_deref(), Some("actorA"));
        assert!(cap.started_at.is_some());
        assert!(cap.heartbeat.is_some());
    }

    #[test]
    fn second_actor_is_held_while_owner_is_fresh() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();

        let outcome = store
            .acquire("build", "actorB", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        assert_eq!(
            outcome,
            AcquireOutcome::Held {
                owner: Some("actorA".to_string())
            }
        );
        assert!(!outcome.owned());
    }

    #[test]
    fn owner_reacquire_is_idempotent() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        let again = store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        assert_eq!(again, AcquireOutcome::Acquired);
    }

    #[test]
    fn acquire_on_done_returns_already_done() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        store.complete("build", "actorA").unwrap();

        let outcome = store
            .acquire("build", "actorB", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        assert_eq!(outcome, AcquireOutcome::AlreadyDone);
    }

    #[test]
    fn stale_in_progress_is_reclaimed() {
        let (store, _dir) = test_store();
        // Plant an in_progress slot whose heartbeat is ancient.
        store
            .mutate(|state| {
                state.capabilities.insert(
                    "build".to_string(),
                    Capability {
                        status: CapabilityStatus::InProgress,
                        owner: Some("crashed".to_string()),
                        started_at: Some(1),
                        heartbeat: Some(1),
                    },
                );
            })
            .unwrap();

        // ttl of 60s: now - 1 is way past, so the slot is stale and reclaimable.
        let outcome = store.acquire("build", "rescuer", 60).unwrap();
        assert_eq!(
            outcome,
            AcquireOutcome::Reclaimed {
                previous_owner: Some("crashed".to_string())
            }
        );
        let cap = store.get_capability("build").unwrap();
        assert_eq!(cap.owner.as_deref(), Some("rescuer"));
        assert_eq!(cap.status, CapabilityStatus::InProgress);
    }

    #[test]
    fn fresh_in_progress_is_not_reclaimed() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        // Heartbeat is "now"; even a tiny ttl should still consider it fresh.
        let outcome = store
            .acquire("build", "actorB", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();
        assert!(matches!(outcome, AcquireOutcome::Held { .. }));
    }

    #[test]
    fn heartbeat_refreshes_for_owner_and_rejects_others() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();

        // Backdate the heartbeat, then refresh it as the owner.
        store
            .mutate(|state| {
                state.capabilities.get_mut("build").unwrap().heartbeat = Some(1);
            })
            .unwrap();
        assert!(store.heartbeat("build", "actorA").unwrap());
        assert!(store.get_capability("build").unwrap().heartbeat.unwrap() > 1);

        // A non-owner cannot heartbeat.
        assert!(!store.heartbeat("build", "actorB").unwrap());
    }

    #[test]
    fn complete_and_release_respect_ownership() {
        let (store, _dir) = test_store();
        store
            .acquire("build", "actorA", DEFAULT_CAPABILITY_TTL_SECS)
            .unwrap();

        // A different actor cannot complete a live in_progress slot.
        assert!(!store.complete("build", "actorB").unwrap());
        assert_eq!(
            store.get_capability("build").unwrap().status,
            CapabilityStatus::InProgress
        );

        // The owner can.
        assert!(store.complete("build", "actorA").unwrap());
        assert_eq!(
            store.get_capability("build").unwrap().status,
            CapabilityStatus::Done
        );

        // Release resets to absent (done slot is unowned-by-status check: owner matches).
        assert!(store.release("build", "actorA").unwrap());
        assert_eq!(
            store.get_capability("build").unwrap().status,
            CapabilityStatus::Absent
        );
    }

    #[test]
    fn facts_roundtrip() {
        let (store, _dir) = test_store();
        assert_eq!(store.get_fact("test_command").unwrap(), None);

        store.set_fact("test_command", "cargo test").unwrap();
        store.set_fact("build_command", "cargo build").unwrap();

        assert_eq!(
            store.get_fact("test_command").unwrap().as_deref(),
            Some("cargo test")
        );
        assert_eq!(
            store.get_fact("build_command").unwrap().as_deref(),
            Some("cargo build")
        );
    }

    #[test]
    fn facts_persist_across_store_handles() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path().join("state");
        ProjectStateStore::with_dir(state_dir.clone())
            .unwrap()
            .set_fact("test_command", "just test")
            .unwrap();

        let reopened = ProjectStateStore::with_dir(state_dir).unwrap();
        assert_eq!(
            reopened.get_fact("test_command").unwrap().as_deref(),
            Some("just test")
        );
    }

    /// The core invariant: under concurrent contention exactly one actor wins
    /// the Absent → InProgress compare-and-swap; everyone else is Held.
    #[test]
    fn concurrent_acquire_has_exactly_one_winner() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path().join("state");
        // Materialize the dir once so each thread's `with_dir` is a no-op create.
        ProjectStateStore::with_dir(state_dir.clone()).unwrap();

        let acquired = AtomicUsize::new(0);
        let held = AtomicUsize::new(0);

        std::thread::scope(|scope| {
            for i in 0..16 {
                let state_dir = state_dir.clone();
                let acquired = &acquired;
                let held = &held;
                scope.spawn(move || {
                    let store = ProjectStateStore::with_dir(state_dir).unwrap();
                    let owner = format!("actor{i}");
                    match store
                        .acquire("build", &owner, DEFAULT_CAPABILITY_TTL_SECS)
                        .unwrap()
                    {
                        AcquireOutcome::Acquired => {
                            acquired.fetch_add(1, Ordering::SeqCst);
                        }
                        AcquireOutcome::Held { .. } => {
                            held.fetch_add(1, Ordering::SeqCst);
                        }
                        other => panic!("unexpected outcome: {other:?}"),
                    }
                });
            }
        });

        assert_eq!(acquired.load(Ordering::SeqCst), 1, "exactly one winner");
        assert_eq!(held.load(Ordering::SeqCst), 15, "the rest are held");
    }

    #[test]
    fn show_emits_pretty_json() {
        let (store, _dir) = test_store();
        store.set_fact("test_command", "cargo test").unwrap();
        let json = store.show().unwrap();
        assert!(json.contains("test_command"));
        assert!(json.contains("\"facts\""));
    }
}

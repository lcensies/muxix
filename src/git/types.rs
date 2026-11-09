use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RemoteBranchSpec {
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct ForkBranchSpec {
    pub owner: String,
    pub branch: String,
}

/// Custom error type for worktree not found
#[derive(Debug, thiserror::Error)]
#[error("Worktree not found: {0}")]
pub struct WorktreeNotFound(pub String);

/// Git status information for a worktree
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
    /// Branch has conflicts when merging with base
    pub has_conflict: bool,
    /// Has uncommitted changes (staged or unstaged)
    pub is_dirty: bool,
    /// Lines added in committed changes only (base...HEAD)
    pub lines_added: usize,
    /// Lines removed in committed changes only (base...HEAD)
    pub lines_removed: usize,
    /// Lines added in uncommitted changes only (working tree + untracked)
    #[serde(default)]
    pub uncommitted_added: usize,
    /// Lines removed in uncommitted changes only (working tree)
    #[serde(default)]
    pub uncommitted_removed: usize,
    /// Timestamp when this status was cached (UNIX seconds)
    #[serde(default)]
    pub cached_at: Option<u64>,
    /// The base branch used for comparison (e.g., "main")
    #[serde(default)]
    pub base_branch: String,
    /// The branch name for this worktree (None if detached HEAD)
    #[serde(default)]
    pub branch: Option<String>,
    /// Whether the branch has an upstream tracking branch
    #[serde(default)]
    pub has_upstream: bool,
    /// Whether a rebase is currently in progress
    #[serde(default)]
    pub is_rebasing: bool,
    /// HEAD's object id when this status was computed. `None` for an empty
    /// repository, a detached HEAD, or an entry written before this field
    /// existed; a refresh with no HEAD oid cannot skip any work.
    #[serde(default)]
    pub head_oid: Option<String>,
    /// The base branch's object id when this status was computed.
    #[serde(default)]
    pub base_oid: Option<String>,
    /// Whether `base_branch` depended on the computing process's config --
    /// its `main_branch`, or the repository default it auto-detected because
    /// it had none. Only the branch's own `muxix-base` is independent of who
    /// computed the status, so only `false` makes a reply unconditionally ours.
    ///
    /// Missing from an entry written before this field existed, where the
    /// provenance is unknown; `true` then costs one re-resolve instead of
    /// granting that entry blanket trust.
    #[serde(default = "default_true")]
    pub base_from_config: bool,
}

fn default_true() -> bool {
    true
}

impl GitStatus {
    /// Would a process configured with `main_branch` accept the base branch
    /// this status was computed against?
    ///
    /// A base taken from the branch's own `muxix-base` is what every process
    /// resolves. A config-derived one is ours only if it is the `main_branch`
    /// we asked for -- or if we have none, in which case we would have taken
    /// the computing process's answer anyway. Numbers computed against another
    /// base must not be shown or reused as a cache.
    pub fn base_matches(&self, main_branch: Option<&str>) -> bool {
        !self.base_from_config
            || main_branch
                .filter(|s| !s.is_empty())
                .is_none_or(|m| m == self.base_branch)
    }
}

#[cfg(test)]
mod tests {
    use super::GitStatus;

    #[test]
    fn a_cache_entry_with_no_provenance_is_not_trusted() {
        let legacy: GitStatus =
            serde_json::from_str(r#"{"ahead":0,"behind":0,"has_conflict":false,"is_dirty":false,"lines_added":1,"lines_removed":0,"base_branch":"main"}"#)
                .unwrap();
        assert!(
            legacy.base_from_config,
            "an entry written before the field existed has unknown provenance"
        );
        assert!(
            !legacy.base_matches(Some("trunk")),
            "it must be re-resolved for a caller configured elsewhere"
        );
        assert!(legacy.base_matches(Some("main")));
    }

    #[test]
    fn an_auto_detected_base_is_not_ours_when_we_configured_another_one() {
        let status = GitStatus {
            base_branch: "master".to_string(),
            base_from_config: true,
            ..Default::default()
        };
        assert!(
            !status.base_matches(Some("trunk")),
            "we would resolve trunk; master is another process's answer"
        );
        assert!(
            status.base_matches(None),
            "with no main_branch of our own we would have detected the same"
        );

        let muxix_base = GitStatus {
            base_branch: "feature-a".to_string(),
            base_from_config: false,
            ..Default::default()
        };
        assert!(
            muxix_base.base_matches(Some("trunk")),
            "a muxix-base outranks every config"
        );
    }
}

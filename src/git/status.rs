use anyhow::Result;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

use crate::cmd::Cmd;

use super::GitStatus;
use super::branch::{get_branch_base_in, get_default_branch_in};

/// Create a git command that won't contend for index.lock.
/// Background monitoring should never block the user's git operations.
fn bg_git<'a>() -> Cmd<'a> {
    Cmd::new("git").arg("--no-optional-locks")
}

pub fn linked_worktree_admin_dir(worktree_path: &Path) -> Option<std::path::PathBuf> {
    let content = std::fs::read_to_string(worktree_path.join(".git")).ok()?;
    let raw = content.trim().strip_prefix("gitdir: ")?;
    let gitdir = Path::new(raw.trim());
    let gitdir = if gitdir.is_absolute() {
        gitdir.to_path_buf()
    } else {
        worktree_path.join(gitdir)
    };

    Some(gitdir)
}

pub fn has_missing_admin_dir(worktree_path: &Path) -> bool {
    linked_worktree_admin_dir(worktree_path).is_some_and(|gitdir| !gitdir.is_dir())
}

/// Check if the worktree has uncommitted changes
pub fn has_uncommitted_changes(worktree_path: &Path) -> Result<bool> {
    let output = bg_git()
        .workdir(worktree_path)
        .args(&["status", "--porcelain"])
        .run_and_capture_stdout()?;

    Ok(!output.is_empty())
}

/// Check if the worktree has tracked changes (staged or modified)
/// This excludes untracked files
pub fn has_tracked_changes(worktree_path: &Path) -> Result<bool> {
    let output = bg_git()
        .workdir(worktree_path)
        .args(&["status", "--porcelain"])
        .run_and_capture_stdout()?;

    // Filter out untracked files (lines starting with "??")
    for line in output.lines() {
        if !line.starts_with("??") && !line.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Check if the worktree has untracked files
pub fn has_untracked_files(worktree_path: &Path) -> Result<bool> {
    let output = bg_git()
        .workdir(worktree_path)
        .args(&["status", "--porcelain"])
        .run_and_capture_stdout()?;

    // Look for untracked files (lines starting with "??")
    for line in output.lines() {
        if line.starts_with("??") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Check if the worktree has staged changes
pub fn has_staged_changes(worktree_path: &Path) -> Result<bool> {
    // Exit code 0 = no changes, 1 = has changes
    // So we invert the result of run_as_check
    let no_changes = bg_git()
        .workdir(worktree_path)
        .args(&["diff", "--cached", "--quiet"])
        .run_as_check()?;
    Ok(!no_changes)
}

/// Check if the worktree has unstaged changes
pub fn has_unstaged_changes(worktree_path: &Path) -> Result<bool> {
    // Exit code 0 = no changes, 1 = has changes
    // So we invert the result of run_as_check
    let no_changes = bg_git()
        .workdir(worktree_path)
        .args(&["diff", "--quiet"])
        .run_as_check()?;
    Ok(!no_changes)
}

/// Branch info and dirty state read out of `git status --porcelain=v2 --branch`.
#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedStatus {
    /// `None` when HEAD is detached.
    branch: Option<String>,
    ahead: usize,
    behind: usize,
    is_dirty: bool,
    has_upstream: bool,
    /// `None` in a repository with no commits yet (`# branch.oid (initial)`).
    head_oid: Option<String>,
}

fn parse_porcelain_v2_status(output: &str) -> ParsedStatus {
    let mut branch_name: Option<String> = None;
    let mut ahead: usize = 0;
    let mut behind: usize = 0;
    let mut is_dirty = false;
    let mut has_upstream = false;
    let mut head_oid: Option<String> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            if rest != "(initial)" {
                head_oid = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            // "(detached)" indicates detached HEAD state
            if rest != "(detached)" {
                branch_name = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: "+<ahead> -<behind>"
            // This line only appears when branch has an upstream
            has_upstream = true;
            let mut parts = rest.split_whitespace();
            if let (Some(part_a), Some(part_b)) = (parts.next(), parts.next()) {
                if let Some(a) = part_a.strip_prefix('+') {
                    ahead = a.parse().unwrap_or(0);
                }
                if let Some(b) = part_b.strip_prefix('-') {
                    behind = b.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            // Any non-header, non-empty line indicates dirty state
            // This includes: '1' (ordinary), '2' (rename/copy), 'u' (unmerged), '?' (untracked)
            is_dirty = true;
            // Headers are always printed first in porcelain v2.
            // Once we find a file entry, we know the repo is dirty and can stop.
            break;
        }
    }

    ParsedStatus {
        branch: branch_name,
        ahead,
        behind,
        is_dirty,
        has_upstream,
        head_oid,
    }
}

/// Count lines in a file, treating it like git (text files only).
/// Returns 0 for binary files or errors.
fn count_lines(path: &Path) -> std::io::Result<usize> {
    use std::fs::File;

    let mut file = File::open(path)?;

    // Check for binary content (heuristic: null byte in first 8KB)
    let mut buffer = [0; 8192];
    let n = file.read(&mut buffer)?;
    if buffer[..n].contains(&0) {
        return Ok(0);
    }

    // Reset file position to start
    file.seek(SeekFrom::Start(0))?;

    let mut count = 0;
    let mut buf = [0; 32 * 1024];
    let mut last_byte = None;

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += buf[..n].iter().filter(|&&b| b == b'\n').count();
        last_byte = Some(buf[n - 1]);
    }

    // If file ends with non-newline character, count that as a line (like git)
    if let Some(b) = last_byte
        && b != b'\n'
    {
        count += 1;
    }

    Ok(count)
}

/// Parse `git diff --numstat` output into (added, removed). Binary files use
/// "-" instead of counts, which fails to parse and contributes nothing.
fn parse_numstat(output: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(a), Some(r)) = (parts.next(), parts.next()) {
            added += a.parse::<usize>().unwrap_or(0);
            removed += r.parse::<usize>().unwrap_or(0);
        }
    }
    (added, removed)
}

/// Lines added/removed by the commits on this branch (`<base>...HEAD`).
fn committed_diff(worktree_path: &Path, base_ref: &str) -> (usize, usize) {
    bg_git()
        .workdir(worktree_path)
        .args(&["diff", "--numstat", &format!("{}...HEAD", base_ref)])
        .run_and_capture_stdout()
        .map(|out| parse_numstat(&out))
        .unwrap_or((0, 0))
}

/// Lines added/removed in the working tree: staged and unstaged changes to
/// tracked files, plus every line of every untracked file.
fn uncommitted_diff(worktree_path: &Path) -> (usize, usize) {
    let (mut added, removed) = bg_git()
        .workdir(worktree_path)
        .args(&["diff", "--numstat", "HEAD"])
        .run_and_capture_stdout()
        .map(|out| parse_numstat(&out))
        .unwrap_or((0, 0));

    // Use -z so paths with spaces or newlines survive.
    if let Ok(output) = bg_git()
        .workdir(worktree_path)
        .args(&["ls-files", "--others", "--exclude-standard", "-z"])
        .run_and_capture_stdout()
    {
        for file_path in output.split('\0') {
            if file_path.is_empty() {
                continue;
            }

            let full_path = worktree_path.join(file_path);

            // Symlinks count as one line (the path), like git does.
            if let Ok(metadata) = std::fs::symlink_metadata(&full_path)
                && metadata.file_type().is_symlink()
            {
                added += 1;
                continue;
            }

            if let Ok(lines) = count_lines(&full_path) {
                added += lines;
            }
        }
    }

    (added, removed)
}

/// Check if a rebase is in progress by looking for rebase state directories in the git dir.
/// For linked worktrees, resolves the actual gitdir from the `.git` file.
fn is_rebasing(worktree_path: &Path) -> bool {
    let dot_git = worktree_path.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else if dot_git.is_file() {
        // Linked worktree: .git is a file containing "gitdir: /path/to/real/gitdir"
        let content = std::fs::read_to_string(&dot_git).unwrap_or_default();
        match content.strip_prefix("gitdir: ") {
            Some(gitdir) => {
                let path = std::path::PathBuf::from(gitdir.trim());
                if path.is_absolute() {
                    path
                } else {
                    worktree_path.join(path)
                }
            }
            None => return false,
        }
    } else {
        return false;
    };

    // Interactive rebase: rebase-merge/
    // Non-interactive rebase or git am: rebase-apply/
    git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir()
}

/// Whether `prev`'s base branch can be reused instead of asking git for it.
///
/// The base comes from `branch.<b>.muxix-base` in the repository's local
/// config, so any write to that file (`muxix set-base`, the dashboard's base
/// picker) forces a fresh lookup. Comparing the config's mtime against the
/// cached status's timestamp needs no extra field and no subprocess.
///
/// shortcut: ignores `config.worktree` (only written under
/// `extensions.worktreeConfig`, which muxix never sets); revisit if muxix
/// starts writing per-worktree config.
fn cached_base_is_current(common_dir: Option<&Path>, prev: &GitStatus, branch: &str) -> bool {
    if prev.base_branch.is_empty() {
        return false;
    }
    // The base is per-branch (`branch.<b>.muxix-base`), and a checkout rewrites
    // HEAD without touching the config file, so the mtime alone would carry the
    // old branch's base into the new one.
    if prev.branch.as_deref() != Some(branch) {
        return false;
    }
    let (Some(common_dir), Some(cached_at)) = (common_dir, prev.cached_at) else {
        return false;
    };
    let Ok(modified) = std::fs::metadata(common_dir.join("config")).and_then(|m| m.modified())
    else {
        return false;
    };
    let config_secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    // +1 covers the one-second granularity of `cached_at`.
    config_secs + 1 < cached_at
}

/// Get git status, reusing values from `prev` whose inputs provably have not
/// changed. A clean worktree whose HEAD and base have not moved costs exactly
/// one git process; see the `git-status-polling` spec.
pub fn get_git_status_cached(
    worktree_path: &Path,
    main_branch: Option<&str>,
    prev: Option<&GitStatus>,
) -> GitStatus {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .ok();

    let is_rebasing = is_rebasing(worktree_path);

    // Get branch info, ahead/behind, dirty state and HEAD's oid in one command
    let parsed = match bg_git()
        .workdir(worktree_path)
        .args(&["status", "--porcelain=v2", "--branch"])
        .run_and_capture_stdout()
    {
        Ok(output) => parse_porcelain_v2_status(&output),
        Err(_) => {
            return GitStatus {
                cached_at: now,
                branch: None,
                is_rebasing,
                ..Default::default()
            };
        }
    };
    let ParsedStatus {
        branch,
        ahead,
        behind,
        is_dirty,
        has_upstream,
        head_oid,
    } = parsed;

    // If no branch (detached HEAD or error), return early with dirty state
    let branch = match branch {
        Some(b) => b,
        None => {
            return GitStatus {
                is_dirty,
                cached_at: now,
                branch: None,
                has_upstream,
                is_rebasing,
                head_oid,
                ..Default::default()
            };
        }
    };

    let common_dir = crate::git::resolve_common_dir(worktree_path);

    // Determine base branch for conflict check and diff stats
    // Priority: cached (while the local config is older than the cache) >
    // muxix-base config > configured main_branch > auto-detected default > "main"
    let base_branch =
        match prev.filter(|p| cached_base_is_current(common_dir.as_deref(), p, &branch)) {
            Some(p) => p.base_branch.clone(),
            None => get_branch_base_in(&branch, Some(worktree_path))
                .ok()
                .or_else(|| main_branch.filter(|s| !s.is_empty()).map(|s| s.to_string()))
                .or_else(|| get_default_branch_in(Some(worktree_path)).ok())
                .unwrap_or_else(|| "main".to_string()),
        };

    let base_oid = common_dir
        .as_deref()
        .and_then(|dir| crate::git::read_branch_oid(dir, &base_branch));

    // A clean worktree has no working-tree diff and no untracked files, so
    // those two commands can only report zero.
    let (uncommitted_added, uncommitted_removed) = if is_dirty {
        uncommitted_diff(worktree_path)
    } else {
        (0, 0)
    };

    // On the base branch: no branch-level diff, but still show uncommitted changes
    if branch == base_branch {
        return GitStatus {
            ahead,
            behind,
            is_dirty,
            uncommitted_added,
            uncommitted_removed,
            cached_at: now,
            base_branch,
            branch: Some(branch),
            has_upstream,
            is_rebasing,
            head_oid,
            base_oid,
            ..Default::default()
        };
    }

    // The conflict check and the `<base>...HEAD` diff depend only on the pair
    // of commits being compared. Both unchanged means both answers stand.
    let unmoved = prev.filter(|p| {
        head_oid.is_some()
            && base_oid.is_some()
            && p.head_oid == head_oid
            && p.base_oid == base_oid
            && p.base_branch == base_branch
    });

    let (has_conflict, lines_added, lines_removed) = match unmoved {
        Some(p) => (p.has_conflict, p.lines_added, p.lines_removed),
        None => {
            // git merge-tree --write-tree returns exit code 1 on conflict (Git 2.38+)
            // Exit code 129 means unknown option (older Git) - treat as no conflict
            crate::cmd::count_git_spawn();
            let status = Command::new("git")
                .current_dir(worktree_path)
                .args([
                    "--no-optional-locks",
                    "merge-tree",
                    "--write-tree",
                    &base_branch,
                    "HEAD",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let has_conflict = matches!(status, Ok(s) if s.code() == Some(1));
            let (added, removed) = committed_diff(worktree_path, &base_branch);
            (has_conflict, added, removed)
        }
    };

    GitStatus {
        ahead,
        behind,
        has_conflict,
        is_dirty,
        lines_added,
        lines_removed,
        uncommitted_added,
        uncommitted_removed,
        cached_at: now,
        base_branch,
        branch: Some(branch),
        has_upstream,
        is_rebasing,
        head_oid,
        base_oid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_porcelain_v2_clean_repo() {
        let output = "# branch.oid abc123def456\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("main".to_string()));
        assert_eq!(got.ahead, 0);
        assert_eq!(got.behind, 0);
        assert!(!got.is_dirty);
        assert!(got.has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_dirty_repo() {
        let output = "# branch.oid abc123\n# branch.head feature\n# branch.upstream origin/feature\n# branch.ab +1 -2\n1 .M N... 100644 100644 100644 abc123 def456 src/file.rs\n? untracked.txt\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("feature".to_string()));
        assert_eq!(got.ahead, 1);
        assert_eq!(got.behind, 2);
        assert!(got.is_dirty);
        assert!(got.has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_no_upstream() {
        // When there's no upstream, branch.ab line is missing
        let output = "# branch.oid abc123\n# branch.head new-branch\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("new-branch".to_string()));
        assert_eq!(got.ahead, 0);
        assert_eq!(got.behind, 0);
        assert!(!got.is_dirty);
        assert!(!got.has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_detached_head() {
        let output = "# branch.oid abc123\n# branch.head (detached)\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, None);
        assert_eq!(got.ahead, 0);
        assert_eq!(got.behind, 0);
        assert!(!got.is_dirty);
        assert!(!got.has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_untracked_only() {
        let output = "# branch.oid abc123\n# branch.head main\n? untracked.txt\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("main".to_string()));
        assert!(got.is_dirty);
    }

    #[test]
    fn test_parse_porcelain_v2_renamed_file() {
        let output = "# branch.oid abc123\n# branch.head main\n2 R. N... 100644 100644 100644 abc123 def456 R100 old.rs\tnew.rs\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("main".to_string()));
        assert!(got.is_dirty);
    }

    #[test]
    fn test_parse_porcelain_v2_initial_commit() {
        // Repo created but no commits made yet
        let output = "# branch.oid (initial)\n# branch.head master\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("master".to_string()));
        assert_eq!(got.ahead, 0);
        assert_eq!(got.behind, 0);
        assert!(!got.is_dirty);
        assert!(!got.has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_unmerged_conflict() {
        // Merge conflict (unmerged entry starting with 'u')
        let output = "# branch.oid abc123\n# branch.head feature\n# branch.upstream origin/feature\n# branch.ab +0 -0\nu UU N... 100644 100644 100644 100644 abc def ghi jkl src/conflict.rs\n";
        let got = parse_porcelain_v2_status(output);
        assert_eq!(got.branch, Some("feature".to_string()));
        assert!(got.is_dirty);
        assert!(got.has_upstream);
    }
}

#[cfg(test)]
mod cached_refresh_tests {
    use super::*;
    use crate::cmd::take_git_spawns;
    use std::process::Command as StdCommand;

    fn run_git(dir: &Path, args: &[&str]) {
        let out = StdCommand::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repo on branch `feature` whose muxix-base is `main`, with one commit
    /// on each. The local config is backdated so a cached base branch counts
    /// as current (see `cached_base_is_current`).
    fn repo_on_feature(temp: &tempfile::TempDir) -> std::path::PathBuf {
        let dir = temp.path().join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "-b", "main"]);
        run_git(&dir, &["config", "user.email", "t@example.invalid"]);
        run_git(&dir, &["config", "user.name", "Test"]);
        std::fs::write(dir.join("README.md"), "one\n").unwrap();
        run_git(&dir, &["add", "."]);
        run_git(&dir, &["commit", "-m", "initial"]);
        run_git(&dir, &["checkout", "-b", "feature"]);
        std::fs::write(dir.join("feature.txt"), "a\nb\n").unwrap();
        run_git(&dir, &["add", "."]);
        run_git(&dir, &["commit", "-m", "feature"]);
        run_git(&dir, &["config", "branch.feature.muxix-base", "main"]);
        backdate_config(&dir);
        dir
    }

    fn backdate_config(dir: &Path) {
        filetime::set_file_mtime(
            dir.join(".git/config"),
            filetime::FileTime::from_unix_time(0, 0),
        )
        .unwrap();
    }

    #[test]
    fn clean_unmoved_worktree_costs_one_process() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);

        let first = get_git_status_cached(&dir, None, None);
        assert_eq!(first.branch.as_deref(), Some("feature"));
        assert_eq!(first.base_branch, "main");
        assert!(
            first.head_oid.is_some(),
            "HEAD oid comes from the status header"
        );
        assert!(first.base_oid.is_some(), "base oid comes from ref storage");
        assert!(!first.is_dirty);
        assert_eq!(first.lines_added, 2);

        take_git_spawns();
        let second = get_git_status_cached(&dir, None, Some(&first));
        assert_eq!(take_git_spawns(), 1);
        assert_eq!(second.lines_added, first.lines_added);
        assert_eq!(second.lines_removed, first.lines_removed);
        assert_eq!(second.has_conflict, first.has_conflict);
        assert_eq!(second.uncommitted_added, 0);
        assert_eq!(second.uncommitted_removed, 0);
        assert_eq!(second.head_oid, first.head_oid);
        assert_eq!(second.base_oid, first.base_oid);
    }

    #[test]
    fn dirty_unmoved_worktree_costs_three_processes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);

        std::fs::write(dir.join("feature.txt"), "a\nb\nc\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x\n").unwrap();

        take_git_spawns();
        let second = get_git_status_cached(&dir, None, Some(&first));
        // status + `diff --numstat HEAD` + `ls-files --others`
        assert_eq!(take_git_spawns(), 3);
        assert!(second.is_dirty);
        assert_eq!(
            second.uncommitted_added, 2,
            "one tracked line + one untracked"
        );
        assert_eq!(second.lines_added, first.lines_added, "carried over");
    }

    #[test]
    fn moved_head_recomputes_the_committed_diff() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);

        std::fs::write(dir.join("feature.txt"), "a\nb\nc\nd\n").unwrap();
        run_git(&dir, &["add", "."]);
        run_git(&dir, &["commit", "-m", "more"]);
        backdate_config(&dir);

        take_git_spawns();
        let second = get_git_status_cached(&dir, None, Some(&first));
        // status + merge-tree + `<base>...HEAD` diff; clean, so nothing else
        assert_eq!(take_git_spawns(), 3);
        assert_ne!(second.head_oid, first.head_oid);
        assert_eq!(second.lines_added, 4);
    }

    #[test]
    fn moved_base_recomputes_the_committed_diff() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);

        // Advance main without touching feature.
        run_git(&dir, &["checkout", "main"]);
        std::fs::write(dir.join("README.md"), "one\ntwo\n").unwrap();
        run_git(&dir, &["commit", "-am", "main moves"]);
        run_git(&dir, &["checkout", "feature"]);
        backdate_config(&dir);

        take_git_spawns();
        let second = get_git_status_cached(&dir, None, Some(&first));
        assert_eq!(take_git_spawns(), 3);
        assert_eq!(second.head_oid, first.head_oid);
        assert_ne!(second.base_oid, first.base_oid, "the base moved");
    }

    #[test]
    fn a_fingerprintless_cache_entry_runs_every_step() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);

        // What an entry written before this change deserializes to.
        let legacy = GitStatus {
            head_oid: None,
            base_oid: None,
            lines_added: 999,
            ..first.clone()
        };

        take_git_spawns();
        let refreshed = get_git_status_cached(&dir, None, Some(&legacy));
        assert_eq!(take_git_spawns(), 3);
        assert_eq!(refreshed.lines_added, first.lines_added);
    }

    /// The dashboard's refresh feeds `prev` from `git_status_cache.json`, so
    /// the fingerprint has to survive a save/load round-trip — a dropped or
    /// rewritten field would make the skip silently never fire.
    #[test]
    fn a_cache_entry_from_disk_still_costs_one_process() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);

        let cache_file = temp.path().join("git_status_cache.json");
        let mut cache = std::collections::HashMap::new();
        cache.insert(dir.clone(), first.clone());
        crate::git::cache::save_status_cache_at(&cache_file, &cache);
        let loaded = crate::git::cache::load_status_cache_at(&cache_file);
        assert_eq!(loaded.get(&dir), Some(&first), "entry round-trips intact");

        take_git_spawns();
        let second = get_git_status_cached(&dir, None, loaded.get(&dir));
        assert_eq!(take_git_spawns(), 1);
        assert_eq!(second.lines_added, first.lines_added);
        assert_eq!(second.head_oid, first.head_oid);
        assert_eq!(second.base_oid, first.base_oid);
    }

    #[test]
    fn a_rewritten_base_config_forces_a_fresh_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);
        assert_eq!(first.base_branch, "main");

        // `muxix set-base` writes the local config; its mtime is now newer
        // than the cached status, so the base must be re-read.
        run_git(&dir, &["branch", "other", "main"]);
        run_git(&dir, &["config", "branch.feature.muxix-base", "other"]);

        let second = get_git_status_cached(&dir, None, Some(&first));
        assert_eq!(second.base_branch, "other");
    }

    #[test]
    fn a_branch_switch_forces_a_fresh_base_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let first = get_git_status_cached(&dir, None, None);
        assert_eq!(first.base_branch, "main");

        // A second branch with its own base, recorded while the config is
        // still backdated: a checkout rewrites HEAD only, so the mtime guard
        // cannot see the switch.
        run_git(&dir, &["branch", "release", "main"]);
        run_git(&dir, &["checkout", "-b", "release-x", "main"]);
        run_git(&dir, &["config", "branch.release-x.muxix-base", "release"]);
        backdate_config(&dir);

        let second = get_git_status_cached(&dir, None, Some(&first));
        assert_eq!(second.branch.as_deref(), Some("release-x"));
        assert_eq!(
            second.base_branch, "release",
            "the base is per-branch and must be re-read after a checkout"
        );
    }

    #[test]
    fn detached_head_stops_after_the_status_call() {
        let temp = tempfile::tempdir().unwrap();
        let dir = repo_on_feature(&temp);
        let head = StdCommand::new("git")
            .current_dir(&dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
        run_git(&dir, &["checkout", "--detach", &head]);

        take_git_spawns();
        let status = get_git_status_cached(&dir, None, None);
        assert_eq!(take_git_spawns(), 1);
        assert_eq!(status.branch, None);
        assert_eq!(status.head_oid.as_deref(), Some(head.as_str()));
    }
}

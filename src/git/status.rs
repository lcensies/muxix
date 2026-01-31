use anyhow::Result;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

use crate::cmd::Cmd;

use super::GitStatus;
use super::branch::{get_branch_base_in, get_default_branch_in};

/// Check if the worktree has uncommitted changes
pub fn has_uncommitted_changes(worktree_path: &Path) -> Result<bool> {
    let output = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["status", "--porcelain"])
        .run_and_capture_stdout()?;

    Ok(!output.is_empty())
}

/// Check if the worktree has tracked changes (staged or modified)
/// This excludes untracked files
pub fn has_tracked_changes(worktree_path: &Path) -> Result<bool> {
    let output = Cmd::new("git")
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
    let output = Cmd::new("git")
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
    let no_changes = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["diff", "--cached", "--quiet"])
        .run_as_check()?;
    Ok(!no_changes)
}

/// Check if the worktree has unstaged changes
pub fn has_unstaged_changes(worktree_path: &Path) -> Result<bool> {
    // Exit code 0 = no changes, 1 = has changes
    // So we invert the result of run_as_check
    let no_changes = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["diff", "--quiet"])
        .run_as_check()?;
    Ok(!no_changes)
}

/// Parse git status porcelain v2 output to extract branch info and dirty state.
/// Returns (branch_name, ahead, behind, is_dirty, has_upstream).
fn parse_porcelain_v2_status(output: &str) -> (Option<String>, usize, usize, bool, bool) {
    let mut branch_name: Option<String> = None;
    let mut ahead: usize = 0;
    let mut behind: usize = 0;
    let mut is_dirty = false;
    let mut has_upstream = false;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
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

    (branch_name, ahead, behind, is_dirty, has_upstream)
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

/// Diff statistics returned by get_diff_stats.
///
/// Separates committed and uncommitted changes:
/// - Committed: changes in base...HEAD (what's been committed on the branch)
/// - Uncommitted: working tree changes + untracked files
struct DiffStats {
    /// Lines added in committed changes only (base...HEAD)
    committed_added: usize,
    /// Lines removed in committed changes only (base...HEAD)
    committed_removed: usize,
    /// Lines added in uncommitted changes (working tree + untracked)
    uncommitted_added: usize,
    /// Lines removed in uncommitted changes (working tree)
    uncommitted_removed: usize,
}

fn get_diff_stats(worktree_path: &Path, base_ref: &str) -> DiffStats {
    let mut committed_added = 0;
    let mut committed_removed = 0;
    let mut uncommitted_added = 0;
    let mut uncommitted_removed = 0;

    // Helper to parse numstat output
    let parse_numstat = |output: &str| -> (usize, usize) {
        let mut a = 0;
        let mut r = 0;
        for line in output.lines() {
            let mut parts = line.split_whitespace();
            // Format: <added> <removed> <filename>
            // Binary files use "-" instead of numbers (parse will fail, which is fine)
            if let (Some(added), Some(removed)) = (parts.next(), parts.next()) {
                a += added.parse::<usize>().unwrap_or(0);
                r += removed.parse::<usize>().unwrap_or(0);
            }
        }
        (a, r)
    };

    // 1. Committed changes (base...HEAD)
    if let Ok(output) = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["diff", "--numstat", &format!("{}...HEAD", base_ref)])
        .run_and_capture_stdout()
    {
        let (a, r) = parse_numstat(&output);
        committed_added += a;
        committed_removed += r;
    }

    // 2. Uncommitted changes (HEAD vs working tree)
    // This covers both staged and unstaged changes to tracked files
    if let Ok(output) = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["diff", "--numstat", "HEAD"])
        .run_and_capture_stdout()
    {
        let (a, r) = parse_numstat(&output);
        uncommitted_added += a;
        uncommitted_removed += r;
    }

    // 3. Untracked files (all lines count as added to uncommitted)
    // Use -z to separate paths with null bytes, handling spaces/special chars correctly
    if let Ok(output) = Cmd::new("git")
        .workdir(worktree_path)
        .args(&["ls-files", "--others", "--exclude-standard", "-z"])
        .run_and_capture_stdout()
    {
        for file_path in output.split('\0') {
            if file_path.is_empty() {
                continue;
            }

            let full_path = worktree_path.join(file_path);

            // Check for symlinks - treat as 1 line (the path) like git does
            if let Ok(metadata) = std::fs::symlink_metadata(&full_path)
                && metadata.file_type().is_symlink()
            {
                uncommitted_added += 1;
                continue;
            }

            if let Ok(lines) = count_lines(&full_path) {
                uncommitted_added += lines;
            }
        }
    }

    DiffStats {
        committed_added,
        committed_removed,
        uncommitted_added,
        uncommitted_removed,
    }
}

/// Get git status for a worktree (ahead/behind, conflicts, dirty state, diff stats).
/// This is designed for dashboard display and prioritizes speed over completeness.
/// Uses `git status --porcelain=v2 --branch` to get most info in a single command.
pub fn get_git_status(worktree_path: &Path) -> GitStatus {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .ok();

    // Get branch info, ahead/behind, and dirty state in one command
    let (branch, ahead, behind, is_dirty, has_upstream) = match Cmd::new("git")
        .workdir(worktree_path)
        .args(&["status", "--porcelain=v2", "--branch"])
        .run_and_capture_stdout()
    {
        Ok(output) => parse_porcelain_v2_status(&output),
        Err(_) => {
            return GitStatus {
                cached_at: now,
                branch: None,
                ..Default::default()
            };
        }
    };

    // If no branch (detached HEAD or error), return early with dirty state
    let branch = match branch {
        Some(b) => b,
        None => {
            return GitStatus {
                is_dirty,
                cached_at: now,
                branch: None,
                has_upstream,
                ..Default::default()
            };
        }
    };

    // Determine base branch for conflict check and diff stats
    // First try workmux-base config, then fall back to default branch
    let base_branch = get_branch_base_in(&branch, Some(worktree_path))
        .ok()
        .or_else(|| get_default_branch_in(Some(worktree_path)).ok())
        .unwrap_or_else(|| "main".to_string());

    // On the base branch: no branch-level diff, but still show uncommitted changes
    if branch == base_branch {
        let stats = get_diff_stats(worktree_path, &branch);

        return GitStatus {
            ahead,
            behind,
            is_dirty,
            uncommitted_added: stats.uncommitted_added,
            uncommitted_removed: stats.uncommitted_removed,
            cached_at: now,
            base_branch,
            branch: Some(branch),
            has_upstream,
            ..Default::default()
        };
    }

    // Use local base branch for comparisons (clone since we need it in the return)
    let base_ref = base_branch.clone();

    // Check for merge conflicts with base branch
    // git merge-tree --write-tree returns exit code 1 on conflict (Git 2.38+)
    // Exit code 129 means unknown option (older Git) - treat as no conflict
    let has_conflict = {
        let status = Command::new("git")
            .current_dir(worktree_path)
            .args(["merge-tree", "--write-tree", &base_ref, "HEAD"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(status, Ok(s) if s.code() == Some(1))
    };

    // Get diff stats (lines added/removed vs base)
    let diff_stats = get_diff_stats(worktree_path, &base_ref);

    GitStatus {
        ahead,
        behind,
        has_conflict,
        is_dirty,
        lines_added: diff_stats.committed_added,
        lines_removed: diff_stats.committed_removed,
        uncommitted_added: diff_stats.uncommitted_added,
        uncommitted_removed: diff_stats.uncommitted_removed,
        cached_at: now,
        base_branch,
        branch: Some(branch),
        has_upstream,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_porcelain_v2_status;

    #[test]
    fn test_parse_porcelain_v2_clean_repo() {
        let output = "# branch.oid abc123def456\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n";
        let (branch, ahead, behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("main".to_string()));
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(!is_dirty);
        assert!(has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_dirty_repo() {
        let output = "# branch.oid abc123\n# branch.head feature\n# branch.upstream origin/feature\n# branch.ab +1 -2\n1 .M N... 100644 100644 100644 abc123 def456 src/file.rs\n? untracked.txt\n";
        let (branch, ahead, behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("feature".to_string()));
        assert_eq!(ahead, 1);
        assert_eq!(behind, 2);
        assert!(is_dirty);
        assert!(has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_no_upstream() {
        // When there's no upstream, branch.ab line is missing
        let output = "# branch.oid abc123\n# branch.head new-branch\n";
        let (branch, ahead, behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("new-branch".to_string()));
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(!is_dirty);
        assert!(!has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_detached_head() {
        let output = "# branch.oid abc123\n# branch.head (detached)\n";
        let (branch, ahead, behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, None);
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(!is_dirty);
        assert!(!has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_untracked_only() {
        let output = "# branch.oid abc123\n# branch.head main\n? untracked.txt\n";
        let (branch, _ahead, _behind, is_dirty, _has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("main".to_string()));
        assert!(is_dirty);
    }

    #[test]
    fn test_parse_porcelain_v2_renamed_file() {
        let output = "# branch.oid abc123\n# branch.head main\n2 R. N... 100644 100644 100644 abc123 def456 R100 old.rs\tnew.rs\n";
        let (branch, _ahead, _behind, is_dirty, _has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("main".to_string()));
        assert!(is_dirty);
    }

    #[test]
    fn test_parse_porcelain_v2_initial_commit() {
        // Repo created but no commits made yet
        let output = "# branch.oid (initial)\n# branch.head master\n";
        let (branch, ahead, behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("master".to_string()));
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(!is_dirty);
        assert!(!has_upstream);
    }

    #[test]
    fn test_parse_porcelain_v2_unmerged_conflict() {
        // Merge conflict (unmerged entry starting with 'u')
        let output = "# branch.oid abc123\n# branch.head feature\n# branch.upstream origin/feature\n# branch.ab +0 -0\nu UU N... 100644 100644 100644 100644 abc def ghi jkl src/conflict.rs\n";
        let (branch, _ahead, _behind, is_dirty, has_upstream) = parse_porcelain_v2_status(output);
        assert_eq!(branch, Some("feature".to_string()));
        assert!(is_dirty);
        assert!(has_upstream);
    }
}

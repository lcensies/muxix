//! Shared helper functions for agent display name extraction.
//!
//! These are used by both the dashboard and sidebar to derive human-readable
//! names from agent pane metadata.

use std::path::Path;

/// Extract the worktree name from a window or session name.
/// Checks window_name first (window mode), then session_name (session mode).
/// Returns (worktree_name, is_main) where is_main indicates if this is the main worktree.
pub fn extract_worktree_name(
    session_name: &str,
    window_name: &str,
    window_prefix: &str,
    path: &Path,
) -> (String, bool) {
    if let Some(stripped) = window_name.strip_prefix(window_prefix) {
        // Window mode: worktree name is in the window name
        (stripped.to_string(), false)
    } else if let Some(stripped) = session_name.strip_prefix(window_prefix) {
        // Session mode: worktree name is in the session name
        (stripped.to_string(), false)
    } else if !is_generic_mux_name(window_name) {
        let is_main = matches!(window_name, "main" | "master");
        (window_name.to_string(), is_main)
    } else if !is_generic_mux_name(session_name) {
        let is_main = matches!(session_name, "main" | "master");
        (session_name.to_string(), is_main)
    } else {
        derive_worktree_name_from_path(path)
    }
}

fn is_generic_mux_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty()
        || trimmed.chars().all(|c| c.is_ascii_digit())
        || matches!(trimmed, "zsh" | "bash" | "sh" | "fish" | "nu" | "nushell")
}

fn derive_worktree_name_from_path(path: &Path) -> (String, bool) {
    let project = extract_project_name(path);
    let basename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    if basename == project {
        ("main".to_string(), true)
    } else {
        (basename, false)
    }
}

/// Extract project name from a worktree path.
/// Finds the git root (where .git is a directory) or falls back to pattern matching.
pub fn extract_project_name(path: &Path) -> String {
    // Walk up the path to find the git root or worktrees pattern
    for ancestor in path.ancestors() {
        // Check if this is the git root (where .git is a directory, not a file)
        let git_path = ancestor.join(".git");
        if git_path.is_dir()
            && let Some(name) = ancestor.file_name()
        {
            return name.to_string_lossy().to_string();
        }

        // Fallback: check for sibling pattern (project__worktrees/)
        if let Some(name) = ancestor.file_name() {
            let name_str = name.to_string_lossy();
            if name_str.ends_with("__worktrees") {
                return name_str
                    .strip_suffix("__worktrees")
                    .unwrap_or(&name_str)
                    .to_string();
            }
        }
    }

    // Fallback: use the directory name (for non-worktree projects)
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_worktree_name_window_mode() {
        let path = Path::new("/home/user/myproject__worktrees/fix-bug");
        let (name, is_main) =
            extract_worktree_name("main-session", "workmux:fix-bug", "workmux:", path);
        assert_eq!(name, "fix-bug");
        assert!(!is_main);
    }

    #[test]
    fn test_extract_worktree_name_session_mode() {
        let path = Path::new("/home/user/myproject__worktrees/feature-auth");
        let (name, is_main) =
            extract_worktree_name("workmux:feature-auth", "zsh", "workmux:", path);
        assert_eq!(name, "feature-auth");
        assert!(!is_main);
    }

    #[test]
    fn test_extract_worktree_name_window_preferred_over_session() {
        let path = Path::new("/home/user/myproject__worktrees/from-window");
        let (name, is_main) = extract_worktree_name(
            "workmux:from-session",
            "workmux:from-window",
            "workmux:",
            path,
        );
        assert_eq!(name, "from-window");
        assert!(!is_main);
    }

    #[test]
    fn test_extract_worktree_name_plain_window_name() {
        let path = Path::new("/home/user/myproject__worktrees/reforge");
        let (name, is_main) = extract_worktree_name("0", "reforge", "workmux:", path);
        assert_eq!(name, "reforge");
        assert!(!is_main);
    }

    #[test]
    fn test_extract_worktree_name_path_fallback_for_generic_names() {
        let path = Path::new("/home/user/myproject__worktrees/fix-bug");
        let (name, is_main) = extract_worktree_name("0", "zsh", "workmux:", path);
        assert_eq!(name, "fix-bug");
        assert!(!is_main);
    }

    #[test]
    fn test_extract_worktree_name_main_from_path_fallback() {
        let path = Path::new("/home/user/myproject");
        let (name, is_main) = extract_worktree_name("0", "zsh", "workmux:", path);
        assert_eq!(name, "main");
        assert!(is_main);
    }

    #[test]
    fn test_extract_project_name_worktrees() {
        let path = PathBuf::from("/home/user/myproject__worktrees/fix-bug");
        assert_eq!(extract_project_name(&path), "myproject");
    }

    #[test]
    fn test_extract_project_name_fallback() {
        let path = PathBuf::from("/home/user/myproject");
        assert_eq!(extract_project_name(&path), "myproject");
    }

    #[test]
    fn test_extract_project_name_git_root() {
        // Test custom worktree_dir inside repo (e.g., .worktrees)
        let temp = tempfile::TempDir::new().unwrap();
        let project_dir = temp.path().join("myproject");
        std::fs::create_dir_all(project_dir.join(".git")).unwrap();
        std::fs::create_dir_all(project_dir.join(".worktrees").join("fix-bug")).unwrap();

        let worktree_path = project_dir.join(".worktrees").join("fix-bug");
        assert_eq!(extract_project_name(&worktree_path), "myproject");
    }

    #[test]
    fn test_extract_project_name_git_file_skipped() {
        // Worktrees have .git as a file, not directory - should be skipped
        let temp = tempfile::TempDir::new().unwrap();
        let project_dir = temp.path().join("myproject");
        let worktree_dir = project_dir.join(".worktrees").join("fix-bug");
        std::fs::create_dir_all(&worktree_dir).unwrap();
        // Create .git as a file (like real worktrees do)
        std::fs::write(worktree_dir.join(".git"), "gitdir: /somewhere/else").unwrap();
        // Create actual git root
        std::fs::create_dir_all(project_dir.join(".git")).unwrap();

        assert_eq!(extract_project_name(&worktree_dir), "myproject");
    }
}

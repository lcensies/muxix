use anyhow::{Result, bail};
use slug::slugify;

/// Derives the "handle" (worktree dir name + tmux window base name)
/// from the branch name and an optional explicit override.
///
/// The handle is always slugified to ensure filesystem/tmux compatibility.
///
/// Priority:
/// 1. Explicit name (--name flag) - bypasses all config
/// 2. Branch name as-is (default)
///
/// Future versions will add config-based strategies (basename, prefix) here.
pub fn derive_handle(branch_name: &str, explicit_name: Option<&str>) -> Result<String> {
    let handle = if let Some(name) = explicit_name {
        // Explicit --name takes priority and bypasses any future prefix config
        slugify(name)
    } else {
        // Default: slugify the branch name
        slugify(branch_name)
    };

    validate_handle(&handle)?;
    Ok(handle)
}

/// Validates that a handle is safe for filesystem and tmux use.
fn validate_handle(handle: &str) -> Result<()> {
    if handle.is_empty() {
        bail!("Handle cannot be empty");
    }

    // Slugify should have removed these, but double check for safety
    if handle.contains("..") || handle.starts_with('/') {
        bail!("Handle cannot contain path traversal");
    }

    if handle.chars().any(char::is_whitespace) {
        bail!("Handle cannot contain whitespace");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_handle_explicit_name() {
        let result = derive_handle("prj-4120/feature", Some("cool-feature")).unwrap();
        assert_eq!(result, "cool-feature");
    }

    #[test]
    fn derive_handle_explicit_name_with_spaces() {
        let result = derive_handle("branch", Some("My Cool Feature")).unwrap();
        assert_eq!(result, "my-cool-feature");
    }

    #[test]
    fn derive_handle_explicit_name_with_special_chars() {
        let result = derive_handle("branch", Some("Feature! @#$%")).unwrap();
        assert_eq!(result, "feature");
    }

    #[test]
    fn derive_handle_branch_name_slugified() {
        let result = derive_handle("prj-4120/create-new-tags", None).unwrap();
        assert_eq!(result, "prj-4120-create-new-tags");
    }

    #[test]
    fn derive_handle_simple_branch() {
        let result = derive_handle("main", None).unwrap();
        assert_eq!(result, "main");
    }

    #[test]
    fn derive_handle_nested_branch() {
        let result = derive_handle("feature/auth/oauth", None).unwrap();
        assert_eq!(result, "feature-auth-oauth");
    }

    #[test]
    fn derive_handle_empty_explicit_name_fails() {
        let result = derive_handle("branch", Some(""));
        assert!(result.is_err());
    }

    #[test]
    fn validate_handle_empty_fails() {
        let result = validate_handle("");
        assert!(result.is_err());
    }

    #[test]
    fn validate_handle_valid() {
        let result = validate_handle("my-feature");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_handle_with_numbers() {
        let result = validate_handle("feature-123");
        assert!(result.is_ok());
    }
}

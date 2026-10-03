//! Shared helpers for plugin spec probes (pi, omp, ...).
//!
//! A declared plugin "spec" is either a scheme-prefixed opaque identifier
//! (`npm:x`, `git:host/x`, `https://...`) or a filesystem path (`./vendor/x`,
//! `../x`, `/abs/x`). Only the latter needs resolving before comparison — pi
//! and omp record path specs relative to their own config dir, so a
//! project-relative declaration never matches the recorded string verbatim.

use std::path::{Path, PathBuf};

/// Whether `spec` is a filesystem path rather than a scheme-prefixed opaque
/// identifier (`npm:`, `git:`, `https:`, ...).
///
/// A spec is scheme-shaped when it has a `:` before its first `/` (or
/// contains no `/` at all, e.g. `npm:x`). Anything else — `./x`, `../x`,
/// `/abs/x`, a bare relative segment — is path-shaped: exactly the set that
/// exact-string comparison gets wrong today.
pub fn is_path_shaped(spec: &str) -> bool {
    let scheme_part = spec.split('/').next().unwrap_or(spec);
    !scheme_part.contains(':')
}

/// Canonicalize `spec` against `base` if it is path-shaped.
///
/// Returns `None` for scheme-shaped specs (compare those as exact strings
/// instead) and for path-shaped specs that don't resolve to an existing
/// filesystem entry. `fs::canonicalize` failure (missing path, dangling
/// symlink, ...) means "not installed" — the safe default, since it degrades
/// to re-running the installer rather than silently matching.
pub fn canonicalize_path_spec(spec: &str, base: &Path) -> Option<PathBuf> {
    if !is_path_shaped(spec) {
        return None;
    }
    base.join(spec).canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scheme_shaped_specs_are_not_path_shaped() {
        assert!(!is_path_shaped("npm:pi-subagents"));
        assert!(!is_path_shaped("git:github.com/x/y"));
        assert!(!is_path_shaped("https://example.com/x"));
    }

    #[test]
    fn path_shaped_specs_are_detected() {
        assert!(is_path_shaped("./vendor/x"));
        assert!(is_path_shaped("../vendor/x"));
        assert!(is_path_shaped("/abs/vendor/x"));
        assert!(is_path_shaped("vendor/x"));
    }

    #[test]
    fn scheme_shaped_specs_never_canonicalize() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(canonicalize_path_spec("npm:x", tmp.path()), None);
    }

    #[test]
    fn dot_relative_resolves_against_base() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("x");
        fs::create_dir(&target).unwrap();

        let resolved = canonicalize_path_spec("./x", tmp.path()).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[test]
    fn parent_relative_resolves_against_base() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let target = tmp.path().join("x");
        fs::create_dir(&target).unwrap();

        // base is the subdir; "../x" should escape it back to tmp/x.
        let resolved = canonicalize_path_spec("../x", &sub).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[test]
    fn absolute_spec_ignores_base() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("x");
        fs::create_dir(&target).unwrap();
        let unrelated_base = tmp.path().join("sub");
        fs::create_dir(&unrelated_base).unwrap();

        let spec = target.to_str().unwrap();
        let resolved = canonicalize_path_spec(spec, &unrelated_base).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_path_resolves_to_real_target() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real");
        fs::create_dir(&target).unwrap();
        let link = tmp.path().join("link");
        symlink(&target, &link).unwrap();

        let resolved = canonicalize_path_spec("./link", tmp.path()).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[test]
    fn nonexistent_path_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(canonicalize_path_spec("./nope", tmp.path()), None);
    }
}

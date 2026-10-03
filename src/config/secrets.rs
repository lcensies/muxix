//! Deferred secret interpolation: `${env:VAR}` and `${file:/path}`.
//!
//! Config values may name a secret rather than contain one. These placeholders
//! are resolved at the point of *use*, never during config resolution, so a
//! resolved config — the thing `workmux config resolve` prints, the thing a Nix
//! module renders into the store — never holds secret material.
//!
//! Workmux is deliberately not a secret manager. This exists so that
//! sops-nix, agenix, a password manager, or a plain mode-600 file can be the
//! source of truth without the secret having to pass through a config file.

use anyhow::{Context as _, Result, bail};
use std::path::Path;

/// Whether a value contains a placeholder that [`expand`] would act on.
pub fn has_placeholder(value: &str) -> bool {
    value.contains("${env:") || value.contains("${file:")
}

/// Expand every `${env:VAR}` and `${file:/path}` placeholder in `value`.
///
/// `key` names the config key being resolved and appears in errors — without
/// it, "environment variable not set" gives the user nothing to act on.
///
/// A `${file:...}` source must not be readable by group or other: a token that
/// any local process can read is not a secret, and silently accepting one would
/// defeat the point of referencing a file instead of inlining the value.
pub fn expand(value: &str, key: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            // An unterminated `${` is literal text, not a broken placeholder.
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let body = &after[..end];
        let remainder = &after[end + 1..];

        match body.split_once(':') {
            Some(("env", name)) => out.push_str(&read_env(name, key)?),
            Some(("file", path)) => out.push_str(&read_file(Path::new(path), key)?),
            // Anything else is not ours; pass it through untouched so a value
            // like `${HOME}` still means whatever it meant before.
            _ => {
                out.push_str("${");
                out.push_str(body);
                out.push('}');
            }
        }
        rest = remainder;
    }

    out.push_str(rest);
    Ok(out)
}

fn read_env(name: &str, key: &str) -> Result<String> {
    let name = name.trim();
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) => bail!("{key}: environment variable {name} is set but empty"),
        Err(_) => bail!("{key}: environment variable {name} is not set"),
    }
}

fn read_file(path: &Path, key: &str) -> Result<String> {
    if !path.exists() {
        bail!("{key}: file {} does not exist", path.display());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("{key}: cannot stat {}", path.display()))?;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "{key}: file {} is readable by group or others (mode {:o}); chmod 600 it",
                path.display(),
                mode & 0o777
            );
        }
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("{key}: cannot read {}", path.display()))?;
    let contents = contents.trim().to_string();
    if contents.is_empty() {
        bail!("{key}: file {} is empty", path.display());
    }
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env vars are process-global; the tests that set them serialize here.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Set several env vars for the duration of `f`, restoring them after.
    ///
    /// Takes every variable at once rather than nesting: the lock is not
    /// reentrant, so a nested call would deadlock.
    fn with_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(name, _)| ((*name).to_string(), std::env::var(name).ok()))
            .collect();
        unsafe {
            for (name, value) in vars {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
        let out = f();
        unsafe {
            for (name, value) in &saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
        out
    }

    fn with_var<T>(name: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        with_vars(&[(name, value)], f)
    }

    fn tmpfile(name: &str, body: &str, mode: u32) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "workmux-secret-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        path
    }

    #[test]
    fn detects_placeholders() {
        assert!(has_placeholder("${env:TOKEN}"));
        assert!(has_placeholder("prefix ${file:/x} suffix"));
        assert!(!has_placeholder("plain value"));
        assert!(!has_placeholder("${HOME}"));
    }

    #[test]
    fn expands_an_env_placeholder() {
        with_var("WORKMUX_TEST_SECRET", Some("s3cret"), || {
            assert_eq!(
                expand("${env:WORKMUX_TEST_SECRET}", "provision.token").unwrap(),
                "s3cret"
            );
        });
    }

    #[test]
    fn expands_within_surrounding_text() {
        with_var("WORKMUX_TEST_SECRET", Some("mid"), || {
            assert_eq!(
                expand("a-${env:WORKMUX_TEST_SECRET}-b", "k").unwrap(),
                "a-mid-b"
            );
        });
    }

    #[test]
    fn expands_several_placeholders() {
        with_vars(
            &[
                ("WORKMUX_TEST_A", Some("1")),
                ("WORKMUX_TEST_B", Some("2")),
            ],
            || {
                assert_eq!(
                    expand("${env:WORKMUX_TEST_A}/${env:WORKMUX_TEST_B}", "k").unwrap(),
                    "1/2"
                );
            },
        );
    }

    #[test]
    fn unset_env_var_names_the_key_and_the_variable() {
        with_var("WORKMUX_TEST_MISSING", None, || {
            let err = expand("${env:WORKMUX_TEST_MISSING}", "provision.token")
                .unwrap_err()
                .to_string();
            assert!(err.contains("provision.token"), "{err}");
            assert!(err.contains("WORKMUX_TEST_MISSING"), "{err}");
        });
    }

    #[test]
    fn empty_env_var_is_an_error() {
        with_var("WORKMUX_TEST_EMPTY", Some(""), || {
            assert!(expand("${env:WORKMUX_TEST_EMPTY}", "k").is_err());
        });
    }

    #[test]
    fn expands_a_file_placeholder() {
        let path = tmpfile("ok.txt", "  file-secret\n", 0o600);
        assert_eq!(
            expand(&format!("${{file:{}}}", path.display()), "k").unwrap(),
            "file-secret",
            "surrounding whitespace is trimmed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn world_readable_file_is_rejected() {
        let path = tmpfile("loose.txt", "secret", 0o644);
        let err = expand(&format!("${{file:{}}}", path.display()), "provision.token")
            .unwrap_err()
            .to_string();
        assert!(err.contains("provision.token"), "{err}");
        assert!(err.contains("644"), "{err}");
    }

    #[test]
    fn missing_file_is_an_error() {
        let err = expand("${file:/nonexistent/workmux/secret}", "k")
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn empty_file_is_an_error() {
        let path = tmpfile("empty.txt", "   \n", 0o600);
        assert!(expand(&format!("${{file:{}}}", path.display()), "k").is_err());
    }

    #[test]
    fn unknown_placeholder_kinds_pass_through() {
        assert_eq!(expand("${HOME}", "k").unwrap(), "${HOME}");
        assert_eq!(expand("${vault:x}", "k").unwrap(), "${vault:x}");
    }

    #[test]
    fn unterminated_placeholder_is_literal() {
        assert_eq!(expand("${env:UNCLOSED", "k").unwrap(), "${env:UNCLOSED");
    }

    #[test]
    fn plain_values_are_unchanged() {
        assert_eq!(expand("just-a-token", "k").unwrap(), "just-a-token");
    }
}

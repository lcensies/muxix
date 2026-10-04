//! prime-agent status tracking setup.
//!
//! prime-agent is PrimeIntellect's hard fork of pi: same `settings.json`
//! (`packages` plugin list), same extension API, same `APPEND_SYSTEM.md` /
//! `muxix-pre-inject.md` prompt files — so every body here delegates to pi's
//! helpers against prime's own directory. What the fork renames:
//!
//! - config dir: `~/.prime/agent`, `PRIME_AGENT_CODING_AGENT_DIR`
//!   (`piConfig.configDir` in its package.json)
//! - plugins: `prime-agent package install|remove`, not `pi install|remove`

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::StatusCheck;
use super::pi::{self, PiInjectionMethod};

/// prime-agent's config/agent directory, honoring `PRIME_AGENT_CODING_AGENT_DIR`.
pub fn agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PRIME_AGENT_CODING_AGENT_DIR") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".prime/agent"))
}

/// prime-agent's settings file, co-owned the way pi's is.
pub fn settings_file() -> Option<PathBuf> {
    Some(agent_dir()?.join("settings.json"))
}

/// Detect prime-agent via its config directory.
pub fn detect() -> Option<&'static str> {
    if std::env::var("PRIME_AGENT_CODING_AGENT_DIR").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $PRIME_AGENT_CODING_AGENT_DIR");
    }
    if agent_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.prime/agent/");
    }
    None
}

/// Whether the muxix status extension is installed for prime-agent, and current.
pub fn check() -> Result<StatusCheck> {
    let Some(dir) = agent_dir() else {
        return Ok(StatusCheck::NotInstalled);
    };
    pi::check_in(&dir)
}

/// Install the muxix status extension for prime-agent.
pub fn install() -> Result<String> {
    let dir = agent_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    pi::install_in(&dir, "prime-agent")
}

/// Whether `spec` is already registered in prime's `settings.json` `packages`.
pub fn plugin_installed(spec: &str, project_root: &Path) -> bool {
    let Some(dir) = agent_dir() else {
        return false;
    };
    pi::plugin_installed_in(&dir, spec, project_root)
}

/// Install an extension with `prime-agent package install`, which registers it
/// in `settings.json` so it also loads on bare launches.
pub fn install_plugin_from_url(url: &str) -> Result<String> {
    let plugin_name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("plugin");

    let status = Command::new("prime-agent")
        .args(["package", "install", url])
        .status()
        .context("Failed to run `prime-agent package install` — is prime-agent on PATH?")?;

    if !status.success() {
        anyhow::bail!("`prime-agent package install {}` failed", url);
    }

    Ok(format!("Installed prime-agent extension: {}", plugin_name))
}

/// Remove an extension registration. The spec is passed through verbatim, so
/// this is called from the project root that declared it.
pub fn remove_plugin(spec: &str) -> Result<String> {
    let status = Command::new("prime-agent")
        .args(["package", "remove", spec])
        .status()
        .context("Failed to run `prime-agent package remove` — is prime-agent on PATH?")?;

    if !status.success() {
        anyhow::bail!("`prime-agent package remove {}` failed", spec);
    }

    Ok(format!("Removed prime-agent extension: {}", spec))
}

/// Prompt bootstrapper: pi's, pointed at prime's agent dir.
pub fn bootstrapper(method: PiInjectionMethod) -> Option<pi::Bootstrapper> {
    agent_dir().map(|dir| pi::Bootstrapper::new_in(dir, method))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // PRIME_AGENT_CODING_AGENT_DIR is process-global; serialize the tests that set it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_agent_dir(dir: &Path, f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("PRIME_AGENT_CODING_AGENT_DIR").ok();
        unsafe { std::env::set_var("PRIME_AGENT_CODING_AGENT_DIR", dir) };
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PRIME_AGENT_CODING_AGENT_DIR", v),
                None => std::env::remove_var("PRIME_AGENT_CODING_AGENT_DIR"),
            }
        }
    }

    /// The fork must never be served pi's directory: a prime dir with the
    /// extension present is installed even when pi's is empty.
    #[test]
    fn extension_state_is_read_from_primes_own_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ext = tmp.path().join("extensions/muxix-status.ts");
        fs::create_dir_all(ext.parent().unwrap()).unwrap();
        with_agent_dir(tmp.path(), || {
            assert!(matches!(check().unwrap(), StatusCheck::NotInstalled));

            fs::write(&ext, "// an older muxix build").unwrap();
            assert!(matches!(check().unwrap(), StatusCheck::Stale { .. }));

            assert!(install().unwrap().contains("prime-agent"));
            assert!(matches!(check().unwrap(), StatusCheck::Installed));
        });
    }

    #[test]
    fn plugin_installed_reads_primes_packages_array() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("settings.json"),
            r#"{"packages":["npm:pi-subagents"]}"#,
        )
        .unwrap();
        with_agent_dir(tmp.path(), || {
            assert!(plugin_installed("npm:pi-subagents", tmp.path()));
            assert!(!plugin_installed("npm:not-there", tmp.path()));
        });
    }

    #[test]
    fn agent_dir_defaults_outside_pis_tree() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("PRIME_AGENT_CODING_AGENT_DIR").ok();
        unsafe { std::env::remove_var("PRIME_AGENT_CODING_AGENT_DIR") };
        let dir = agent_dir().unwrap();
        assert!(dir.ends_with(".prime/agent"), "{dir:?}");
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("PRIME_AGENT_CODING_AGENT_DIR", v);
            }
        }
    }
}

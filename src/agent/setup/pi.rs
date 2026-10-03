//! Pi agent status tracking setup.
//!
//! Detects pi via its config directory at `~/.pi/agent/`.
//! Override with `PI_CODING_AGENT_DIR` env var.
//!
//! Installs extension by writing `workmux-status.ts` to the extensions directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::StatusCheck;

/// The pi extension source, embedded at compile time.
const EXTENSION_SOURCE: &str = include_str!("../../../.pi/extensions/workmux-status.ts");

/// How workmux injects the configured system prompt into pi.
///
/// When pi runs through cliproxy, the proxy forces its own system prompt and
/// discards `APPEND_SYSTEM.md` at the API level. `BeforeAgentStart` is the
/// fallback: the extension reads a separate file and injects it via pi's
/// `context` hook, appending the content to the first user message. The
/// conversation payload survives the proxy where the system prompt does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PiInjectionMethod {
    /// Inject via the extension's `context` hook (default), appending the
    /// content to the first user message. Writes to
    /// `~/.pi/agent/workmux-pre-inject.md`.
    #[default]
    BeforeAgentStart,
    /// Write directly to `APPEND_SYSTEM.md` (native pi mechanism).
    /// Does not survive cliproxy, which strips system prompts at the API level.
    AppendSystem,
}

/// Pi-specific bootstrap configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PiBootstrapConfig {
    /// How to inject the workmux system prompt. Default: `before_agent_start`.
    #[serde(default)]
    pub injection_method: PiInjectionMethod,
}

fn pi_agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".pi/agent"))
}

/// Pi's own settings file (`~/.pi/agent/settings.json`), co-owned: pi writes
/// `packages` and its own preferences here too.
pub fn settings_file() -> Option<PathBuf> {
    Some(pi_agent_dir()?.join("settings.json"))
}

/// Where pi's `@hsingjui/pi-hooks` compat extension reads Claude-format hooks:
/// the `hooks` key of the general settings file (the writer only appends under
/// that key, sibling settings are untouched).
pub fn declared_hook_target() -> Option<crate::command::setup::agent_hooks::HookTarget> {
    use crate::command::setup::agent_hooks::{HookTarget, RequiredPlugin};
    fn key(event: crate::bootstrap::HookEvent) -> Option<&'static str> {
        use crate::bootstrap::HookEvent;
        match event {
            HookEvent::SessionReady => Some("SessionStart"),
            HookEvent::TurnDone => Some("Stop"),
        }
    }
    Some(HookTarget {
        file: settings_file()?,
        event_key: key,
        requires_plugin: Some(RequiredPlugin {
            spec: "npm:@hsingjui/pi-hooks",
            name_fragment: "pi-hooks",
        }),
    })
}

fn extension_path() -> Option<PathBuf> {
    pi_agent_dir().map(|d| d.join("extensions/workmux-status.ts"))
}

pub struct Bootstrapper {
    agent_dir: PathBuf,
    injection_method: PiInjectionMethod,
}

impl Bootstrapper {
    pub fn new() -> Option<Self> {
        pi_agent_dir().map(|d| Self {
            agent_dir: d,
            injection_method: PiInjectionMethod::default(),
        })
    }

    pub fn new_with_method(method: PiInjectionMethod) -> Option<Self> {
        pi_agent_dir().map(|d| Self {
            agent_dir: d,
            injection_method: method,
        })
    }

    /// Path where the `BeforeAgentStart` method writes the inject content.
    fn pre_inject_path(&self) -> PathBuf {
        self.agent_dir.join("workmux-pre-inject.md")
    }
}

impl super::AgentBootstrapper for Bootstrapper {
    fn instructions_path(&self) -> PathBuf {
        match self.injection_method {
            PiInjectionMethod::AppendSystem => self.agent_dir.join("APPEND_SYSTEM.md"),
            PiInjectionMethod::BeforeAgentStart => self.pre_inject_path(),
        }
    }

    fn apply_prompt(&self, prompt: &str) -> anyhow::Result<()> {
        match self.injection_method {
            PiInjectionMethod::AppendSystem => {
                // Write raw content without HTML comment sentinels.
                // APPEND_SYSTEM.md is fed directly into the model's system prompt —
                // HTML comment sentinels would be invisible to the model.
                let path = self.agent_dir.join("APPEND_SYSTEM.md");
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, format!("{}\n", prompt.trim()))?;
            }
            PiInjectionMethod::BeforeAgentStart => {
                // Write to a separate file read by the extension's before_agent_start
                // hook. This survives cliproxy, which strips APPEND_SYSTEM.md at
                // the API level.
                let path = self.pre_inject_path();
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, format!("{}\n", prompt.trim()))?;
            }
        }
        Ok(())
    }
}

/// Detect if pi is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if std::env::var("PI_CODING_AGENT_DIR").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $PI_CODING_AGENT_DIR");
    }
    if pi_agent_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.pi/agent/");
    }
    None
}

/// Check if workmux extension is installed for pi, and current.
///
/// The installed file is a copy of `EXTENSION_SOURCE`, so anything else means
/// workmux was upgraded without a re-setup; existence alone would pin users to
/// whichever version first ran setup.
pub fn check() -> Result<StatusCheck> {
    let Some(path) = extension_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    match fs::read_to_string(&path) {
        Err(_) => Ok(StatusCheck::NotInstalled),
        Ok(body) if body == EXTENSION_SOURCE => Ok(StatusCheck::Installed),
        Ok(_) => Ok(StatusCheck::Stale {
            missing: vec!["workmux-status.ts differs from this workmux build".into()],
        }),
    }
}

/// Whether `spec` is already registered in pi's `settings.json` `packages`.
///
/// `pi install` records each installed spec verbatim in that array, but for
/// path-shaped specs (`./vendor/x`, not `npm:x`/`git:...`) the recorded string
/// is relative to pi's own agent dir while the declared spec is relative to
/// the project root — same plugin, different strings. Path-shaped specs are
/// therefore compared as canonical filesystem paths; scheme-prefixed specs
/// keep exact-string comparison. On any read/parse failure, or if a
/// path-shaped spec doesn't canonicalize (not yet installed), this returns
/// `false` so setup still attempts the install rather than skipping a
/// genuinely missing plugin.
pub fn plugin_installed(spec: &str, project_root: &std::path::Path) -> bool {
    let Some(dir) = pi_agent_dir() else {
        return false;
    };
    let Ok(body) = fs::read_to_string(dir.join("settings.json")) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
        return false;
    };
    let Some(arr) = json.get("packages").and_then(|p| p.as_array()) else {
        return false;
    };

    if super::spec::is_path_shaped(spec) {
        let Some(declared) = super::spec::canonicalize_path_spec(spec, project_root) else {
            return false;
        };
        return arr.iter().any(|v| {
            v.as_str()
                .and_then(|s| super::spec::canonicalize_path_spec(s, &dir))
                .is_some_and(|entry| entry == declared)
        });
    }

    arr.iter().any(|v| v.as_str() == Some(spec))
}

/// Install a pi extension from a URL using `pi install`.
///
/// This is the correct mechanism — `pi install` registers the extension in
/// `settings.json` so it loads on every pi startup, including bare launches
/// not managed by workmux.
pub fn install_plugin_from_url(url: &str) -> Result<String> {
    let plugin_name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("plugin");

    let status = Command::new("pi")
        .args(["install", url])
        .status()
        .context("Failed to run `pi install` — is pi on PATH?")?;

    if !status.success() {
        anyhow::bail!("`pi install {}` failed", url);
    }

    Ok(format!("Installed pi extension: {}", plugin_name))
}

/// Remove a pi extension registration with `pi remove`.
///
/// The spec is passed through verbatim; pi resolves path-shaped specs against
/// the cwd, so this is called from the project root that declared it.
pub fn remove_plugin(spec: &str) -> Result<String> {
    let status = Command::new("pi")
        .args(["remove", spec])
        .status()
        .context("Failed to run `pi remove` \u{2014} is pi on PATH?")?;

    if !status.success() {
        anyhow::bail!("`pi remove {}` failed", spec);
    }

    Ok(format!("Removed pi extension: {}", spec))
}

/// Install workmux extension for pi.
/// Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        extension_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create pi extensions directory")?;
    }

    fs::write(&path, EXTENSION_SOURCE).context("Failed to write pi extension")?;

    Ok(format!(
        "Installed extension to {}. Restart pi for it to take effect.",
        path.display()
    ))
}

#[cfg(test)]
mod plugin_installed_tests {
    use super::*;

    // PI_CODING_AGENT_DIR is process-global; serialize the tests that set it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_agent_dir(dir: &std::path::Path, f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("PI_CODING_AGENT_DIR").ok();
        unsafe { std::env::set_var("PI_CODING_AGENT_DIR", dir) };
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
                None => std::env::remove_var("PI_CODING_AGENT_DIR"),
            }
        }
    }

    #[test]
    fn outdated_extension_copy_is_stale_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let ext = tmp.path().join("extensions/workmux-status.ts");
        fs::create_dir_all(ext.parent().unwrap()).unwrap();
        with_agent_dir(tmp.path(), || {
            assert!(matches!(check().unwrap(), StatusCheck::NotInstalled));

            fs::write(&ext, "// an older workmux build").unwrap();
            assert!(matches!(check().unwrap(), StatusCheck::Stale { .. }));

            fs::write(&ext, EXTENSION_SOURCE).unwrap();
            assert!(matches!(check().unwrap(), StatusCheck::Installed));
        });
    }

    #[test]
    fn matches_only_exact_packages_entries() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("settings.json"),
            r#"{"packages":["npm:pi-subagents","git:github.com/x/y"]}"#,
        )
        .unwrap();
        // Scheme-shaped specs ignore project_root, so any path works here.
        let project_root = tmp.path();
        with_agent_dir(tmp.path(), || {
            assert!(plugin_installed("npm:pi-subagents", project_root));
            assert!(plugin_installed("git:github.com/x/y", project_root));
            assert!(!plugin_installed("npm:pi-sub", project_root));
            assert!(!plugin_installed("npm:not-there", project_root));
        });
    }

    #[test]
    fn missing_or_bad_settings_reports_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        with_agent_dir(tmp.path(), || {
            assert!(
                !plugin_installed("npm:anything", tmp.path()),
                "no settings.json"
            );
        });
        fs::write(tmp.path().join("settings.json"), "not json").unwrap();
        with_agent_dir(tmp.path(), || {
            assert!(
                !plugin_installed("npm:anything", tmp.path()),
                "unparseable settings"
            );
        });
    }

    /// The proposal's motivating bug: pi records a path spec relative to its
    /// own agent dir (`../repos/proj/vendor/x`), while the project declares
    /// it relative to the project root (`./vendor/x`). Same plugin, same
    /// canonical path — the probe must match.
    #[test]
    fn path_shaped_spec_matches_across_relative_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("repos/proj");
        fs::create_dir_all(project_root.join("vendor/x")).unwrap();

        let agent_dir = tmp.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            r#"{"packages":["../repos/proj/vendor/x"]}"#,
        )
        .unwrap();

        with_agent_dir(&agent_dir, || {
            assert!(plugin_installed("./vendor/x", &project_root));
        });
    }

    #[test]
    fn path_shaped_spec_with_no_matching_entry_is_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("proj");
        fs::create_dir_all(project_root.join("vendor/x")).unwrap();
        fs::create_dir_all(project_root.join("vendor/y")).unwrap();

        let agent_dir = tmp.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            format!(
                r#"{{"packages":["{}"]}}"#,
                project_root.join("vendor/y").to_str().unwrap()
            ),
        )
        .unwrap();

        with_agent_dir(&agent_dir, || {
            assert!(!plugin_installed("./vendor/x", &project_root));
        });
    }

    #[test]
    fn nonexistent_path_shaped_declared_spec_is_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            r#"{"packages":["./vendor/x"]}"#,
        )
        .unwrap();

        with_agent_dir(&agent_dir, || {
            assert!(!plugin_installed("./vendor/x", tmp.path()));
        });
    }
}

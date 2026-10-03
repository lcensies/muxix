//! oh-my-pi (omp) agent status tracking setup.
//!
//! omp is a pi-compatible agent: same extension mechanism and prompt-injection
//! behavior as pi, but with its own command (`omp`) and config directory at
//! `~/.omp/agent/`. Override with the `OMP_CODING_AGENT_DIR` env var.
//!
//! Installs the same muxix extension as pi by writing `muxix-status.ts`
//! to the extensions directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::StatusCheck;
use super::pi::PiInjectionMethod;

/// The extension source, shared with pi (omp is pi-compatible).
const EXTENSION_SOURCE: &str = include_str!("../../../.pi/extensions/muxix-status.ts");

/// omp's agent directory.
///
/// omp itself honors `PI_CODING_AGENT_DIR` (its pi heritage: the var relocates
/// the default profile's agent dir, `config.yml` and agent data included; omp's
/// own named profiles ignore it). `OMP_CODING_AGENT_DIR` is a muxix-side
/// override only and wins when set, so a host can point muxix at an omp tree
/// without also relocating pi.
pub fn agent_dir() -> Option<PathBuf> {
    for var in ["OMP_CODING_AGENT_DIR", "PI_CODING_AGENT_DIR"] {
        if let Ok(dir) = std::env::var(var) {
            return Some(PathBuf::from(dir));
        }
    }
    home::home_dir().map(|h| h.join(".omp/agent"))
}

fn extension_path() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("extensions/muxix-status.ts"))
}

/// omp's own settings file.
///
/// omp is pi-compatible in its extension and prompt mechanisms but NOT in its
/// settings store: it keeps settings in YAML at `<agent dir>/config.yml` and
/// never reads pi's `settings.json`. A declared `settings:` patch is applied as
/// YAML (see `settings_format`).
pub fn settings_file() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("config.yml"))
}

pub struct Bootstrapper {
    agent_dir: PathBuf,
    injection_method: PiInjectionMethod,
}

impl Bootstrapper {
    #[allow(dead_code)]
    pub fn new() -> Option<Self> {
        agent_dir().map(|d| Self {
            agent_dir: d,
            injection_method: PiInjectionMethod::default(),
        })
    }

    pub fn new_with_method(method: PiInjectionMethod) -> Option<Self> {
        agent_dir().map(|d| Self {
            agent_dir: d,
            injection_method: method,
        })
    }

    /// Path where the `BeforeAgentStart` method writes the inject content.
    fn pre_inject_path(&self) -> PathBuf {
        self.agent_dir.join("muxix-pre-inject.md")
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
        let path = match self.injection_method {
            PiInjectionMethod::AppendSystem => self.agent_dir.join("APPEND_SYSTEM.md"),
            PiInjectionMethod::BeforeAgentStart => self.pre_inject_path(),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, format!("{}\n", prompt.trim()))?;
        Ok(())
    }
}

/// Detect if omp is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if std::env::var("OMP_CODING_AGENT_DIR").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $OMP_CODING_AGENT_DIR");
    }
    if agent_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.omp/agent/");
    }
    None
}

/// Check if muxix extension is installed for omp, and current.
///
/// Content compare, not existence: an upgraded muxix must overwrite an older
/// copy of the extension (see `pi::check`).
pub fn check() -> Result<StatusCheck> {
    let Some(path) = extension_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    match fs::read_to_string(&path) {
        Err(_) => Ok(StatusCheck::NotInstalled),
        Ok(body) if body == EXTENSION_SOURCE => Ok(StatusCheck::Installed),
        Ok(_) => Ok(StatusCheck::Stale {
            missing: vec!["muxix-status.ts differs from this muxix build".into()],
        }),
    }
}

/// Whether `spec` is already registered in omp's `settings.json` `packages`.
///
/// omp is pi-compatible: `omp install` records each installed spec verbatim,
/// and for path-shaped specs (`./vendor/x`, not `npm:x`/`git:...`) the
/// recorded string is relative to omp's own agent dir while the declared spec
/// is relative to the project root — same plugin, different strings.
/// Path-shaped specs are therefore compared as canonical filesystem paths;
/// scheme-prefixed specs keep exact-string comparison. On any read/parse
/// failure, or if a path-shaped spec doesn't canonicalize (not yet
/// installed), this returns `false` so setup still attempts the install
/// rather than skipping a genuinely missing plugin.
#[allow(dead_code)]
pub fn plugin_installed(spec: &str, project_root: &std::path::Path) -> bool {
    let Some(dir) = agent_dir() else {
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

/// Install an omp extension from a URL using `omp install`.
///
/// `omp install` registers the extension in `settings.json` so it loads on
/// every omp startup, including bare launches not managed by muxix.
pub fn install_plugin_from_url(url: &str) -> Result<String> {
    let plugin_name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("plugin");

    let status = Command::new("omp")
        .args(["install", url])
        .status()
        .context("Failed to run `omp install` — is omp on PATH?")?;

    if !status.success() {
        anyhow::bail!("`omp install {}` failed", url);
    }

    Ok(format!("Installed omp extension: {}", plugin_name))
}

/// Remove an omp extension registration with `omp remove`.
pub fn remove_plugin(spec: &str) -> Result<String> {
    let status = Command::new("omp")
        .args(["remove", spec])
        .status()
        .context("Failed to run `omp remove` \u{2014} is omp on PATH?")?;

    if !status.success() {
        anyhow::bail!("`omp remove {}` failed", spec);
    }

    Ok(format!("Removed omp extension: {}", spec))
}

/// Install muxix extension for omp.
/// Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        extension_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create omp extensions directory")?;
    }

    fs::write(&path, EXTENSION_SOURCE).context("Failed to write omp extension")?;

    Ok(format!(
        "Installed extension to {}. Restart omp for it to take effect.",
        path.display()
    ))
}

#[cfg(test)]
mod plugin_installed_tests {
    use super::*;

    // OMP_CODING_AGENT_DIR is process-global; serialize the tests that set it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_agent_dir(dir: &std::path::Path, f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("OMP_CODING_AGENT_DIR").ok();
        unsafe { std::env::set_var("OMP_CODING_AGENT_DIR", dir) };
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("OMP_CODING_AGENT_DIR", v),
                None => std::env::remove_var("OMP_CODING_AGENT_DIR"),
            }
        }
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

    /// The proposal's motivating bug, omp equivalent: omp records a path spec
    /// relative to its own agent dir (`../repos/proj/vendor/x`), while the
    /// project declares it relative to the project root (`./vendor/x`). Same
    /// plugin, same canonical path — the probe must match.
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

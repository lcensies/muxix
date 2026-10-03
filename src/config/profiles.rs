//! Named config profiles: selection and overlay.
//!
//! A profile is a partial config declared under `profiles:` and applied on top
//! of the resolved base when selected. Profiles are how one config file covers
//! several situations — a corporate machine and a personal one, a Python stack
//! and a Rust one — without needing a separate file per case.
//!
//! # Why profiles are flat
//!
//! A profile may not declare `profiles:` or `include:` of its own. Nesting
//! those would make resolution order a graph problem with no obvious reading
//! order, and the failure mode is a config nobody can explain. Composition
//! already has a mechanism — `include:` — and it operates on files, where a
//! reader can follow it.

use anyhow::{Result, bail};
use serde_yaml::Value;
use std::collections::BTreeMap;

use super::resolve::{Layer, LayerKind};

/// Environment variable naming the profile(s) to apply.
pub const PROFILE_ENV: &str = "WORKMUX_PROFILE";

/// The `--profile` flag's value, captured once at startup.
///
/// `Config::load` is called from dozens of places, most of which have no access
/// to parsed CLI args, so the flag is recorded here rather than threaded
/// through every signature. [`select`] still applies it at CLI precedence, and
/// [`select_with`] takes it explicitly so tests never depend on this global.
static CLI_PROFILE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Record the `--profile` flag. Called once, from CLI startup.
pub fn set_cli_profile(value: Option<String>) {
    let _ = CLI_PROFILE.set(value);
}

/// The recorded `--profile` value, if the flag was given.
pub fn cli_profile() -> Option<&'static str> {
    CLI_PROFILE.get().and_then(|v| v.as_deref())
}

/// Which profiles to apply, and where that decision came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub names: Vec<String>,
    /// Human-readable origin, for error messages and `--explain`.
    pub source: &'static str,
}

impl Selection {
    fn none(source: &'static str) -> Self {
        Self {
            names: Vec::new(),
            source,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Split a comma-separated profile list, dropping empty entries.
fn split_names(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Resolve which profiles apply.
///
/// Precedence, highest first: the `--profile` flag, `WORKMUX_PROFILE`, the
/// config's `default_profile`. A higher-precedence source *replaces* the list
/// from lower ones rather than adding to it, so `--profile x` on a machine with
/// `WORKMUX_PROFILE=y` applies only `x`.
///
/// An explicitly empty value at any level disables profiles entirely — that is
/// how a user turns off a `default_profile` for one command.
pub fn select(cli: Option<&str>, default_profile: Option<&str>) -> Selection {
    if let Some(raw) = cli {
        let names = split_names(raw);
        return if names.is_empty() {
            Selection::none("--profile (empty)")
        } else {
            Selection {
                names,
                source: "--profile",
            }
        };
    }

    if let Ok(raw) = std::env::var(PROFILE_ENV) {
        let names = split_names(&raw);
        return if names.is_empty() {
            Selection::none("WORKMUX_PROFILE (empty)")
        } else {
            Selection {
                names,
                source: "WORKMUX_PROFILE",
            }
        };
    }

    if let Some(raw) = default_profile {
        let names = split_names(raw);
        if !names.is_empty() {
            return Selection {
                names,
                source: "default_profile",
            };
        }
    }

    Selection::none("none")
}

/// Read the `profiles:` block out of one layer's value.
///
/// Returns an empty map when the key is absent, so a config that declares no
/// profiles costs nothing.
pub fn declared(value: &Value) -> Result<BTreeMap<String, Value>> {
    let Some(raw) = value.get("profiles") else {
        return Ok(BTreeMap::new());
    };
    if raw.is_null() {
        return Ok(BTreeMap::new());
    }
    let Value::Mapping(map) = raw else {
        bail!("`profiles:` must be a mapping of profile name to config");
    };

    let mut out = BTreeMap::new();
    for (key, body) in map {
        let Some(name) = key.as_str() else {
            bail!("profile names must be strings");
        };
        if body.get("profiles").is_some() {
            bail!("profile `{name}` declares nested `profiles:`, which is not supported");
        }
        if body.get("include").is_some() {
            bail!(
                "profile `{name}` declares `include:`, which is not supported -- \
                 put the include at the top level of the file instead"
            );
        }
        out.insert(name.to_string(), body.clone());
    }
    Ok(out)
}

/// Remove the `profiles:` key from a value.
///
/// Like `include:`, it is directive rather than configuration and must not
/// survive into the merged result.
pub fn strip_profiles_key(value: &mut Value) {
    if let Value::Mapping(map) = value {
        map.remove(Value::String("profiles".to_string()));
    }
}

/// Build the layers for a selection, applied left to right.
///
/// `available` is the union of profiles declared across every file layer;
/// `trusted` records, per profile name, whether the layer that declared it may
/// set global-only keys — a profile declared in a project config is no more
/// trusted than the project config itself.
pub fn layers(
    selection: &Selection,
    available: &BTreeMap<String, Value>,
    trusted: &BTreeMap<String, bool>,
) -> Result<Vec<Layer>> {
    if selection.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::with_capacity(selection.names.len());
    for name in &selection.names {
        let Some(body) = available.get(name) else {
            let mut known: Vec<&str> = available.keys().map(String::as_str).collect();
            known.sort_unstable();
            let known = if known.is_empty() {
                "none are declared".to_string()
            } else {
                known.join(", ")
            };
            bail!(
                "unknown profile `{name}` (selected via {}); available profiles: {known}",
                selection.source
            );
        };
        out.push(
            Layer::new(
                format!("profile:{name}"),
                format!("profile `{name}` ({})", selection.source),
                LayerKind::Profile,
                body.clone(),
            )
            .trusted(trusted.get(name).copied().unwrap_or(false)),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    /// `WORKMUX_PROFILE` is process-global, so the tests that touch it are
    /// serialized behind this lock rather than racing each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(PROFILE_ENV).ok();
        unsafe {
            match value {
                Some(v) => std::env::set_var(PROFILE_ENV, v),
                None => std::env::remove_var(PROFILE_ENV),
            }
        }
        let out = f();
        unsafe {
            match previous {
                Some(v) => std::env::set_var(PROFILE_ENV, v),
                None => std::env::remove_var(PROFILE_ENV),
            }
        }
        out
    }

    // --- selection precedence ---------------------------------------------

    #[test]
    fn cli_flag_beats_env_var() {
        let sel = with_env(Some("corp"), || select(Some("personal"), None));
        assert_eq!(sel.names, vec!["personal"]);
    }

    #[test]
    fn env_var_beats_default_profile() {
        let sel = with_env(Some("personal"), || select(None, Some("corp")));
        assert_eq!(sel.names, vec!["personal"]);
    }

    #[test]
    fn default_profile_used_when_nothing_else_selects() {
        let sel = with_env(None, || select(None, Some("corp")));
        assert_eq!(sel.names, vec!["corp"]);
        assert_eq!(sel.source, "default_profile");
    }

    #[test]
    fn nothing_selected_is_empty() {
        let sel = with_env(None, || select(None, None));
        assert!(sel.is_empty());
    }

    #[test]
    fn empty_cli_flag_disables_a_default_profile() {
        let sel = with_env(None, || select(Some(""), Some("corp")));
        assert!(sel.is_empty());
    }

    #[test]
    fn empty_env_var_disables_a_default_profile() {
        let sel = with_env(Some(""), || select(None, Some("corp")));
        assert!(sel.is_empty());
    }

    #[test]
    fn comma_separated_names_are_split_and_trimmed() {
        let sel = with_env(None, || select(Some("a, b ,c"), None));
        assert_eq!(sel.names, vec!["a", "b", "c"]);
    }

    #[test]
    fn higher_precedence_replaces_rather_than_appends() {
        let sel = with_env(Some("env1,env2"), || select(Some("cli1"), Some("def1")));
        assert_eq!(sel.names, vec!["cli1"]);
    }

    // --- declaration -------------------------------------------------------

    #[test]
    fn absent_profiles_key_is_empty() {
        assert!(declared(&yaml("agent: claude")).unwrap().is_empty());
    }

    #[test]
    fn declared_profiles_are_read() {
        let got = declared(&yaml("profiles:\n  corp:\n    agent: codex")).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            got["corp"].get("agent").unwrap().as_str(),
            Some("codex")
        );
    }

    #[test]
    fn nested_profiles_are_rejected() {
        let err = declared(&yaml("profiles:\n  corp:\n    profiles:\n      inner:\n        agent: x"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("corp"), "{err}");
        assert!(err.contains("nested"), "{err}");
    }

    #[test]
    fn include_inside_a_profile_is_rejected() {
        let err = declared(&yaml("profiles:\n  corp:\n    include: [./a.yaml]"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("corp"), "{err}");
        assert!(err.contains("include"), "{err}");
    }

    #[test]
    fn non_mapping_profiles_block_is_rejected() {
        assert!(declared(&yaml("profiles: 5")).is_err());
    }

    #[test]
    fn strip_removes_the_key() {
        let mut value = yaml("agent: claude\nprofiles:\n  corp:\n    agent: codex");
        strip_profiles_key(&mut value);
        assert!(value.get("profiles").is_none());
        assert!(value.get("agent").is_some());
    }

    // --- layers ------------------------------------------------------------

    #[test]
    fn no_selection_produces_no_layers() {
        let available = declared(&yaml("profiles:\n  corp:\n    agent: codex")).unwrap();
        let sel = Selection::none("none");
        assert!(layers(&sel, &available, &BTreeMap::new()).unwrap().is_empty());
    }

    #[test]
    fn selected_profile_becomes_a_layer() {
        let available = declared(&yaml("profiles:\n  corp:\n    agent: codex")).unwrap();
        let sel = Selection {
            names: vec!["corp".into()],
            source: "--profile",
        };
        let got = layers(&sel, &available, &BTreeMap::new()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value.get("agent").unwrap().as_str(), Some("codex"));
    }

    #[test]
    fn multiple_profiles_keep_selection_order() {
        let available =
            declared(&yaml("profiles:\n  a:\n    agent: one\n  b:\n    agent: two")).unwrap();
        let sel = Selection {
            names: vec!["b".into(), "a".into()],
            source: "--profile",
        };
        let got = layers(&sel, &available, &BTreeMap::new()).unwrap();
        assert_eq!(got[0].value.get("agent").unwrap().as_str(), Some("two"));
        assert_eq!(got[1].value.get("agent").unwrap().as_str(), Some("one"));
    }

    #[test]
    fn unknown_profile_lists_the_available_names() {
        let available =
            declared(&yaml("profiles:\n  corp:\n    agent: x\n  home:\n    agent: y")).unwrap();
        let sel = Selection {
            names: vec!["nope".into()],
            source: "--profile",
        };
        let err = layers(&sel, &available, &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains("corp"), "{err}");
        assert!(err.contains("home"), "{err}");
    }

    #[test]
    fn unknown_profile_with_none_declared_says_so() {
        let sel = Selection {
            names: vec!["nope".into()],
            source: "--profile",
        };
        let err = layers(&sel, &BTreeMap::new(), &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("none are declared"), "{err}");
    }

    #[test]
    fn profile_trust_is_inherited_from_its_declaring_layer() {
        let available =
            declared(&yaml("profiles:\n  g:\n    agent: x\n  p:\n    agent: y")).unwrap();
        let trusted = BTreeMap::from([("g".to_string(), true), ("p".to_string(), false)]);
        let sel = Selection {
            names: vec!["g".into(), "p".into()],
            source: "--profile",
        };
        let got = layers(&sel, &available, &trusted).unwrap();
        assert!(got[0].trusted, "global-declared profile is trusted");
        assert!(!got[1].trusted, "project-declared profile is not");
    }
}

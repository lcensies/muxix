//! Agent config profiles: lightweight, host-override differentiation of an
//! agent's skills / extensions / config without sandboxing.
//!
//! # Model
//!
//! The real agent config dir (e.g. `~/.pi/agent/`, `~/.claude/`) is the
//! **base** — it is also the **default**, and workmux never wholesale-overrides
//! it. A named profile layers *on top* of base:
//!
//! ```text
//! base (~/.pi/agent, shared with the user)
//!    ⊕  profile source (user-authored deltas)
//!    ↓ materialized as ↓
//! derived overlay dir  ← the agent's config-dir env var points here
//! ```
//!
//! Two directories per profile, with opposite ownership:
//!
//! * **source** — `~/.config/workmux/agent-profiles/<name>/` — user-authored.
//!   Setup only *reads* it and **never prunes** it.
//! * **derived** — `~/.local/state/workmux/agent-profiles/<name>/<agent>/` — a
//!   symlink-farm merge of base + source (source wins on conflicts). Fully
//!   workmux-owned: rebuilt and pruned freely, because it is reconstructable.
//!
//! Because pruning only ever touches the derived tree, deleting a removed
//! profile can never destroy user data — the source tree is left untouched.
//!
//! `workmux exec --profile <name> <agent>` points the agent's config-dir env
//! var at the derived dir; with no profile it falls through to base, so a bare
//! agent and `workmux exec` with no profile use the exact same directory and
//! can never drift.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::config::AgentProfileAgent;

/// One entry of the desired overlay: either a symlink or a generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanEntry {
    /// Symlink to an absolute target (base or profile source).
    Link(PathBuf),
    /// Regular file with generated content (settings delta, rendered prompt).
    File(String),
}

pub type Plan = BTreeMap<PathBuf, PlanEntry>;

/// Declarative delta artifacts, computed from `agent_profiles.<name>.agents.<id>`
/// before planning. Empty (`Default`) means plain file-overlay behavior.
#[derive(Debug, Default)]
pub struct DeltaPlan {
    /// rel path → generated file content, written as a regular file.
    pub generated: BTreeMap<PathBuf, String>,
    /// rel paths omitted from the overlay entirely.
    pub excludes: BTreeSet<PathBuf>,
    /// rel path → absolute link target added on top of base (source still wins).
    pub extra_links: BTreeMap<PathBuf, PathBuf>,
    /// Human-readable warnings surfaced as setup items.
    pub warnings: Vec<String>,
}

impl DeltaPlan {
    /// Whether any delta path lies strictly below `rel` (forces recursion into
    /// a subtree that would otherwise be symlinked wholesale).
    fn touches_below(&self, rel: &Path) -> bool {
        self.generated
            .keys()
            .chain(self.extra_links.keys())
            .chain(self.excludes.iter())
            .any(|p| p.starts_with(rel) && p != rel)
    }

    /// Child names the deltas introduce directly under `rel` (e.g. a profile-only
    /// skill dir that exists in neither base nor source).
    fn names_under(&self, rel: &Path) -> Vec<std::ffi::OsString> {
        self.generated
            .keys()
            .chain(self.extra_links.keys())
            .filter_map(|p| p.strip_prefix(rel).ok())
            .filter_map(|rest| rest.components().next())
            .map(|c| c.as_os_str().to_os_string())
            .collect()
    }
}

/// The environment variable an agent honors to relocate its config dir.
///
/// Only agents with a single, well-defined redirect var can be profiled this
/// way. `None` means "profiling unsupported" and callers must refuse rather
/// than silently launch the un-profiled agent.
pub fn config_dir_env(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "pi" => Some("PI_CODING_AGENT_DIR"),
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        _ => None,
    }
}

/// Root of user-authored profile sources: `~/.config/workmux/agent-profiles/`.
pub fn source_root() -> Result<PathBuf> {
    Ok(crate::xdg::config_dir()?.join("agent-profiles"))
}

/// A single profile's user-authored source tree (may not exist; that just means
/// the overlay equals base).
pub fn source_dir(profile: &str) -> Result<PathBuf> {
    Ok(source_root()?.join(profile))
}

/// Root of derived overlay dirs: `~/.local/state/workmux/agent-profiles/`.
pub fn build_root() -> Result<PathBuf> {
    Ok(crate::xdg::state_dir()?.join("agent-profiles"))
}

/// The derived overlay dir for `(profile, agent)` — what the config-dir env var
/// points at.
pub fn build_dir(profile: &str, agent_id: &str) -> Result<PathBuf> {
    Ok(build_root()?.join(profile).join(agent_id))
}

#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    // No symlink privilege guarantee on Windows; fall back to a directory
    // junction-like copy is out of scope. Treat as unsupported.
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "agent profiles require symlink support",
    ))
}

/// Compute the desired overlay: relative path → plan entry.
///
/// Merge rule, applied recursively: when a name is a directory in *both* base
/// and source, recurse so deeper overrides compose; otherwise source wins if
/// present, else base. A name present in only one side is symlinked wholesale
/// (no need to recurse into a subtree with no overrides). Deltas layer on top:
/// excluded paths vanish, generated paths become regular files (unless the
/// source tree shadows them), extra links are planted, and any subtree a delta
/// path lies under is recursed into instead of wholesale-linked.
pub fn plan(base: &Path, source: &Path, delta: &DeltaPlan) -> Plan {
    let mut out = BTreeMap::new();
    plan_into(base, source, delta, Path::new(""), &mut out);
    out
}

/// The environment variable an agent honors to relocate its SESSION storage,
/// independent of its config dir.
///
/// A profile's derived overlay is reconstructable and gets wiped on every rebuild,
/// so session history cannot live inside it. Where an agent exposes a separate
/// session-dir var, point it at the persistent per-profile data tree instead, so
/// `corp` history is isolated from base without surviving only by luck. `None`
/// means the agent has no such knob and keeps sessions in its (already isolated)
/// config dir.
pub fn session_dir_env(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        // Pi's own precedence: --session-dir > PI_CODING_AGENT_SESSION_DIR > sessionDir setting.
        "pi" => Some("PI_CODING_AGENT_SESSION_DIR"),
        // claude keeps transcripts under CLAUDE_CONFIG_DIR; codex under CODEX_HOME.
        // Both are already redirected by config_dir_env, so there is nothing to add.
        _ => None,
    }
}

/// Root of persistent per-profile agent data: `~/.local/state/workmux/agent-profile-data/`.
///
/// Deliberately a sibling of `agent-profiles/` (the derived overlay root) rather than
/// a child: `materialize` removes the overlay wholesale, and anything the agent
/// *authors* — session transcripts above all — must outlive that.
pub fn data_root() -> Result<PathBuf> {
    Ok(crate::xdg::state_dir()?.join("agent-profile-data"))
}

/// Session storage for `(profile, agent)`, persistent across overlay rebuilds.
pub fn session_dir(profile: &str, agent_id: &str) -> Result<PathBuf> {
    Ok(data_root()?.join(profile).join(agent_id).join("sessions"))
}

fn dir_entries(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut names = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            names.push(entry.file_name());
        }
    }
    names
}

fn plan_into(base: &Path, source: &Path, delta: &DeltaPlan, rel: &Path, out: &mut Plan) {
    let base_here = base.join(rel);
    let source_here = source.join(rel);

    let mut names: Vec<std::ffi::OsString> = dir_entries(&base_here);
    for name in dir_entries(&source_here)
        .into_iter()
        .chain(delta.names_under(rel))
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.sort();

    for name in names {
        let child_rel = rel.join(&name);
        if delta.excludes.contains(&child_rel) {
            continue;
        }
        let bp = base.join(&child_rel);
        let sp = source.join(&child_rel);
        let s_exists = sp.symlink_metadata().is_ok();
        let b_exists = bp.symlink_metadata().is_ok();

        if let Some(content) = delta.generated.get(&child_rel) {
            // Source tree shadows generated content (user file beats deltas);
            // the shadow warning is emitted by the delta builder, which knows
            // the source dir.
            if s_exists {
                out.insert(child_rel, PlanEntry::Link(sp));
            } else {
                out.insert(child_rel, PlanEntry::File(content.clone()));
            }
            continue;
        }
        if let Some(target) = delta.extra_links.get(&child_rel) {
            let entry = if s_exists { sp } else { target.clone() };
            out.insert(child_rel, PlanEntry::Link(entry));
            continue;
        }

        if s_exists && b_exists && bp.is_dir() && sp.is_dir() {
            // Both directories — merge so a profile can override one entry
            // without shadowing its siblings.
            plan_into(base, source, delta, &child_rel, out);
        } else if s_exists {
            // Source wins (override, or source-only file/dir).
            out.insert(child_rel, PlanEntry::Link(sp));
        } else if b_exists {
            if bp.is_dir() && delta.touches_below(&child_rel) {
                // A delta lives inside this base-only subtree: recurse so the
                // delta applies while siblings stay linked.
                plan_into(base, source, delta, &child_rel, out);
            } else {
                out.insert(child_rel, PlanEntry::Link(bp));
            }
        } else if delta.touches_below(&child_rel) {
            // Neither side has it, but a delta plants something deeper
            // (e.g. skills/<profile-only-skill>): recurse to reach it.
            plan_into(base, source, delta, &child_rel, out);
        }
    }
}

/// Read the overlay currently on disk at `dest`: relative path → entry.
///
/// Symlinks are recorded as `Link`, regular files as `File` (their content —
/// only generated files exist as regular files in a derived tree); real
/// directories are structural and are recursed into.
pub fn read_current(dest: &Path) -> Plan {
    let mut out = BTreeMap::new();
    read_current_into(dest, Path::new(""), &mut out);
    out
}

fn read_current_into(dest: &Path, rel: &Path, out: &mut Plan) {
    let here = dest.join(rel);
    let Ok(rd) = std::fs::read_dir(&here) else {
        return;
    };
    for entry in rd.flatten() {
        let child_rel = rel.join(entry.file_name());
        let path = dest.join(&child_rel);
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            if let Ok(target) = std::fs::read_link(&path) {
                out.insert(child_rel, PlanEntry::Link(target));
            }
        } else if meta.is_dir() {
            read_current_into(dest, &child_rel, out);
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            out.insert(child_rel, PlanEntry::File(content));
        }
    }
}

/// Whether the derived dir already matches the desired plan.
pub fn in_sync(dest: &Path, plan: &Plan) -> bool {
    &read_current(dest) == plan
}

/// Rebuild the derived dir from the plan. Destructive to `dest` only (it is a
/// derived tree). Returns `true` if `dest` did not exist before (i.e. a fresh
/// install rather than an update).
pub fn materialize(dest: &Path, plan: &Plan) -> Result<bool> {
    let existed = dest.exists();
    if existed {
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("removing stale overlay dir {}", dest.display()))?;
    }
    std::fs::create_dir_all(dest)
        .with_context(|| format!("creating overlay dir {}", dest.display()))?;

    for (rel, entry) in plan {
        let link = dest.join(rel);
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        match entry {
            PlanEntry::Link(target) => make_symlink(target, &link)
                .with_context(|| format!("linking {} -> {}", link.display(), target.display()))?,
            PlanEntry::File(content) => std::fs::write(&link, content)
                .with_context(|| format!("writing generated {}", link.display()))?,
        }
    }
    Ok(!existed)
}

/// RFC 7386 merge patch: objects merge recursively, everything else replaces,
/// `null` deletes the key.
pub fn json_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    use serde_json::Value;
    if let Value::Object(pobj) = patch {
        if !target.is_object() {
            *target = Value::Object(Default::default());
        }
        let tobj = target.as_object_mut().expect("just ensured object");
        for (k, v) in pobj {
            if v.is_null() {
                tobj.remove(k);
            } else {
                json_merge_patch(tobj.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
    } else {
        *target = patch.clone();
    }
}

const SPEC_SCHEMES: [&str; 5] = ["npm:", "git:", "https:", "http:", "ssh:"];

/// Rewrite an added plugin spec so it resolves from the overlay dir: `~/` →
/// home, scheme-less relative → base agent dir (mirroring pi's own resolution
/// of relative settings entries), schemes and absolute paths verbatim.
fn absolutize_spec(spec: &str, base_agent_dir: &Path) -> String {
    if SPEC_SCHEMES.iter().any(|s| spec.starts_with(s)) || spec.starts_with('/') {
        return spec.to_string();
    }
    if let Some(rest) = spec.strip_prefix("~/") {
        if let Some(home) = home::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    normalize(&base_agent_dir.join(spec))
        .to_string_lossy()
        .into_owned()
}

/// Lexically resolve `.` and `..` components (no filesystem access).
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve an `additional_skills` source to an absolute path: `~/` → home,
/// relative → project root.
fn resolve_skill_source(src: &str, project_root: &Path) -> PathBuf {
    if let Some(rest) = src.strip_prefix("~/") {
        if let Some(home) = home::home_dir() {
            return home.join(rest);
        }
    }
    let p = Path::new(src);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        project_root.join(p)
    }
}

/// Generate the overlay `settings.json` for pi: base packages − excludes
/// (incl. excluded features' plugin specs) + absolutized additions, then the
/// RFC 7386 patch.
fn generate_pi_settings(
    base: &Path,
    deltas: &AgentProfileAgent,
    bootstrap: Option<&crate::bootstrap::BootstrapConfig>,
    warnings: &mut Vec<String>,
) -> Result<String> {
    use crate::agent::setup::Agent;
    use serde_json::Value;

    let body = std::fs::read_to_string(base.join("settings.json")).unwrap_or_else(|_| "{}".into());
    let mut json: Value =
        serde_json::from_str(&body).context("parsing base pi settings.json")?;

    let mut excluded: Vec<String> = deltas.exclude_plugins.clone();
    if let Some(bc) = bootstrap {
        for key in &deltas.exclude_features {
            match bc.features.get(key) {
                Some(f) => {
                    if let Some(spec) = f.plugin_for(Agent::Pi) {
                        excluded.push(spec.to_string());
                    }
                }
                None => warnings.push(format!("exclude_features: unknown feature `{key}`")),
            }
        }
    }

    let packages = json
        .as_object_mut()
        .context("base pi settings.json is not an object")?
        .entry("packages")
        .or_insert_with(|| Value::Array(vec![]));
    let arr = packages
        .as_array_mut()
        .context("base pi settings.json `packages` is not an array")?;

    for ex in &deltas.exclude_plugins {
        if !arr.iter().any(|v| v.as_str() == Some(ex)) {
            warnings.push(format!("exclude_plugins: `{ex}` matches no base packages entry"));
        }
    }
    arr.retain(|v| v.as_str().is_none_or(|s| !excluded.iter().any(|e| e == s)));

    // Base entries like `../../repos/...` resolve relative to the agent dir;
    // from the overlay dir they dangle, so rewrite them against base. Entries
    // inside the agent dir (`extensions/x.ts`) keep resolving through the
    // overlay's symlinks and stay as-is.
    for v in arr.iter_mut() {
        if let Some(s) = v.as_str() {
            if s.starts_with("../") {
                *v = Value::String(absolutize_spec(s, base));
            }
        }
    }

    for add in &deltas.additional_plugins {
        let spec = absolutize_spec(add, base);
        if let Some(name) = spec.strip_prefix("npm:") {
            if !base.join("npm/node_modules").join(name).exists() {
                warnings.push(format!(
                    "additional_plugins: `{spec}` not fetched under base npm tree; overlays install nothing"
                ));
            }
        } else if !spec.contains(':') && !Path::new(&spec).exists() {
            warnings.push(format!("additional_plugins: path `{spec}` does not exist"));
        }
        if !arr.iter().any(|v| v.as_str() == Some(spec.as_str())) {
            arr.push(Value::String(spec));
        }
    }

    if let Some(patch) = &deltas.settings {
        json_merge_patch(&mut json, patch);
    }

    Ok(format!("{}\n", serde_json::to_string_pretty(&json)?))
}

/// Render the overlay prompt file for pi from the profile-effective component
/// set. `None` when no prompt delta is declared (base file stays linked).
fn generate_pi_prompt(
    deltas: &AgentProfileAgent,
    bootstrap: &crate::bootstrap::BootstrapConfig,
    project_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<(PathBuf, String)>> {
    use crate::agent::setup::pi::PiInjectionMethod;
    use crate::agent::setup::Agent;

    let feature_components: Vec<&str> = deltas
        .exclude_features
        .iter()
        .filter_map(|k| bootstrap.features.get(k))
        .filter_map(|f| f.prompt_component_for(Agent::Pi))
        .collect();
    let has_delta = !deltas.additional_prompt_components.is_empty()
        || !deltas.exclude_prompt_components.is_empty()
        || !feature_components.is_empty();
    if !has_delta {
        return Ok(None);
    }

    let mut comps = bootstrap.prompt_components_for(Agent::Pi);
    for ex in &deltas.exclude_prompt_components {
        if !comps.contains(ex) {
            warnings.push(format!(
                "exclude_prompt_components: `{ex}` is not in the base component set"
            ));
        }
    }
    comps.retain(|c| {
        !deltas.exclude_prompt_components.contains(c)
            && !feature_components.iter().any(|f| f == c)
    });
    comps.extend(deltas.additional_prompt_components.iter().cloned());
    comps.sort();
    comps.dedup();

    let (prompt, _) = crate::bootstrap::merge_prompt_components("", &comps, project_root)?;
    let method = bootstrap
        .pi
        .as_ref()
        .map(|p| p.injection_method.clone())
        .unwrap_or_default();
    let rel = match method {
        PiInjectionMethod::AppendSystem => "APPEND_SYSTEM.md",
        PiInjectionMethod::BeforeAgentStart => "workmux-pre-inject.md",
    };
    Ok(Some((PathBuf::from(rel), format!("{}\n", prompt.trim()))))
}

/// Build the pi `DeltaPlan` for one profile: generated settings/prompt, skill
/// links and exclusions, raw path exclusions, plus all warnings — including a
/// shadow note for each generated file the profile source tree overrides.
pub fn pi_delta_plan(
    base: &Path,
    source: &Path,
    deltas: &AgentProfileAgent,
    bootstrap: Option<&crate::bootstrap::BootstrapConfig>,
    project_root: &Path,
) -> Result<DeltaPlan> {
    use crate::agent::setup::Agent;

    let mut dp = DeltaPlan::default();
    if deltas.is_empty() {
        return Ok(dp);
    }
    let mut warnings = Vec::new();

    let needs_settings = !deltas.additional_plugins.is_empty()
        || !deltas.exclude_plugins.is_empty()
        || deltas.settings.is_some()
        || deltas.exclude_features.iter().any(|k| {
            bootstrap
                .and_then(|bc| bc.features.get(k))
                .and_then(|f| f.plugin_for(Agent::Pi))
                .is_some()
        });
    if needs_settings {
        let content = generate_pi_settings(base, deltas, bootstrap, &mut warnings)?;
        dp.generated.insert(PathBuf::from("settings.json"), content);
    }

    match bootstrap {
        Some(bc) => {
            if let Some((rel, content)) = generate_pi_prompt(deltas, bc, project_root, &mut warnings)? {
                dp.generated.insert(rel, content);
            }
        }
        None if !deltas.additional_prompt_components.is_empty()
            || !deltas.exclude_prompt_components.is_empty() =>
        {
            warnings.push(
                "prompt component deltas declared but no bootstrap config loaded; prompt not re-rendered"
                    .into(),
            );
        }
        None => {}
    }

    for name in &deltas.exclude_skills {
        let rel = Path::new("skills").join(name);
        if !base.join(&rel).exists() {
            warnings.push(format!("exclude_skills: `{name}` is not installed in base"));
        }
        dp.excludes.insert(rel);
    }
    for src in &deltas.additional_skills {
        let abs = resolve_skill_source(src, project_root);
        if !abs.exists() {
            warnings.push(format!("additional_skills: `{}` does not exist", abs.display()));
        }
        let Some(name) = abs.file_name() else {
            warnings.push(format!("additional_skills: `{src}` has no basename"));
            continue;
        };
        dp.extra_links.insert(Path::new("skills").join(name), abs.clone());
    }

    for p in &deltas.exclude_paths {
        let rel = PathBuf::from(p);
        if rel.is_absolute() {
            warnings.push(format!("exclude_paths: `{p}` must be agent-dir-relative"));
            continue;
        }
        if !base.join(&rel).exists() {
            warnings.push(format!("exclude_paths: `{p}` does not exist in base"));
        }
        dp.excludes.insert(rel);
    }

    for rel in dp.generated.keys() {
        if source.join(rel).symlink_metadata().is_ok() {
            warnings.push(format!(
                "profile source tree shadows generated `{}`; declarative deltas for it are inert",
                rel.display()
            ));
        }
    }

    dp.warnings = warnings;
    Ok(dp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn link(p: &Path) -> PlanEntry {
        PlanEntry::Link(p.to_path_buf())
    }

    fn no_delta() -> DeltaPlan {
        DeltaPlan::default()
    }

    #[test]
    fn session_dir_env_is_pi_only_and_data_lives_outside_the_overlay() {
        assert_eq!(session_dir_env("pi"), Some("PI_CODING_AGENT_SESSION_DIR"));
        // These keep transcripts under their redirected config dir, so there is
        // nothing extra to point at.
        assert_eq!(session_dir_env("claude"), None);
        assert_eq!(session_dir_env("codex"), None);
        assert_eq!(session_dir_env("opencode"), None);
    }

    #[test]
    fn profile_session_dir_is_not_inside_the_derived_tree() {
        // materialize() removes the derived overlay wholesale on every rebuild, so
        // session history kept under build_root() would be destroyed by a setup run.
        let sessions = session_dir("corp", "pi").unwrap();
        assert!(sessions.ends_with("agent-profile-data/corp/pi/sessions"), "{sessions:?}");
        assert!(
            !sessions.starts_with(build_root().unwrap()),
            "{sessions:?} must not live under the derived overlay root"
        );
    }

    #[test]
    fn config_dir_env_known_and_unknown() {
        assert_eq!(config_dir_env("pi"), Some("PI_CODING_AGENT_DIR"));
        assert_eq!(config_dir_env("claude"), Some("CLAUDE_CONFIG_DIR"));
        assert_eq!(config_dir_env("opencode"), None);
    }

    #[test]
    fn plan_base_only_symlinks_whole_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        write(&base.join("skills/a/SKILL.md"), "a");
        write(&base.join("settings.json"), "{}");
        std::fs::create_dir_all(&source).unwrap();

        let p = plan(&base, &source, &no_delta());
        // No source → skills/ and settings.json symlinked at the top level.
        assert_eq!(p.get(Path::new("skills")), Some(&link(&base.join("skills"))));
        assert_eq!(
            p.get(Path::new("settings.json")),
            Some(&link(&base.join("settings.json")))
        );
    }

    #[test]
    fn plan_merges_dirs_and_source_wins_on_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        write(&base.join("skills/a/SKILL.md"), "base-a");
        write(&base.join("skills/b/SKILL.md"), "base-b");
        write(&source.join("skills/a/SKILL.md"), "override-a");
        write(&source.join("extensions/x.ts"), "x");

        let p = plan(&base, &source, &no_delta());
        // skills is a dir in both → merged: a overridden by source (recursed to
        // the file), b inherited wholesale from base.
        assert_eq!(
            p.get(Path::new("skills/a/SKILL.md")),
            Some(&link(&source.join("skills/a/SKILL.md"))),
            "profile overrides skill a"
        );
        assert_eq!(
            p.get(Path::new("skills/b")),
            Some(&link(&base.join("skills/b"))),
            "sibling skill b inherited from base"
        );
        // extensions exists only in source.
        assert_eq!(
            p.get(Path::new("extensions")),
            Some(&link(&source.join("extensions")))
        );
        // The merged dir itself is structural, not a link.
        assert!(!p.contains_key(Path::new("skills")));
        assert!(!p.contains_key(Path::new("skills/a")));
    }

    #[cfg(unix)]
    #[test]
    fn materialize_then_in_sync_and_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        let dest = tmp.path().join("dest");
        write(&base.join("skills/a/SKILL.md"), "base-a");
        write(&source.join("skills/a/SKILL.md"), "override-a");
        write(&source.join("extensions/x.ts"), "x");

        let p = plan(&base, &source, &no_delta());
        let fresh = materialize(&dest, &p).unwrap();
        assert!(fresh, "first build is an install");
        assert!(in_sync(&dest, &p), "just-built dir is in sync");

        // Overridden skill resolves through the symlink to source content.
        let a = std::fs::read_to_string(dest.join("skills/a/SKILL.md")).unwrap();
        assert_eq!(a, "override-a");

        // Rebuild is a no-op-equivalent (still in sync) and reports update.
        let fresh2 = materialize(&dest, &p).unwrap();
        assert!(!fresh2, "second build is an update, not install");
        assert!(in_sync(&dest, &p));
    }

    #[cfg(unix)]
    #[test]
    fn base_only_subtree_reflects_new_base_entries_without_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        let dest = tmp.path().join("dest");
        write(&base.join("skills/a/SKILL.md"), "a");
        std::fs::create_dir_all(&source).unwrap();

        let p1 = plan(&base, &source, &no_delta());
        materialize(&dest, &p1).unwrap();
        assert!(in_sync(&dest, &p1));

        // Because skills/ is symlinked wholesale, a new base skill is visible
        // through the overlay with no rebuild and is not drift.
        write(&base.join("skills/c/SKILL.md"), "c");
        let p2 = plan(&base, &source, &no_delta());
        assert!(in_sync(&dest, &p2), "wholesale symlink transparently follows base");
        assert!(dest.join("skills/c/SKILL.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn drift_detected_when_profile_adds_override() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        let dest = tmp.path().join("dest");
        write(&base.join("skills/a/SKILL.md"), "a");
        std::fs::create_dir_all(&source).unwrap();

        let p1 = plan(&base, &source, &no_delta());
        materialize(&dest, &p1).unwrap();
        assert!(in_sync(&dest, &p1));

        // User authors a profile override → the merge structure changes → drift.
        write(&source.join("skills/a/SKILL.md"), "override");
        let p2 = plan(&base, &source, &no_delta());
        assert!(!in_sync(&dest, &p2), "new source override is drift");

        materialize(&dest, &p2).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("skills/a/SKILL.md")).unwrap(),
            "override"
        );
    }

    // --- delta planning ---

    fn deltas(f: impl FnOnce(&mut AgentProfileAgent)) -> AgentProfileAgent {
        let mut d = AgentProfileAgent::default();
        f(&mut d);
        d
    }

    #[test]
    fn json_merge_patch_rfc7386() {
        let mut v = serde_json::json!({"a": {"b": 1, "c": 2}, "d": 3});
        json_merge_patch(
            &mut v,
            &serde_json::json!({"a": {"b": null, "e": 4}, "d": [5]}),
        );
        assert_eq!(v, serde_json::json!({"a": {"c": 2, "e": 4}, "d": [5]}));
    }

    #[test]
    fn absolutize_spec_forms() {
        let base = Path::new("/base/agent");
        assert_eq!(absolutize_spec("npm:foo", base), "npm:foo");
        assert_eq!(absolutize_spec("git:github.com/x/y", base), "git:github.com/x/y");
        assert_eq!(absolutize_spec("/abs/path", base), "/abs/path");
        assert_eq!(absolutize_spec("../x", base), "/base/x");
        let home = home::home_dir().unwrap();
        assert_eq!(
            absolutize_spec("~/x", base),
            home.join("x").to_string_lossy()
        );
    }

    #[test]
    fn settings_generated_with_package_delta_and_patch() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        write(
            &base.join("settings.json"),
            r#"{"packages":["npm:pi-cliproxyapi","npm:keep","../../escape/pkg","extensions/in.ts"],"defaultProvider":"anthropic","taskflow":{"x":1}}"#,
        );
        let ext = tmp.path().join("litellm");
        std::fs::create_dir_all(&ext).unwrap();

        let d = deltas(|d| {
            d.additional_plugins = vec![ext.to_string_lossy().into_owned()];
            d.exclude_plugins = vec!["npm:pi-cliproxyapi".into()];
            d.settings = Some(serde_json::json!({"defaultProvider": "litellm", "taskflow": null}));
        });
        let dp = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        assert!(dp.warnings.is_empty(), "warnings: {:?}", dp.warnings);

        let content = &dp.generated[Path::new("settings.json")];
        let v: serde_json::Value = serde_json::from_str(content).unwrap();
        let pkgs: Vec<&str> = v["packages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        let escaped = normalize(&base.join("../../escape/pkg"));
        assert_eq!(
            pkgs,
            vec![
                "npm:keep",
                escaped.to_str().unwrap(),
                "extensions/in.ts",
                ext.to_str().unwrap()
            ],
            "escaping relative base entry absolutized, in-dir entry untouched"
        );
        assert_eq!(v["defaultProvider"], "litellm");
        assert!(v.get("taskflow").is_none(), "null patch deletes key");

        // Overlay: settings.json generated, siblings still linked.
        let p = plan(&base, &source, &dp);
        assert!(matches!(p.get(Path::new("settings.json")), Some(PlanEntry::File(_))));
    }

    #[test]
    fn unmatched_exclude_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        write(&base.join("settings.json"), r#"{"packages":["npm:real"]}"#);

        let d = deltas(|d| d.exclude_plugins = vec!["npm:typo".into()]);
        let dp = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        assert!(dp.warnings.iter().any(|w| w.contains("npm:typo")));
    }

    #[cfg(unix)]
    #[test]
    fn skills_delta_excludes_and_adds_per_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        write(&base.join("skills/keep/SKILL.md"), "keep");
        write(&base.join("skills/drop/SKILL.md"), "drop");
        let extra = tmp.path().join("proj/skills/audit");
        write(&extra.join("SKILL.md"), "audit");

        let d = deltas(|d| {
            d.exclude_skills = vec!["drop".into()];
            d.additional_skills = vec![extra.to_string_lossy().into_owned()];
        });
        let dp = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        let p = plan(&base, &source, &dp);

        assert_eq!(
            p.get(Path::new("skills/keep")),
            Some(&link(&base.join("skills/keep"))),
            "sibling skill stays linked"
        );
        assert!(!p.contains_key(Path::new("skills/drop")));
        assert_eq!(p.get(Path::new("skills/audit")), Some(&link(&extra)));

        let dest = tmp.path().join("dest");
        materialize(&dest, &p).unwrap();
        assert!(in_sync(&dest, &p));
        assert!(dest.join("skills/keep/SKILL.md").exists());
        assert!(!dest.join("skills/drop").exists());
        assert!(dest.join("skills/audit/SKILL.md").exists());
    }

    #[test]
    fn exclude_paths_recurses_shared_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        write(&base.join("extensions/keep.ts"), "k");
        write(&base.join("extensions/drop.ts"), "d");

        let d = deltas(|d| d.exclude_paths = vec!["extensions/drop.ts".into()]);
        let dp = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        let p = plan(&base, &source, &dp);
        assert_eq!(
            p.get(Path::new("extensions/keep.ts")),
            Some(&link(&base.join("extensions/keep.ts")))
        );
        assert!(!p.contains_key(Path::new("extensions/drop.ts")));
        assert!(!p.contains_key(Path::new("extensions")), "recursed, not wholesale");
    }

    #[test]
    fn source_tree_shadows_generated_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        write(&base.join("settings.json"), r#"{"packages":[]}"#);
        write(&source.join("settings.json"), r#"{"hand":"authored"}"#);

        let d = deltas(|d| d.settings = Some(serde_json::json!({"x": 1})));
        let dp = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        assert!(dp.warnings.iter().any(|w| w.contains("shadows")));

        let p = plan(&base, &source, &dp);
        assert_eq!(
            p.get(Path::new("settings.json")),
            Some(&link(&source.join("settings.json"))),
            "user file wins over generated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn generated_file_drift_on_base_change() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let source = tmp.path().join("source");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&source).unwrap();
        write(&base.join("settings.json"), r#"{"packages":["npm:a"]}"#);

        let d = deltas(|d| d.exclude_plugins = vec!["npm:a".into()]);
        let dp1 = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        let p1 = plan(&base, &source, &dp1);
        materialize(&dest, &p1).unwrap();
        assert!(in_sync(&dest, &p1));

        // Base grows a package → generated content changes → drift.
        write(&base.join("settings.json"), r#"{"packages":["npm:a","npm:b"]}"#);
        let dp2 = pi_delta_plan(&base, &source, &d, None, tmp.path()).unwrap();
        let p2 = plan(&base, &source, &dp2);
        assert!(!in_sync(&dest, &p2), "base package change is drift");
        materialize(&dest, &p2).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dest.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(v["packages"], serde_json::json!(["npm:b"]));
    }
}

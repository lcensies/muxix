//! Keeping muxix's project registry and an ADE's project list in agreement.
//!
//! ADEs track projects too, so a directory added in one tool should not have to
//! be added again in the other. Identity is the **canonical root path**, never
//! the display name: both tools let you name a project whatever you like, and
//! the same directory under two names is one project.
//!
//! Everything destructive is off by default. Sync is a convenience, and a
//! convenience that deletes a project entry because another tool forgot about it
//! is a bug with no upside. Removals therefore require both an explicit opt-in
//! *and* a recorded last-synced set — "absent over there" alone can never be
//! told apart from "newly added over here".

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{AdeConfig, AdeProjectsConfig, SyncDirection};
use crate::projects::registry::{ProjectEntry, Registry};

/// A project as one side of the sync sees it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SyncProject {
    pub root: PathBuf,
    pub name: String,
}

/// What a sync would do. Produced without touching either side, so `--dry-run`
/// and the real thing share one code path.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub add_to_muxix: Vec<SyncProject>,
    pub add_to_ade: Vec<SyncProject>,
    pub remove_from_muxix: Vec<SyncProject>,
    pub remove_from_ade: Vec<SyncProject>,
    /// Projects present on both sides whose names differ, when name
    /// propagation is off or the policy is manual.
    pub conflicts: Vec<(SyncProject, SyncProject)>,
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.add_to_muxix.is_empty()
            && self.add_to_ade.is_empty()
            && self.remove_from_muxix.is_empty()
            && self.remove_from_ade.is_empty()
            && self.conflicts.is_empty()
    }
}

/// The set of roots recorded at the last successful sync, per ADE.
///
/// Only consulted when removal propagation is on: without it, a project missing
/// from one side is indistinguishable from a new one on the other.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LastSynced {
    #[serde(default)]
    pub roots: BTreeSet<PathBuf>,
}

impl LastSynced {
    fn path(ade: &str) -> Result<PathBuf> {
        Ok(crate::xdg::state_dir()?.join(format!("sync-{ade}.json")))
    }

    pub fn load(ade: &str) -> Self {
        Self::path(ade)
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, ade: &str) -> Result<()> {
        let path = Self::path(ade)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::util::write_atomic(&path, &serde_json::to_string(self)?)
    }
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Build the plan. Pure: no I/O, so every rule is directly testable.
pub fn plan(
    muxix: &[SyncProject],
    ade: &[SyncProject],
    cfg: &crate::config::ProjectSyncConfig,
    last: &LastSynced,
) -> SyncPlan {
    let mut plan = SyncPlan::default();
    if cfg.direction == SyncDirection::Off {
        return plan;
    }
    let pull = matches!(
        cfg.direction,
        SyncDirection::Pull | SyncDirection::Bidirectional
    );
    let push = matches!(
        cfg.direction,
        SyncDirection::Push | SyncDirection::Bidirectional
    );

    let wm_roots: BTreeSet<PathBuf> = muxix.iter().map(|p| canon(&p.root)).collect();
    let ade_roots: BTreeSet<PathBuf> = ade.iter().map(|p| canon(&p.root)).collect();

    for p in ade {
        let root = canon(&p.root);
        if wm_roots.contains(&root) {
            continue;
        }
        // Known at last sync and now gone from muxix → a removal to mirror,
        // not an addition to replay. Without the record it is an addition.
        if cfg.removals && last.roots.contains(&root) {
            if push {
                plan.remove_from_ade.push(p.clone());
            }
        } else if pull {
            plan.add_to_muxix.push(p.clone());
        }
    }

    for p in muxix {
        let root = canon(&p.root);
        if ade_roots.contains(&root) {
            continue;
        }
        if cfg.removals && last.roots.contains(&root) {
            if pull {
                plan.remove_from_muxix.push(p.clone());
            }
        } else if push {
            plan.add_to_ade.push(p.clone());
        }
    }

    // Same root, different names: not two projects, and not a conflict unless
    // the user asked for names to travel.
    if cfg.names {
        for w in muxix {
            let root = canon(&w.root);
            if let Some(a) = ade.iter().find(|a| canon(&a.root) == root)
                && a.name != w.name
            {
                plan.conflicts.push((w.clone(), a.clone()));
            }
        }
    }

    plan
}

/// A tool that tracks projects.
///
/// Muxix implements this for its own registry rather than being special-cased
/// on one side of the sync: it has project management, a daemon, and agent
/// management, which is exactly what makes something an ADE. Keeping it under
/// the same interface means the sync has no privileged side, and syncing two
/// external managers to each other needs no new code.
pub trait ProjectRegistrySource {
    /// Registry name, used in messages and in the last-synced record.
    fn name(&self) -> &str;
    fn list(&self) -> Result<Vec<SyncProject>>;
    fn add(&self, p: &SyncProject) -> Result<()>;
    fn remove(&self, p: &SyncProject) -> Result<()>;
}

/// Muxix's own project registry (`~/.config/muxix/projects.yaml`).
pub struct MuxixProjects;

impl ProjectRegistrySource for MuxixProjects {
    fn name(&self) -> &str {
        "muxix"
    }

    fn list(&self) -> Result<Vec<SyncProject>> {
        Ok(Registry::load()?
            .projects
            .iter()
            .map(SyncProject::from)
            .collect())
    }

    fn add(&self, p: &SyncProject) -> Result<()> {
        let mut reg = Registry::load()?;
        reg.add(&p.root)?;
        Ok(())
    }

    fn remove(&self, p: &SyncProject) -> Result<()> {
        let mut reg = Registry::load()?;
        reg.remove(&p.name)?;
        Ok(())
    }
}

/// An external ADE's project list, driven through its CLI.
pub struct AdeProjects<'a> {
    pub name: &'a str,
    pub command: &'a str,
    pub cfg: &'a AdeProjectsConfig,
}

impl<'a> AdeProjects<'a> {
    fn run(&self, template: &[String], p: &SyncProject) -> Result<()> {
        if template.is_empty() {
            bail!("{} has no command configured for this operation", self.name);
        }
        let args: Vec<String> = template
            .iter()
            .map(|a| {
                a.replace("{root}", &p.root.to_string_lossy())
                    .replace("{name}", &p.name)
            })
            .collect();
        let out = Command::new(self.command).args(&args).output()?;
        if !out.status.success() {
            bail!(
                "{} {}: {}",
                self.command,
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

impl<'a> ProjectRegistrySource for AdeProjects<'a> {
    fn name(&self) -> &str {
        self.name
    }

    fn list(&self) -> Result<Vec<SyncProject>> {
        let out = Command::new(self.command)
            .args(&self.cfg.list_args)
            .output()
            .with_context(|| format!("running {}", self.command))?;
        if !out.status.success() {
            bail!(
                "{} {}: {}",
                self.command,
                self.cfg.list_args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Value = serde_json::from_str(text.trim())
            .with_context(|| format!("unparseable project list from {}", self.command))?;
        Ok(parse_projects(
            &v,
            &self.cfg.root_field,
            &self.cfg.name_field,
        ))
    }

    fn add(&self, p: &SyncProject) -> Result<()> {
        self.run(&self.cfg.add_args, p)
    }

    fn remove(&self, p: &SyncProject) -> Result<()> {
        self.run(&self.cfg.remove_args, p)
    }
}

/// Extract projects from a manager's JSON, accepting a bare array or an object
/// wrapping one — managers differ and both shapes are common.
pub fn parse_projects(v: &Value, root_field: &str, name_field: &str) -> Vec<SyncProject> {
    let items = match v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .values()
            .find_map(|x| x.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let root = item.get(root_field)?.as_str()?;
            let name = item
                .get(name_field)
                .and_then(|x| x.as_str())
                .unwrap_or(root);
            Some(SyncProject {
                root: PathBuf::from(root),
                name: name.to_string(),
            })
        })
        .collect()
}

/// Apply a plan between two sources. Neither side is privileged.
fn apply(plan: &SyncPlan, muxix: &dyn ProjectRegistrySource, ade: &dyn ProjectRegistrySource) {
    for (target, adds, removes) in [
        (muxix, &plan.add_to_muxix, &plan.remove_from_muxix),
        (ade, &plan.add_to_ade, &plan.remove_from_ade),
    ] {
        for p in adds {
            if let Err(e) = target.add(p) {
                eprintln!(
                    "sync: could not add {} to {}: {e}",
                    p.root.display(),
                    target.name()
                );
            }
        }
        for p in removes {
            if let Err(e) = target.remove(p) {
                eprintln!(
                    "sync: could not remove {} from {}: {e}",
                    p.name,
                    target.name()
                );
            }
        }
    }
}

/// Sync one ADE. Returns the plan that was applied (or would be, on dry run).
pub fn sync_one(ade_name: &str, ade: &AdeConfig, dry_run: bool) -> Result<SyncPlan> {
    let Some(pcfg) = &ade.projects else {
        // A manager with no project concept is never a sync target.
        return Ok(SyncPlan::default());
    };
    if pcfg.sync.direction == SyncDirection::Off {
        return Ok(SyncPlan::default());
    }

    let muxix = MuxixProjects;
    let other = AdeProjects {
        name: ade_name,
        command: &ade.command,
        cfg: pcfg,
    };
    let last = LastSynced::load(ade_name);
    let plan = plan(&muxix.list()?, &other.list()?, &pcfg.sync, &last);

    if dry_run || plan.is_empty() {
        return Ok(plan);
    }
    apply(&plan, &muxix, &other);

    // Record what both sides hold now, so a later removal can be told apart
    // from a later addition.
    let after: BTreeSet<PathBuf> = muxix
        .list()
        .unwrap_or_default()
        .iter()
        .map(|p| canon(&p.root))
        .chain(
            other
                .list()
                .unwrap_or_default()
                .iter()
                .map(|p| canon(&p.root)),
        )
        .collect();
    LastSynced { roots: after }.save(ade_name)?;

    Ok(plan)
}

/// `muxix project sync [--dry-run] [--ade <name>]`.
pub fn cli_sync(only: Option<&str>, dry_run: bool) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let ades = effective_ades(&cfg);
    if ades.is_empty() {
        println!("no ADEs configured");
        return Ok(());
    }

    let mut any = false;
    for (name, ade) in &ades {
        if only.is_some_and(|o| o != name) {
            continue;
        }
        any = true;
        let plan = match sync_one(name, ade, dry_run) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{name}: sync failed: {e}");
                continue;
            }
        };
        if plan.is_empty() {
            println!("{name}: nothing to do");
            continue;
        }
        let verb = if dry_run { "would " } else { "" };
        for p in &plan.add_to_muxix {
            println!("{name}: {verb}track {}", p.root.display());
        }
        for p in &plan.add_to_ade {
            println!("{name}: {verb}add {} to {name}", p.root.display());
        }
        for p in &plan.remove_from_muxix {
            println!("{name}: {verb}untrack {}", p.root.display());
        }
        for p in &plan.remove_from_ade {
            println!("{name}: {verb}remove {} from {name}", p.root.display());
        }
        for (w, a) in &plan.conflicts {
            println!(
                "{name}: name conflict for {}: muxix '{}' vs {name} '{}'",
                w.root.display(),
                w.name,
                a.name
            );
        }
    }
    if only.is_some() && !any {
        bail!("no ADE named '{}' is configured", only.unwrap_or_default());
    }
    Ok(())
}

/// Configured ADEs: built-in presets with the user's config merged over them.
pub fn effective_ades(
    cfg: &crate::config::Config,
) -> std::collections::BTreeMap<String, AdeConfig> {
    let presets = AdeConfig::presets();
    let mut map = presets.clone();
    for (name, ade) in &cfg.ade {
        // A user block tweaks a preset rather than replacing it; a block for an
        // unknown ADE stands on its own.
        let merged = match presets.get(name) {
            Some(preset) => ade.clone().merge_over(preset),
            None => ade.clone(),
        };
        map.insert(name.clone(), merged);
    }
    map
}

/// Run sync for every configured ADE, swallowing errors. Called from the daemon
/// tick: a sync failure must never abort the tick or block dispatch.
#[allow(dead_code)]
pub fn sync_all_quiet() {
    let cfg = match crate::config::Config::load(None) {
        Ok(c) => c,
        Err(_) => return,
    };
    for (name, ade) in effective_ades(&cfg) {
        let enabled = ade
            .projects
            .as_ref()
            .is_some_and(|p| p.sync.direction != SyncDirection::Off);
        if !enabled {
            continue;
        }
        if let Err(e) = sync_one(&name, &ade, false) {
            eprintln!("sync ({name}): {e}");
        }
    }
}

/// Convenience for callers that hold entries rather than sync projects.
impl From<&ProjectEntry> for SyncProject {
    fn from(e: &ProjectEntry) -> Self {
        SyncProject {
            root: e.root.clone(),
            name: e.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProjectSyncConfig, SyncDirection};

    fn p(root: &str, name: &str) -> SyncProject {
        SyncProject {
            root: PathBuf::from(root),
            name: name.to_string(),
        }
    }

    fn cfg(direction: SyncDirection) -> ProjectSyncConfig {
        ProjectSyncConfig {
            direction,
            ..Default::default()
        }
    }

    #[test]
    fn off_does_nothing_at_all() {
        let plan = plan(
            &[p("/a", "a")],
            &[p("/b", "b")],
            &cfg(SyncDirection::Off),
            &LastSynced::default(),
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn pull_only_touches_muxix() {
        let plan = plan(
            &[p("/a", "a")],
            &[p("/b", "b")],
            &cfg(SyncDirection::Pull),
            &LastSynced::default(),
        );
        assert_eq!(plan.add_to_muxix, vec![p("/b", "b")]);
        assert!(plan.add_to_ade.is_empty());
    }

    #[test]
    fn push_only_touches_the_ade() {
        let plan = plan(
            &[p("/a", "a")],
            &[p("/b", "b")],
            &cfg(SyncDirection::Push),
            &LastSynced::default(),
        );
        assert_eq!(plan.add_to_ade, vec![p("/a", "a")]);
        assert!(plan.add_to_muxix.is_empty());
    }

    #[test]
    fn bidirectional_is_a_union() {
        let plan = plan(
            &[p("/a", "a")],
            &[p("/b", "b")],
            &cfg(SyncDirection::Bidirectional),
            &LastSynced::default(),
        );
        assert_eq!(plan.add_to_muxix, vec![p("/b", "b")]);
        assert_eq!(plan.add_to_ade, vec![p("/a", "a")]);
    }

    #[test]
    fn a_synced_union_is_idempotent() {
        let both = [p("/a", "a"), p("/b", "b")];
        let plan = plan(
            &both,
            &both,
            &cfg(SyncDirection::Bidirectional),
            &LastSynced::default(),
        );
        assert!(plan.is_empty(), "{plan:?}");
    }

    #[test]
    fn names_are_not_identity() {
        // Same directory, different display names: one project, no conflict.
        let plan = plan(
            &[p("/a", "api")],
            &[p("/a", "backend")],
            &cfg(SyncDirection::Bidirectional),
            &LastSynced::default(),
        );
        assert!(plan.is_empty(), "{plan:?}");
    }

    #[test]
    fn name_differences_surface_only_when_names_are_enabled() {
        let c = ProjectSyncConfig {
            direction: SyncDirection::Bidirectional,
            names: true,
            ..Default::default()
        };
        let plan = plan(
            &[p("/a", "api")],
            &[p("/a", "backend")],
            &c,
            &LastSynced::default(),
        );
        assert_eq!(plan.conflicts.len(), 1);
    }

    #[test]
    fn removals_do_not_propagate_by_default() {
        // /a is gone from the ADE, but removals are off: it stays tracked.
        let last = LastSynced {
            roots: BTreeSet::from([PathBuf::from("/a")]),
        };
        let plan = plan(
            &[p("/a", "a")],
            &[],
            &cfg(SyncDirection::Bidirectional),
            &last,
        );
        assert!(plan.remove_from_muxix.is_empty());
        assert_eq!(plan.add_to_ade, vec![p("/a", "a")]);
    }

    #[test]
    fn removals_propagate_only_for_previously_synced_projects() {
        let c = ProjectSyncConfig {
            direction: SyncDirection::Bidirectional,
            removals: true,
            ..Default::default()
        };
        let last = LastSynced {
            roots: BTreeSet::from([PathBuf::from("/a")]),
        };
        let plan = plan(&[p("/a", "a")], &[], &c, &last);
        assert_eq!(plan.remove_from_muxix, vec![p("/a", "a")]);
        assert!(plan.add_to_ade.is_empty());
    }

    #[test]
    fn a_never_synced_project_is_an_addition_not_a_removal() {
        // Enabling removals must not retroactively delete what predates it.
        let c = ProjectSyncConfig {
            direction: SyncDirection::Bidirectional,
            removals: true,
            ..Default::default()
        };
        let plan = plan(&[p("/new", "new")], &[], &c, &LastSynced::default());
        assert!(plan.remove_from_muxix.is_empty());
        assert_eq!(plan.add_to_ade, vec![p("/new", "new")]);
    }

    #[test]
    fn one_sided_removal_does_not_loop_back_as_an_addition() {
        // The failure mode of naive bidirectional sync: A removes, B re-adds,
        // forever. With the last-synced record the removal wins once.
        let c = ProjectSyncConfig {
            direction: SyncDirection::Bidirectional,
            removals: true,
            ..Default::default()
        };
        let last = LastSynced {
            roots: BTreeSet::from([PathBuf::from("/a")]),
        };
        let plan = plan(&[], &[p("/a", "a")], &c, &last);
        assert_eq!(plan.remove_from_ade, vec![p("/a", "a")]);
        assert!(plan.add_to_muxix.is_empty());
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::config::{AdeConfig, AdeProjectsConfig, Config, ProjectSyncConfig};

    #[test]
    fn a_policy_only_block_keeps_the_presets_commands() {
        // Regression: replacing instead of merging blanked out the preset's
        // command lists, leaving the ADE looking broken with no visible cause.
        // This is exactly the shape of block the docs tell people to write.
        let mut cfg = Config::default();
        let mut user_block = AdeConfig::paseo();
        user_block.start_args.clear();
        user_block.send_args.clear();
        user_block.list_args.clear();
        user_block.stop_args.clear();
        user_block.import_args.clear();
        user_block.projects = Some(AdeProjectsConfig {
            sync: ProjectSyncConfig {
                direction: SyncDirection::Bidirectional,
                ..Default::default()
            },
            ..Default::default()
        });
        cfg.ade.insert("paseo".to_string(), user_block);

        let merged = effective_ades(&cfg);
        let paseo = merged.get("paseo").expect("preset must survive");
        assert!(!paseo.list_args.is_empty(), "agent commands must survive");
        assert!(!paseo.start_args.is_empty(), "agent commands must survive");
        let projects = paseo.projects.as_ref().unwrap();
        assert!(
            !projects.list_args.is_empty(),
            "project commands must survive"
        );
        assert_eq!(projects.sync.direction, SyncDirection::Bidirectional);
    }

    #[test]
    fn an_unknown_ade_stands_on_its_own() {
        let mut cfg = Config::default();
        let mut custom = AdeConfig::paseo();
        custom.command = "other".to_string();
        cfg.ade.insert("other".to_string(), custom);
        let merged = effective_ades(&cfg);
        assert_eq!(merged.get("other").unwrap().command, "other");
        assert!(merged.contains_key("paseo"), "presets stay available");
    }
}

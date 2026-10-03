//! Provenance for the harness features `muxix setup` installs.
//!
//! Every other section decides "is this installed?" by probing the agent's own
//! config, which cannot tell *who* installed a thing. That is fine for
//! converging toward the config, but it makes the reverse direction —
//! removing what the config no longer declares — unsafe: a skill directory the
//! user wrote by hand is indistinguishable from one muxix copied there.
//!
//! This manifest is that missing half. It records what muxix installed, for
//! which project, and how to remove it; a feature absent from the manifest is
//! never touched, so the worst a bug here can do is fail to clean up.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::report::{ItemResult, Outcome, Section};
use crate::agent::setup::Agent;

/// One harness feature muxix installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedEntry {
    pub section: Section,
    /// Agent display name (`Agent::name`), or `None` for agent-independent work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// How the item is reported: a skill name, a plugin spec, `skill:event`.
    pub name: String,
    /// Project root that declared it. Pruning is scoped to this.
    pub project: PathBuf,
    /// What to remove: a filesystem path, a plugin spec, a hook command.
    /// Section-dependent; see [`remove_one`].
    pub target: String,
}

/// Identity of a feature, independent of which project declared it.
type Key = (Section, Option<String>, String);

impl ManagedEntry {
    fn key(&self) -> Key {
        (self.section, self.agent.clone(), self.name.clone())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ManagedEntry>,
}

const VERSION: u32 = 1;

pub fn manifest_path() -> anyhow::Result<PathBuf> {
    Ok(crate::xdg::state_dir()?.join("managed.json"))
}

impl Manifest {
    /// Load the manifest, or an empty one when it is missing or unreadable.
    ///
    /// An unparsable manifest degrades to "muxix has installed nothing":
    /// pruning stops until the next run rebuilds it, which is the safe
    /// direction to fail in.
    pub fn load() -> Manifest {
        let Ok(path) = manifest_path() else {
            return Manifest::default();
        };
        Manifest::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Manifest {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&manifest_path()?)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut doc = self.clone();
        doc.version = VERSION;
        doc.entries.sort_by(|a, b| {
            (a.section.as_str(), &a.agent, &a.name, &a.project).cmp(&(
                b.section.as_str(),
                &b.agent,
                &b.name,
                &b.project,
            ))
        });
        crate::state::store::write_atomic(path, serde_json::to_string_pretty(&doc)?.as_bytes())
    }

    /// Entries this project installed that the current run did not, restricted
    /// to sections the run actually covered.
    ///
    /// An entry another project still claims is kept: agent config dirs are
    /// global, so the last declaring project — not the first one to run setup —
    /// decides when a shared feature goes away.
    pub fn stale_for(
        &self,
        project: &Path,
        installed: &BTreeSet<Key>,
        sections: &[Section],
    ) -> Vec<ManagedEntry> {
        let claimed_elsewhere: BTreeSet<Key> = self
            .entries
            .iter()
            .filter(|e| e.project != project)
            .map(ManagedEntry::key)
            .collect();

        self.entries
            .iter()
            .filter(|e| e.project == project)
            .filter(|e| sections.contains(&e.section))
            .filter(|e| !installed.contains(&e.key()))
            .filter(|e| !claimed_elsewhere.contains(&e.key()))
            .cloned()
            .collect()
    }

    /// Replace this project's entries for `sections` with `current`, leaving
    /// every other project's and every unrun section's entries alone.
    pub fn record(
        &mut self,
        project: &Path,
        sections: &[Section],
        current: Vec<ManagedEntry>,
        keep: &[ManagedEntry],
    ) {
        self.entries
            .retain(|e| !(e.project == project && sections.contains(&e.section)));
        self.entries.extend(current);
        for entry in keep {
            if !self.entries.contains(entry) {
                self.entries.push(entry.clone());
            }
        }
    }
}

/// The managed entries a finished report implies, for `project`.
///
/// Only converged items count: a failed or skipped install did not put
/// anything on the machine, so recording it would make the next run try to
/// remove something that was never there.
pub fn entries_from_report(items: &[ItemResult], project: &Path) -> Vec<ManagedEntry> {
    items
        .iter()
        .filter(|i| {
            matches!(
                i.outcome,
                Outcome::Installed | Outcome::Updated | Outcome::UpToDate
            )
        })
        .filter_map(|i| {
            Some(ManagedEntry {
                section: i.section,
                agent: i.agent.clone(),
                name: i.name.clone(),
                project: project.to_path_buf(),
                target: i.managed.clone()?,
            })
        })
        .collect()
}

fn agent_from_display(name: &str) -> Option<Agent> {
    Agent::ALL.into_iter().find(|a| a.name() == name)
}

/// Remove every stale feature, reporting one item each.
pub fn prune(stale: &[ManagedEntry], dry_run: bool) -> Vec<ItemResult> {
    stale
        .iter()
        .filter_map(|entry| {
            let item = ItemResult::new(
                entry.section,
                entry.agent.as_deref(),
                entry.name.clone(),
                Outcome::Removed,
            );
            if dry_run {
                return Some(
                    item.with_detail(format!("no longer declared; would remove {}", entry.target)),
                );
            }
            match remove_one(entry) {
                Ok(Some(detail)) => Some(item.with_detail(detail)),
                // The section has no removal path; leave the entry and say nothing.
                Ok(None) => None,
                Err(e) => Some(ItemResult::failed(
                    entry.section,
                    entry.agent.as_deref(),
                    entry.name.clone(),
                    format!("could not remove: {e}"),
                )),
            }
        })
        .collect()
}

/// Remove one feature. `Ok(None)` means this section is not prunable.
fn remove_one(entry: &ManagedEntry) -> anyhow::Result<Option<String>> {
    let target = Path::new(&entry.target);
    match entry.section {
        Section::Skills => {
            // An already-absent target is convergence, not an error.
            if target.exists() {
                std::fs::remove_dir_all(target)?;
            }
            Ok(Some(format!("removed {}", target.display())))
        }
        Section::Subagents => {
            if target.exists() {
                std::fs::remove_file(target)?;
            }
            Ok(Some(format!("removed {}", target.display())))
        }
        Section::Plugins => {
            let agent = entry
                .agent
                .as_deref()
                .and_then(agent_from_display)
                .ok_or_else(|| anyhow::anyhow!("unknown agent for plugin entry"))?;
            uninstall_plugin(agent, &entry.target).map(Some)
        }
        Section::AgentHooks => {
            let agent = entry
                .agent
                .as_deref()
                .and_then(agent_from_display)
                .ok_or_else(|| anyhow::anyhow!("unknown agent for hook entry"))?;
            super::agent_hooks::remove_command(agent, &entry.target).map(Some)
        }
        Section::Deps => {
            let (prefix, name) = crate::deps::split_package_dir(&entry.target)
                .ok_or_else(|| anyhow::anyhow!("not an npm package dir: {}", entry.target))?;
            if target.exists() {
                crate::deps::npm_uninstall(&prefix, &name)?;
            }
            Ok(Some(format!(
                "npm uninstall {name} (prefix {})",
                prefix.display()
            )))
        }
        _ => Ok(None),
    }
}

/// `deps_strict`: every package under the npm prefix that no entity declares,
/// as synthetic stale entries so [`prune`] handles them like managed ones.
pub fn undeclared_npm(
    prefix: &Path,
    declared: &BTreeSet<String>,
    project: &Path,
) -> Vec<ManagedEntry> {
    crate::deps::installed_packages(prefix)
        .into_iter()
        .filter(|name| !declared.contains(name))
        .map(|name| ManagedEntry {
            section: Section::Deps,
            agent: None,
            name: name.clone(),
            project: project.to_path_buf(),
            target: crate::deps::package_dir(prefix, &name)
                .to_string_lossy()
                .into_owned(),
        })
        .collect()
}

fn uninstall_plugin(agent: Agent, spec: &str) -> anyhow::Result<String> {
    use crate::agent::setup::{claude, omp, opencode, pi};
    match agent {
        Agent::Claude => claude::uninstall_plugin(spec),
        Agent::Pi => pi::remove_plugin(spec),
        Agent::Omp => omp::remove_plugin(spec),
        Agent::OpenCode => opencode::uninstall_plugin(spec),
        _ => anyhow::bail!("no plugin uninstaller for {}", agent.name()),
    }
}

/// Key set of everything the current run installed or confirmed.
pub fn installed_keys(items: &[ItemResult]) -> BTreeSet<Key> {
    items
        .iter()
        .filter(|i| i.managed.is_some())
        .filter(|i| i.outcome != Outcome::Removed)
        .map(|i| (i.section, i.agent.clone(), i.name.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(section: Section, name: &str, project: &str) -> ManagedEntry {
        ManagedEntry {
            section,
            agent: Some("pi".to_string()),
            name: name.to_string(),
            project: PathBuf::from(project),
            target: format!("/tmp/{name}"),
        }
    }

    fn keys(entries: &[ManagedEntry]) -> BTreeSet<Key> {
        entries.iter().map(ManagedEntry::key).collect()
    }

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("managed.json");
        let m = Manifest {
            version: VERSION,
            entries: vec![entry(Section::Skills, "grill-me", "/proj")],
        };
        m.save_to(&path).unwrap();
        let back = Manifest::load_from(&path);
        assert_eq!(back.version, VERSION);
        assert_eq!(back.entries, m.entries);
    }

    #[test]
    fn corrupt_manifest_loads_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("managed.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(Manifest::load_from(&path).entries.is_empty());
    }

    #[test]
    fn missing_manifest_loads_empty() {
        assert!(
            Manifest::load_from(Path::new("/nonexistent/managed.json"))
                .entries
                .is_empty()
        );
    }

    #[test]
    fn dropped_entry_is_stale() {
        let m = Manifest {
            version: VERSION,
            entries: vec![
                entry(Section::Skills, "kept", "/proj"),
                entry(Section::Skills, "dropped", "/proj"),
            ],
        };
        let installed = keys(&[entry(Section::Skills, "kept", "/proj")]);
        let stale = m.stale_for(Path::new("/proj"), &installed, &[Section::Skills]);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name, "dropped");
    }

    #[test]
    fn other_projects_entries_are_never_stale_here() {
        let m = Manifest {
            version: VERSION,
            entries: vec![entry(Section::Skills, "theirs", "/other")],
        };
        let stale = m.stale_for(Path::new("/proj"), &BTreeSet::new(), &[Section::Skills]);
        assert!(stale.is_empty());
    }

    #[test]
    fn a_feature_another_project_still_claims_survives() {
        let m = Manifest {
            version: VERSION,
            entries: vec![
                entry(Section::Skills, "shared", "/proj"),
                entry(Section::Skills, "shared", "/other"),
            ],
        };
        let stale = m.stale_for(Path::new("/proj"), &BTreeSet::new(), &[Section::Skills]);
        assert!(stale.is_empty(), "shared skill must outlive one claimant");
    }

    #[test]
    fn unrun_sections_are_not_pruned() {
        let m = Manifest {
            version: VERSION,
            entries: vec![entry(Section::Plugins, "npm:x", "/proj")],
        };
        let stale = m.stale_for(Path::new("/proj"), &BTreeSet::new(), &[Section::Skills]);
        assert!(stale.is_empty(), "--only skills must not prune plugins");
    }

    #[test]
    fn record_replaces_only_this_projects_run_sections() {
        let mut m = Manifest {
            version: VERSION,
            entries: vec![
                entry(Section::Skills, "old", "/proj"),
                entry(Section::Plugins, "npm:x", "/proj"),
                entry(Section::Skills, "theirs", "/other"),
            ],
        };
        m.record(
            Path::new("/proj"),
            &[Section::Skills],
            vec![entry(Section::Skills, "new", "/proj")],
            &[],
        );
        let names: BTreeSet<&str> = m.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            BTreeSet::from(["new", "npm:x", "theirs"]),
            "only this project's skills entries are replaced"
        );
    }

    #[test]
    fn removing_an_already_absent_skill_is_convergence_not_failure() {
        let e = ManagedEntry {
            section: Section::Skills,
            agent: Some("pi".into()),
            name: "gone".into(),
            project: PathBuf::from("/proj"),
            target: "/nonexistent/skills/gone".into(),
        };
        let out = prune(&[e], false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].outcome, Outcome::Removed);
    }

    #[test]
    fn prune_deletes_a_skill_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("grill-me");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "x").unwrap();

        let e = ManagedEntry {
            section: Section::Skills,
            agent: Some("pi".into()),
            name: "grill-me".into(),
            project: PathBuf::from("/proj"),
            target: dir.display().to_string(),
        };
        assert_eq!(
            prune(std::slice::from_ref(&e), true)[0].outcome,
            Outcome::Removed
        );
        assert!(dir.exists(), "dry run must not delete");

        assert_eq!(prune(&[e], false)[0].outcome, Outcome::Removed);
        assert!(!dir.exists());
    }

    #[test]
    fn unmanaged_items_never_enter_the_manifest() {
        let items = vec![
            ItemResult::new(Section::Skills, Some("pi"), "bundled", Outcome::Installed),
            ItemResult::new(Section::Skills, Some("pi"), "declared", Outcome::Installed)
                .managed_at("/tmp/declared"),
            ItemResult::failed(Section::Skills, Some("pi"), "broken", "boom")
                .managed_at("/tmp/broken"),
        ];
        let entries = entries_from_report(&items, Path::new("/proj"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "declared");
    }
}

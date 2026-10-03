//! Persistent registry of tracked project directories (`workmux project add/rm/list`).
//!
//! Stored as a plain YAML list at `~/.config/workmux/projects.yaml` so it is
//! trivially hand-editable:
//!
//! ```yaml
//! - name: workmux
//!   root: /home/user/repos/workmux
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug)]
pub struct Registry {
    path: PathBuf,
    pub projects: Vec<ProjectEntry>,
}

/// Outcome of an add, so callers can print an honest notice.
#[derive(Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added(ProjectEntry),
    AlreadyTracked(ProjectEntry),
}

impl Registry {
    pub fn load() -> Result<Self> {
        Self::load_from(crate::xdg::config_dir()?.join("projects.yaml"))
    }

    pub fn load_from(path: PathBuf) -> Result<Self> {
        let projects = match fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
        };
        Ok(Self { path, projects })
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let yaml = serde_yaml::to_string(&self.projects)?;
        // Atomic rewrite: a crash mid-write must not truncate the registry.
        let tmp = self.path.with_extension("yaml.tmp");
        fs::write(&tmp, yaml)?;
        fs::rename(&tmp, &self.path)
            .with_context(|| format!("Failed to write {}", self.path.display()))?;
        Ok(())
    }

    pub fn add(&mut self, dir: &Path) -> Result<AddOutcome> {
        if !dir.is_dir() {
            bail!("'{}' is not an existing directory", dir.display());
        }
        let root = dir
            .canonicalize()
            .with_context(|| format!("Could not resolve '{}'", dir.display()))?;
        if let Some(existing) = self.projects.iter().find(|p| p.root == root) {
            return Ok(AddOutcome::AlreadyTracked(existing.clone()));
        }
        let name = root
            .file_name()
            .context("Directory has no basename")?
            .to_string_lossy()
            .to_string();
        let entry = ProjectEntry { name, root };
        self.projects.push(entry.clone());
        self.save()?;
        Ok(AddOutcome::Added(entry))
    }

    /// Look up by name or path (unresolvable paths still match literally).
    pub fn find(&self, target: &str) -> Option<&ProjectEntry> {
        let target_path = Path::new(target).canonicalize().ok();
        self.projects.iter().find(|p| {
            p.name == target
                || p.root == Path::new(target)
                || target_path.as_deref() == Some(&p.root)
        })
    }

    /// Remove by name or path. Returns the removed entry, if any.
    pub fn remove(&mut self, target: &str) -> Result<Option<ProjectEntry>> {
        let Some(entry) = self.find(target).cloned() else {
            return Ok(None);
        };
        self.projects.retain(|p| p != &entry);
        self.save()?;
        Ok(Some(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_in(dir: &Path) -> Registry {
        Registry::load_from(dir.join("projects.yaml")).unwrap()
    }

    #[test]
    fn add_list_remove_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("myproj");
        fs::create_dir(&proj).unwrap();

        let mut reg = registry_in(tmp.path());
        let outcome = reg.add(&proj).unwrap();
        assert!(matches!(outcome, AddOutcome::Added(ref e) if e.name == "myproj"));

        // Persisted and reloadable
        let mut reg = registry_in(tmp.path());
        assert_eq!(reg.projects.len(), 1);

        // Idempotent
        assert!(matches!(
            reg.add(&proj).unwrap(),
            AddOutcome::AlreadyTracked(_)
        ));
        assert_eq!(reg.projects.len(), 1);

        // Remove by name
        assert!(reg.remove("myproj").unwrap().is_some());
        assert!(registry_in(tmp.path()).projects.is_empty());
        // Removing again is a no-op
        assert!(reg.remove("myproj").unwrap().is_none());
    }

    #[test]
    fn add_missing_dir_fails_and_leaves_registry_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = registry_in(tmp.path());
        assert!(reg.add(&tmp.path().join("nope")).is_err());
        assert!(reg.projects.is_empty());
        assert!(!tmp.path().join("projects.yaml").exists());
    }

    #[test]
    fn remove_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p1");
        fs::create_dir(&proj).unwrap();
        let mut reg = registry_in(tmp.path());
        reg.add(&proj).unwrap();
        assert!(reg.remove(proj.to_str().unwrap()).unwrap().is_some());
    }

    #[test]
    fn find_by_name_or_path() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p1");
        fs::create_dir(&proj).unwrap();
        let mut reg = registry_in(tmp.path());
        reg.add(&proj).unwrap();
        assert_eq!(reg.find("p1").unwrap().root, proj.canonicalize().unwrap());
        assert!(reg.find(proj.to_str().unwrap()).is_some());
        assert!(reg.find("nope").is_none());
    }
}

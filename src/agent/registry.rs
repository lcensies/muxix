//! Agent definition registry — loads and resolves named `AgentDefinition` profiles.
//!
//! Resolution order (first match wins):
//!   1. Inline `agent_defs:` in `.muxix.yaml`
//!   2. Files in `.muxix/agents/<name>.yaml`
//!   3. Remote sources (git/url) — fetched and cached locally (stub in Phase 1)
//!
//! Registry sources are declared in `.muxix.yaml` under `agent_registries:`:
//! ```yaml
//! agent_registries:
//!   - local: .muxix/agents        # always searched first (default location)
//!   - git:
//!       url: https://github.com/example-org/agent-registry
//!       ref: v1.0.0
//!       path: agents/
//! ```

use crate::agent::definition::AgentDefinition;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// A source in the registry search path.
///
/// Local sources are scanned eagerly at load time. Remote sources are fetched
/// on demand and cached (Phase 1: remote fetch is stubbed — returns an error
/// suggesting `muxix agent add` to vendor the profile locally).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RegistrySource {
    /// Local directory of `<name>.yaml` agent definition files.
    Local {
        /// Path relative to the project root, or absolute. Defaults to
        /// `.muxix/agents` if omitted.
        #[serde(default)]
        local: Option<String>,
    },
    /// Remote git repository containing agent definitions.
    Git { git: GitRegistrySource },
    /// Raw URL to a single agent definition YAML or a directory index.
    Url { url: String },
}

/// Git-based registry source configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRegistrySource {
    /// Repository URL (https or ssh).
    pub url: String,
    /// Git ref (tag, branch, or commit SHA) to pin for reproducibility.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
    /// Subdirectory within the repository containing agent definition files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The resolved agent definition registry.
///
/// Constructed once at runner setup from:
/// - Inline `agent_defs:` from `.muxix.yaml`
/// - Files scanned from local registry directories
///
/// Remote registry resolution is not yet implemented (Phase 1 stub).
#[derive(Debug, Default, Clone)]
pub struct AgentRegistry {
    /// All named definitions, in resolution order (earlier entries win on conflict).
    definitions: BTreeMap<String, AgentDefinition>,
}

impl AgentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a registry from:
    /// 1. Inline `agent_defs` (highest priority — project-local overrides)
    /// 2. Files in each `local` registry source directory
    ///
    /// `project_root` is used to resolve relative paths in registry sources.
    pub fn load(
        inline_defs: &BTreeMap<String, AgentDefinition>,
        sources: &[RegistrySource],
        project_root: &Path,
    ) -> Result<Self> {
        let mut registry = Self::new();

        // 1. Inline defs from .muxix.yaml win over everything else.
        for (name, def) in inline_defs {
            registry.definitions.insert(name.clone(), def.clone());
        }

        // 2. Local directory sources.
        for source in sources {
            match source {
                RegistrySource::Local { local } => {
                    let dir = local.as_deref().unwrap_or(".muxix/agents");
                    let dir_path = if Path::new(dir).is_absolute() {
                        PathBuf::from(dir)
                    } else {
                        project_root.join(dir)
                    };
                    if dir_path.exists() {
                        registry.load_from_dir(&dir_path)?;
                    }
                }
                RegistrySource::Git { git } => {
                    // Phase 1 stub: remote git registries are not yet fetched.
                    // Use `muxix agent add <url>` to vendor a remote profile locally.
                    debug!(
                        url = %git.url,
                        "remote git registry not yet fetched; use `muxix agent add` to vendor"
                    );
                }
                RegistrySource::Url { url } => {
                    debug!(
                        url = %url,
                        "remote url registry not yet fetched; use `muxix agent add` to vendor"
                    );
                }
            }
        }

        // Always scan the default local directory if not already declared.
        let default_dir = project_root.join(".muxix/agents");
        let default_declared = sources.iter().any(|s| {
            matches!(s, RegistrySource::Local { local } if local.as_deref().unwrap_or(".muxix/agents") == ".muxix/agents")
        });
        if !default_declared && default_dir.exists() {
            registry.load_from_dir(&default_dir)?;
        }

        Ok(registry)
    }

    /// Scan a directory for `<name>.yaml` and `<name>.yml` agent definition files.
    /// Files are loaded in alphabetical order; names are derived from the file stem.
    /// Inline definitions (already in registry) take precedence — files do NOT
    /// overwrite inline entries.
    fn load_from_dir(&mut self, dir: &Path) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading agent registry dir {}", dir.display()))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "yaml" && ext != "yml" {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Inline defs already loaded — skip to preserve override order.
            if self.definitions.contains_key(&name) {
                continue;
            }
            let content = fs::read_to_string(&path)
                .with_context(|| format!("reading agent definition {}", path.display()))?;
            let def: AgentDefinition = serde_yaml::from_str(&content)
                .with_context(|| format!("parsing agent definition {}", path.display()))?;
            debug!(name = %name, path = %path.display(), "loaded agent definition");
            self.definitions.insert(name, def);
        }
        Ok(())
    }

    /// Resolve a named agent definition. Returns `None` if not found.
    pub fn get(&self, name: &str) -> Option<&AgentDefinition> {
        self.definitions.get(name)
    }

    /// All known definition names, sorted.
    #[allow(dead_code)]
    pub fn names(&self) -> Vec<&str> {
        self.definitions.keys().map(|s| s.as_str()).collect()
    }

    /// All definitions, sorted by name.
    pub fn all(&self) -> Vec<(&str, &AgentDefinition)> {
        self.definitions
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }

    /// Whether the registry has any definitions.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::AgentDefinition;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_def(permission_mode: &str) -> AgentDefinition {
        AgentDefinition {
            permission_mode: Some(permission_mode.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn empty_registry_returns_none() {
        let r = AgentRegistry::new();
        assert!(r.get("anything").is_none());
        assert!(r.is_empty());
    }

    #[test]
    fn inline_defs_loaded_first() {
        let mut inline = BTreeMap::new();
        inline.insert("planner".to_string(), make_def("plan"));

        let dir = TempDir::new().unwrap();
        let r = AgentRegistry::load(&inline, &[], dir.path()).unwrap();
        let def = r.get("planner").unwrap();
        assert_eq!(def.permission_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn loads_yaml_files_from_dir() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".muxix").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        let def_yaml = "permission_mode: plan\ndescription: test planner\n";
        let mut f = fs::File::create(agents_dir.join("planner.yaml")).unwrap();
        f.write_all(def_yaml.as_bytes()).unwrap();

        let r = AgentRegistry::load(&BTreeMap::new(), &[], dir.path()).unwrap();
        let def = r.get("planner").unwrap();
        assert_eq!(def.permission_mode.as_deref(), Some("plan"));
        assert_eq!(def.description.as_deref(), Some("test planner"));
    }

    #[test]
    fn inline_overrides_file() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".muxix").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // File says "plan", inline says "implement" — inline wins.
        let def_yaml = "permission_mode: plan\n";
        let mut f = fs::File::create(agents_dir.join("worker.yaml")).unwrap();
        f.write_all(def_yaml.as_bytes()).unwrap();

        let mut inline = BTreeMap::new();
        inline.insert("worker".to_string(), make_def("implement"));

        let r = AgentRegistry::load(&inline, &[], dir.path()).unwrap();
        let def = r.get("worker").unwrap();
        assert_eq!(def.permission_mode.as_deref(), Some("implement"));
    }

    #[test]
    fn names_sorted() {
        let mut inline = BTreeMap::new();
        inline.insert("zz".to_string(), make_def("plan"));
        inline.insert("aa".to_string(), make_def("plan"));
        let r = AgentRegistry::load(&inline, &[], Path::new("/tmp")).unwrap();
        assert_eq!(r.names(), vec!["aa", "zz"]);
    }

    #[test]
    fn unknown_name_returns_none() {
        let dir = TempDir::new().unwrap();
        let r = AgentRegistry::load(&BTreeMap::new(), &[], dir.path()).unwrap();
        assert!(r.get("nonexistent").is_none());
    }
}

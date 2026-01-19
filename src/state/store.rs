//! Filesystem-based state persistence for agent state.

use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tracing::warn;

use super::types::{AgentState, GlobalSettings, PaneKey};

/// Manages filesystem-based state persistence for workmux agents.
///
/// Directory structure:
/// ```text
/// $XDG_STATE_HOME/workmux/           # ~/.local/state/workmux/
/// ├── settings.json                   # Global dashboard settings
/// └── agents/
///     ├── tmux__default__%1.json     # {backend}__{instance}__{pane_id}.json
///     └── wezterm__main__3.json
/// ```
pub struct StateStore {
    base_path: PathBuf,
}

impl StateStore {
    /// Create a new StateStore using XDG_STATE_HOME.
    ///
    /// Creates the base directory and agents subdirectory if they don't exist.
    pub fn new() -> Result<Self> {
        let base = get_state_dir()?.join("workmux");
        fs::create_dir_all(&base).context("Failed to create state directory")?;
        fs::create_dir_all(base.join("agents")).context("Failed to create agents directory")?;
        Ok(Self { base_path: base })
    }

    /// Create a StateStore with a custom base path (for testing).
    #[cfg(test)]
    pub fn with_path(base_path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&base_path)?;
        fs::create_dir_all(base_path.join("agents"))?;
        Ok(Self { base_path })
    }

    /// Path to agents directory.
    fn agents_dir(&self) -> PathBuf {
        self.base_path.join("agents")
    }

    /// Path to settings file.
    fn settings_path(&self) -> PathBuf {
        self.base_path.join("settings.json")
    }

    /// Path to a specific agent's state file.
    fn agent_path(&self, key: &PaneKey) -> PathBuf {
        self.agents_dir().join(key.to_filename())
    }

    /// Create or update agent state.
    ///
    /// Uses atomic write (temp file + rename) for crash safety.
    pub fn upsert_agent(&self, state: &AgentState) -> Result<()> {
        let path = self.agent_path(&state.pane_key);
        let content = serde_json::to_string_pretty(state)?;
        write_atomic(&path, content.as_bytes())
    }

    /// Read agent state by pane key.
    ///
    /// Returns None if the agent doesn't exist or the file is corrupted.
    #[allow(dead_code)] // Used in tests, may be used in future features
    pub fn get_agent(&self, key: &PaneKey) -> Result<Option<AgentState>> {
        read_agent_file(&self.agent_path(key))
    }

    /// List all agent states.
    ///
    /// Used for reconciliation and dashboard display.
    /// Skips corrupted files (logs warning and deletes them).
    pub fn list_all_agents(&self) -> Result<Vec<AgentState>> {
        let agents_dir = self.agents_dir();
        if !agents_dir.exists() {
            return Ok(Vec::new());
        }

        let mut agents = Vec::new();
        for entry in fs::read_dir(&agents_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".tmp"))
                && let Some(state) = read_agent_file(&path)?
            {
                agents.push(state);
            }
        }
        Ok(agents)
    }

    /// Delete agent state.
    ///
    /// No-op if the file doesn't exist.
    pub fn delete_agent(&self, key: &PaneKey) -> Result<()> {
        let path = self.agent_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).context("Failed to delete agent state"),
        }
    }

    /// Load global settings.
    ///
    /// Returns defaults if the file is missing or corrupted.
    pub fn load_settings(&self) -> Result<GlobalSettings> {
        let path = self.settings_path();
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(settings) => Ok(settings),
                Err(e) => {
                    warn!(?path, error = %e, "corrupted settings file, using defaults");
                    Ok(GlobalSettings::default())
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(GlobalSettings::default()),
            Err(e) => Err(e).context("Failed to read settings"),
        }
    }

    /// Save global settings.
    ///
    /// Uses atomic write for crash safety.
    pub fn save_settings(&self, settings: &GlobalSettings) -> Result<()> {
        let path = self.settings_path();
        let content = serde_json::to_string_pretty(settings)?;
        write_atomic(&path, content.as_bytes())
    }

    /// Load agents with reconciliation against live multiplexer state.
    ///
    /// Two-layer exit detection:
    /// - **PID validation**: Pane was closed and recycled (stored PID != live PID)
    /// - **Command comparison**: Agent exited within pane (foreground command changed)
    ///
    /// Returns only valid agents; removes stale state files.
    pub fn load_reconciled_agents(
        &self,
        mux: &dyn crate::multiplexer::Multiplexer,
    ) -> Result<Vec<crate::multiplexer::AgentPane>> {
        let mut valid_agents = Vec::new();
        let backend = mux.name();
        let instance = mux.instance_id();

        for state in self.list_all_agents()? {
            // Skip agents from other backends/instances
            if state.pane_key.backend != backend || state.pane_key.instance != instance {
                continue;
            }

            let live_pane = mux.get_live_pane_info(&state.pane_key.pane_id)?;

            match live_pane {
                None => {
                    // Pane no longer exists in multiplexer
                    self.delete_agent(&state.pane_key)?;
                }
                Some(ref live) if live.pid != state.pane_pid => {
                    // PID mismatch - pane ID was recycled by a new process
                    self.delete_agent(&state.pane_key)?;
                }
                Some(ref live) if live.current_command != state.command => {
                    // Command changed - agent exited (e.g., "node" -> "zsh")
                    self.delete_agent(&state.pane_key)?;
                }
                Some(live) => {
                    // Valid - include in dashboard
                    let agent_pane = state.to_agent_pane(
                        live.session.unwrap_or_default(),
                        live.window.unwrap_or_default(),
                    );
                    valid_agents.push(agent_pane);
                }
            }
        }

        Ok(valid_agents)
    }
}

/// Write content atomically using temp file + rename.
///
/// This ensures the target file is never partially written.
fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).context("Failed to write temp file")?;
    fs::rename(&tmp, path).context("Failed to rename temp file")?;
    Ok(())
}

/// Get the XDG state directory.
///
/// Checks XDG_STATE_HOME first, falls back to ~/.local/state.
fn get_state_dir() -> Result<PathBuf> {
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home));
    }

    if let Some(home_dir) = home::home_dir() {
        return Ok(home_dir.join(".local/state"));
    }

    anyhow::bail!("Could not determine state directory")
}

/// Read and parse an agent state file.
///
/// Returns None if file doesn't exist.
/// Deletes corrupted files and returns None (recoverable error).
fn read_agent_file(path: &Path) -> Result<Option<AgentState>> {
    match fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => Ok(Some(state)),
            Err(e) => {
                warn!(?path, error = %e, "corrupted state file, deleting");
                let _ = fs::remove_file(path);
                Ok(None)
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("Failed to read agent state"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (StateStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = StateStore::with_path(dir.path().to_path_buf()).unwrap();
        (store, dir)
    }

    fn test_pane_key() -> PaneKey {
        PaneKey {
            backend: "tmux".to_string(),
            instance: "default".to_string(),
            pane_id: "%1".to_string(),
        }
    }

    fn test_agent_state(key: PaneKey) -> AgentState {
        AgentState {
            pane_key: key,
            workdir: PathBuf::from("/home/user/project"),
            status: Some("🤖".to_string()),
            status_ts: Some(1234567890),
            pane_title: Some("Implementing feature X".to_string()),
            pane_pid: 12345,
            command: "node".to_string(),
            updated_ts: 1234567890,
        }
    }

    #[test]
    fn test_upsert_and_get_agent() {
        let (store, _dir) = test_store();
        let key = test_pane_key();
        let state = test_agent_state(key.clone());

        store.upsert_agent(&state).unwrap();

        let retrieved = store.get_agent(&key).unwrap().unwrap();
        assert_eq!(retrieved.pane_key, state.pane_key);
        assert_eq!(retrieved.workdir, state.workdir);
        assert_eq!(retrieved.status, state.status);
        assert_eq!(retrieved.pane_pid, state.pane_pid);
    }

    #[test]
    fn test_get_nonexistent_agent() {
        let (store, _dir) = test_store();
        let key = test_pane_key();

        let result = store.get_agent(&key).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_list_all_agents() {
        let (store, _dir) = test_store();

        let key1 = PaneKey {
            backend: "tmux".to_string(),
            instance: "default".to_string(),
            pane_id: "%1".to_string(),
        };
        let key2 = PaneKey {
            backend: "tmux".to_string(),
            instance: "default".to_string(),
            pane_id: "%2".to_string(),
        };

        store.upsert_agent(&test_agent_state(key1)).unwrap();
        store.upsert_agent(&test_agent_state(key2)).unwrap();

        let agents = store.list_all_agents().unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[test]
    fn test_delete_agent() {
        let (store, _dir) = test_store();
        let key = test_pane_key();
        let state = test_agent_state(key.clone());

        store.upsert_agent(&state).unwrap();
        assert!(store.get_agent(&key).unwrap().is_some());

        store.delete_agent(&key).unwrap();
        assert!(store.get_agent(&key).unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent_agent() {
        let (store, _dir) = test_store();
        let key = test_pane_key();

        // Should not error
        store.delete_agent(&key).unwrap();
    }

    #[test]
    fn test_atomic_write_creates_no_tmp_files() {
        let (store, dir) = test_store();
        let key = test_pane_key();
        let state = test_agent_state(key);

        store.upsert_agent(&state).unwrap();

        // Check no .tmp files remain
        let agents_dir = dir.path().join("agents");
        for entry in fs::read_dir(&agents_dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(!name.ends_with(".tmp"), "temp file should be cleaned up");
        }
    }

    #[test]
    fn test_corrupted_file_deleted() {
        let (store, dir) = test_store();
        let key = test_pane_key();

        // Write corrupted JSON
        let path = dir.path().join("agents").join(key.to_filename());
        fs::write(&path, "not valid json {{{").unwrap();

        // Should return None, not error
        let result = store.get_agent(&key).unwrap();
        assert!(result.is_none());

        // File should be deleted
        assert!(!path.exists());
    }

    #[test]
    fn test_settings_roundtrip() {
        let (store, _dir) = test_store();

        let settings = GlobalSettings {
            sort_mode: "priority".to_string(),
            hide_stale: true,
            preview_size: Some(30),
        };

        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();

        assert_eq!(loaded.sort_mode, settings.sort_mode);
        assert_eq!(loaded.hide_stale, settings.hide_stale);
        assert_eq!(loaded.preview_size, settings.preview_size);
    }

    #[test]
    fn test_missing_settings_returns_defaults() {
        let (store, _dir) = test_store();

        let settings = store.load_settings().unwrap();
        assert_eq!(settings.sort_mode, "");
        assert!(!settings.hide_stale);
        assert!(settings.preview_size.is_none());
    }

    #[test]
    fn test_corrupted_settings_returns_defaults() {
        let (store, dir) = test_store();

        let path = dir.path().join("settings.json");
        fs::write(&path, "not valid json").unwrap();

        let settings = store.load_settings().unwrap();
        assert_eq!(settings.sort_mode, "");
    }

    #[test]
    fn test_list_all_agents_ignores_tmp_files() {
        let (store, dir) = test_store();
        let key = test_pane_key();
        let state = test_agent_state(key);

        store.upsert_agent(&state).unwrap();

        // Create a stray tmp file
        let tmp_path = dir.path().join("agents").join("some_file.json.tmp");
        fs::write(&tmp_path, "{}").unwrap();

        let agents = store.list_all_agents().unwrap();
        assert_eq!(agents.len(), 1);
    }
}

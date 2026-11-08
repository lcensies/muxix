//! A scriptable [`Multiplexer`] for unit tests.
//!
//! Lives in the crate rather than in one test module so every module's tests
//! drive the same double. Behaviour is set with the builder methods; what the
//! code under test did is read back from [`FakeMux::calls`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, anyhow};

use super::types::{
    CreateSessionParams, CreateWindowInSessionParams, CreateWindowParams, LivePaneInfo,
    PaneSetupOptions, PaneSetupResult,
};
use super::{Multiplexer, PaneHandshake};
use crate::config::Config;

/// A multiplexer with a settable pane list that records what was done to it.
pub struct FakeMux {
    backend: &'static str,
    panes: Mutex<HashMap<String, LivePaneInfo>>,
    /// Make every pane query fail, for the "the pane query is unanswerable" arm.
    pane_query_fails: bool,
    current_pane: Option<String>,
    log: Mutex<Vec<String>>,
}

impl Default for FakeMux {
    fn default() -> Self {
        Self {
            backend: "tmux",
            panes: Mutex::new(HashMap::new()),
            pane_query_fails: false,
            current_pane: None,
            log: Mutex::new(Vec::new()),
        }
    }
}

impl FakeMux {
    /// A double reporting `backend` as its name, for backend-specific branches.
    pub fn named(backend: &'static str) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    /// Add a live pane running `command`.
    pub fn with_pane(self, pane_id: &str, pid: Option<u32>, command: &str) -> Self {
        self.panes.lock().unwrap().insert(
            pane_id.to_string(),
            LivePaneInfo {
                pid,
                current_command: Some(command.to_string()),
                working_dir: PathBuf::from("/repo/wt"),
                title: None,
                session: None,
                window: None,
            },
        );
        self
    }

    /// The pane a split would open beside.
    pub fn with_current_pane(mut self, pane_id: &str) -> Self {
        self.current_pane = Some(pane_id.to_string());
        self
    }

    /// Every pane query answers `Err`.
    pub fn failing_pane_query(mut self) -> Self {
        self.pane_query_fails = true;
        self
    }

    /// What the code under test did, in order.
    pub fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn record(&self, what: impl Into<String>) {
        self.log.lock().unwrap().push(what.into());
    }

    fn live(&self) -> Result<HashMap<String, LivePaneInfo>> {
        if self.pane_query_fails {
            return Err(anyhow!("pane query failed"));
        }
        Ok(self.panes.lock().unwrap().clone())
    }
}

impl Multiplexer for FakeMux {
    fn name(&self) -> &'static str {
        self.backend
    }

    fn is_running(&self) -> Result<bool> {
        Ok(true)
    }

    fn current_pane_id(&self) -> Option<String> {
        self.current_pane.clone()
    }

    fn active_pane_id(&self) -> Option<String> {
        self.current_pane.clone()
    }

    fn get_client_active_pane_path(&self) -> Result<PathBuf> {
        Ok(PathBuf::new())
    }

    fn create_window(&self, _params: CreateWindowParams) -> Result<String> {
        self.record("create_window");
        Ok("pane-1".to_string())
    }

    fn create_session(&self, _params: CreateSessionParams) -> Result<String> {
        self.record("create_session");
        Ok("pane-1".to_string())
    }

    fn create_window_in_session(&self, _params: CreateWindowInSessionParams) -> Result<String> {
        self.record("create_window_in_session");
        Ok("pane-1".to_string())
    }

    fn switch_to_session(&self, _prefix: &str, _name: &str) -> Result<()> {
        Ok(())
    }

    fn session_exists(&self, _full_name: &str) -> Result<bool> {
        Ok(false)
    }

    fn kill_session(&self, _full_name: &str) -> Result<()> {
        Ok(())
    }

    fn kill_window(&self, _full_name: &str) -> Result<()> {
        Ok(())
    }

    fn schedule_window_close(&self, _full_name: &str, _delay: Duration) -> Result<()> {
        Ok(())
    }

    fn schedule_session_close(&self, _full_name: &str, _delay: Duration) -> Result<()> {
        Ok(())
    }

    fn run_deferred_script(&self, _script: &str) -> Result<()> {
        Ok(())
    }

    fn shell_select_window_cmd(&self, _full_name: &str) -> Result<String> {
        Ok(String::new())
    }

    fn shell_kill_window_cmd(&self, _full_name: &str) -> Result<String> {
        Ok(String::new())
    }

    fn shell_switch_session_cmd(&self, _full_name: &str) -> Result<String> {
        Ok(String::new())
    }

    fn shell_kill_session_cmd(&self, _full_name: &str) -> Result<String> {
        Ok(String::new())
    }

    fn select_window(&self, _prefix: &str, _name: &str) -> Result<()> {
        Ok(())
    }

    fn window_exists(&self, _prefix: &str, _name: &str) -> Result<bool> {
        Ok(false)
    }

    fn window_exists_by_full_name(&self, _full_name: &str) -> Result<bool> {
        Ok(false)
    }

    fn current_window_name(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }

    fn get_all_session_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }

    fn filter_active_windows(&self, _windows: &[String]) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn find_last_window_with_prefix(&self, _prefix: &str) -> Result<Option<String>> {
        Ok(None)
    }

    fn find_last_window_with_base_handle(
        &self,
        _prefix: &str,
        _base_handle: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn wait_until_windows_closed(&self, _full_window_names: &[String]) -> Result<()> {
        Ok(())
    }

    fn wait_until_session_closed(&self, _full_session_name: &str) -> Result<()> {
        Ok(())
    }

    fn select_pane(&self, _pane_id: &str) -> Result<()> {
        Ok(())
    }

    fn switch_to_pane(&self, pane_id: &str, _window_hint: Option<&str>) -> Result<()> {
        self.record(format!("switch_to_pane {pane_id}"));
        Ok(())
    }

    fn display_message(&self, text: &str) {
        self.record(format!("message {text}"));
    }

    fn kill_pane(&self, pane_id: &str) -> Result<()> {
        self.record(format!("kill_pane {pane_id}"));
        Ok(())
    }

    fn respawn_pane(&self, pane_id: &str, _cwd: &Path, _cmd: Option<&str>) -> Result<String> {
        Ok(pane_id.to_string())
    }

    fn capture_pane(&self, _pane_id: &str, _lines: u16) -> Option<String> {
        None
    }

    fn send_keys(&self, _pane_id: &str, _command: &str) -> Result<()> {
        Ok(())
    }

    fn send_keys_to_agent(
        &self,
        _pane_id: &str,
        _command: &str,
        _agent: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    fn send_key(&self, _pane_id: &str, _key: &str) -> Result<()> {
        Ok(())
    }

    fn paste_multiline(&self, _pane_id: &str, _content: &str) -> Result<()> {
        Ok(())
    }

    fn get_default_shell(&self) -> Result<String> {
        Ok("/bin/sh".to_string())
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        Err(anyhow!("not used"))
    }

    fn set_status(&self, pane_id: &str, icon: &str, _auto_clear_on_focus: bool) -> Result<()> {
        self.record(format!("set_status {pane_id} {icon}"));
        Ok(())
    }

    fn clear_status(&self, pane_id: &str) -> Result<()> {
        self.record(format!("clear_status {pane_id}"));
        Ok(())
    }

    fn ensure_status_format(&self, _pane_id: &str) -> Result<()> {
        Ok(())
    }

    fn split_pane(
        &self,
        _target_pane_id: &str,
        _direction: &crate::config::SplitDirection,
        _cwd: &Path,
        _size: Option<u16>,
        _percentage: Option<u8>,
        _command: Option<&str>,
    ) -> Result<String> {
        self.record("split_pane");
        Ok("pane-2".to_string())
    }

    fn setup_panes(
        &self,
        initial_pane_id: &str,
        _panes: &[crate::config::PaneConfig],
        _working_dir: &Path,
        _options: PaneSetupOptions<'_>,
        _config: &Config,
        _task_agent: Option<&str>,
    ) -> Result<PaneSetupResult> {
        Ok(PaneSetupResult {
            focus_pane_id: initial_pane_id.to_string(),
            zoom_pane_id: None,
        })
    }

    fn instance_id(&self) -> String {
        "test".to_string()
    }

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        Ok(self.live()?.get(pane_id).cloned())
    }

    fn get_all_live_pane_info(&self) -> Result<HashMap<String, LivePaneInfo>> {
        self.live()
    }
}

//! Unified handle for multiplexer targets (windows or sessions).
//!
//! Centralizes all mode-dependent dispatch so callers don't need
//! `if mode == Session { ... } else { ... }` branches.

use anyhow::Result;
use std::time::Duration;

use crate::config::MuxMode;

use super::Multiplexer;
use super::util;

/// Returns "window" or "session" for a given mode.
pub fn mode_label(mode: MuxMode) -> &'static str {
    match mode {
        MuxMode::Window => "window",
        MuxMode::Session => "session",
    }
}

/// A unified handle for a multiplexer target (window or session).
///
/// Wraps a reference to the backend, the mode, prefix, and handle name,
/// then dispatches to the correct window or session methods.
pub struct MuxHandle<'a> {
    mux: &'a dyn Multiplexer,
    mode: MuxMode,
    prefix: &'a str,
    name: &'a str,
}

#[allow(dead_code)]
impl<'a> MuxHandle<'a> {
    pub fn new(mux: &'a dyn Multiplexer, mode: MuxMode, prefix: &'a str, name: &'a str) -> Self {
        Self {
            mux,
            mode,
            prefix,
            name,
        }
    }

    /// Returns "window" or "session".
    pub fn kind(&self) -> &'static str {
        match self.mode {
            MuxMode::Window => "window",
            MuxMode::Session => "session",
        }
    }

    pub fn is_session(&self) -> bool {
        self.mode == MuxMode::Session
    }

    pub fn mode(&self) -> MuxMode {
        self.mode
    }

    /// The prefixed name (e.g., "wm-feature-auth").
    pub fn full_name(&self) -> String {
        util::prefixed(self.prefix, self.name)
    }

    /// Check if the target exists.
    pub fn exists(&self) -> Result<bool> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.session_exists(&full),
            MuxMode::Window => self.mux.window_exists(self.prefix, self.name),
        }
    }

    /// Check if a target exists by its full name (including prefix).
    /// Useful when the full name was obtained from current_name() or similar.
    pub fn exists_full(mux: &dyn Multiplexer, mode: MuxMode, full_name: &str) -> Result<bool> {
        match mode {
            MuxMode::Session => mux.session_exists(full_name),
            MuxMode::Window => mux.window_exists_by_full_name(full_name),
        }
    }

    /// Activate (focus/switch to) the target.
    pub fn select(&self) -> Result<()> {
        match self.mode {
            MuxMode::Session => self.mux.switch_to_session(self.prefix, self.name),
            MuxMode::Window => self.mux.select_window(self.prefix, self.name),
        }
    }

    /// Kill the target.
    pub fn kill(&self) -> Result<()> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.kill_session(&full),
            MuxMode::Window => self.mux.kill_window(&full),
        }
    }

    /// Kill a target by its full name.
    pub fn kill_full(mux: &dyn Multiplexer, mode: MuxMode, full_name: &str) -> Result<()> {
        match mode {
            MuxMode::Session => mux.kill_session(full_name),
            MuxMode::Window => mux.kill_window(full_name),
        }
    }

    /// Schedule the target to close after a delay.
    pub fn schedule_close(&self, delay: Duration) -> Result<()> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.schedule_session_close(&full, delay),
            MuxMode::Window => self.mux.schedule_window_close(&full, delay),
        }
    }

    /// Schedule a target to close after a delay, by full name.
    pub fn schedule_close_full(
        mux: &dyn Multiplexer,
        mode: MuxMode,
        full_name: &str,
        delay: Duration,
    ) -> Result<()> {
        match mode {
            MuxMode::Session => mux.schedule_session_close(full_name, delay),
            MuxMode::Window => mux.schedule_window_close(full_name, delay),
        }
    }

    /// Get the current target name (session name or window name).
    pub fn current_name(&self) -> Result<Option<String>> {
        match self.mode {
            MuxMode::Session => Ok(self.mux.current_session()),
            MuxMode::Window => self.mux.current_window_name(),
        }
    }

    /// Generate a shell command to kill this target (for deferred scripts).
    pub fn shell_kill_cmd(&self) -> Result<String> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.shell_kill_session_cmd(&full),
            MuxMode::Window => self.mux.shell_kill_window_cmd(&full),
        }
    }

    /// Generate a shell command to select/activate this target (for deferred scripts).
    pub fn shell_select_cmd(&self) -> Result<String> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.shell_switch_session_cmd(&full),
            MuxMode::Window => self.mux.shell_select_window_cmd(&full),
        }
    }

    /// Wait until the target is closed.
    pub fn wait_until_closed(&self) -> Result<()> {
        let full = self.full_name();
        match self.mode {
            MuxMode::Session => self.mux.wait_until_session_closed(&full),
            MuxMode::Window => self.mux.wait_until_windows_closed(&[full]),
        }
    }

    /// Generate a shell command to kill a target by full name (for deferred scripts).
    pub fn shell_kill_cmd_full(
        mux: &dyn Multiplexer,
        mode: MuxMode,
        full_name: &str,
    ) -> Result<String> {
        match mode {
            MuxMode::Session => mux.shell_kill_session_cmd(full_name),
            MuxMode::Window => mux.shell_kill_window_cmd(full_name),
        }
    }

    /// Generate a shell command to select a target by full name (for deferred scripts).
    pub fn shell_select_cmd_full(
        mux: &dyn Multiplexer,
        mode: MuxMode,
        full_name: &str,
    ) -> Result<String> {
        match mode {
            MuxMode::Session => mux.shell_switch_session_cmd(full_name),
            MuxMode::Window => mux.shell_select_window_cmd(full_name),
        }
    }
}

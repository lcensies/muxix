//! Everything about a coding **agent**: its identity, its launch/runtime
//! parameters (profile), how it's detected and set up, and its bindings to
//! harness mechanisms (signals, MCP). This is the home for *what an agent is*.
//!
//! Deliberately separate from `crate::multiplexer`, which owns *how panes are
//! spawned and driven* (tmux/wezterm). The signal/MCP **mechanisms** live in
//! their own modules (`pipeline::turn_signal`, `mcp`); this module owns only the
//! per-agent **bindings** to them.
//!
//! Submodules:
//! - [`setup`]    — the `Agent` enum, detection, hook/plugin install, bootstrap,
//!   capability matrix (MCP/skills/signal support).
//! - [`profile`]  — launch/runtime parameters keyed by the agent command
//!   (continue flag, permission-mode flags, prompt argument, …).
//! - [`identity`] — classifying a running pane to an agent + sidebar metadata.

pub mod ade;
pub mod agent_profiles;
pub mod definition;
pub mod identity;
pub mod profile;
pub mod registry;
pub mod runtime;
pub mod setup;

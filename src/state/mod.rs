//! Filesystem-based state storage for workmux agents.
//!
//! This module provides persistent state storage that works across all
//! terminal multiplexer backends (tmux, WezTerm, Zellij).

mod store;
mod types;

pub use store::StateStore;
pub use types::{AgentState, PaneKey};

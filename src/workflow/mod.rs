// Module declarations
mod agent_resolve;
mod cleanup;
mod context;
mod create;
pub mod file_ops;
mod list;
mod merge;
mod open;
pub mod pr;
pub mod prompt_loader;
mod remove;
pub mod resurrect;
mod setup;
pub mod types;

// Public API re-exports
pub use agent_resolve::{
    find_worktree_root, match_agents_to_worktree, resolve_worktree_agent, resolve_worktree_agents,
};
pub use create::{create, create_with_changes};
pub use list::{list, list_in};
pub use merge::merge;
pub use open::open;
pub use remove::remove;
pub use setup::write_prompt_file;

// Re-export commonly used types for convenience
pub use context::WorkflowContext;
pub use types::{CreateArgs, SetupOptions};

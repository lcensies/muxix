//! Sandbox backends for running agents in isolated environments.

mod container;
pub mod lima;

pub use container::build_image;
pub use container::run_auth;
pub use container::wrap_for_container;
pub use lima::ensure_vm_running as ensure_lima_vm;
pub use lima::wrap_for_lima;

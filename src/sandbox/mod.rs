//! Container sandbox for running agents in isolated Docker/Podman containers.

mod container;

pub use container::run_auth;
pub use container::wrap_for_container;

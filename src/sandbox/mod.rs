//! Sandbox backends for running agents in isolated environments.

mod container;
pub mod guest;
pub mod lima;
pub mod rpc;

pub(crate) use container::build_docker_run_args;
pub use container::build_image;
pub(crate) use container::ensure_sandbox_config_dirs;
pub use container::run_auth;
pub use container::stop_containers_for_handle;
pub use container::wrap_for_container;
pub use lima::ensure_vm_running as ensure_lima_vm;
pub use lima::wrap_for_lima;

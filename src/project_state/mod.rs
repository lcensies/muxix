//! Per-project runtime state store at `.workmux/state/project.json`.
//!
//! Holds tri-state capability flags and free-form facts discovered/produced by
//! setup (e.g. `test_command`, `build_command`). Setup steps and harness bash
//! gates coordinate through a compare-and-swap [`store::ProjectStateStore::acquire`]
//! so only one actor performs a given setup step, with heartbeats and TTL-based
//! stale-lock reclaim guarding against a crashed owner deadlocking the rest.
//!
//! This is runtime FACTS, not workflow config — the workflow lives in
//! `.workmux/workflows/harness.yaml`.

mod lock;
pub mod store;
pub mod types;

pub use store::{DEFAULT_CAPABILITY_TTL_SECS, ProjectStateStore};
pub use types::AcquireOutcome;
// Re-exported as the module's public data model for preflight/harness consumers;
// not all are referenced elsewhere yet.
#[allow(unused_imports)]
pub use types::{Capability, CapabilityStatus, ProjectState};

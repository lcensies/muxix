//! Task graph primitives: the `tasks/index.json` store and its CLI surface.
//!
//! muxix owns the *store*, not the schedule: it loads, validates and atomically
//! updates tasks so an external harness can decide what to run next.

pub mod graph;
pub mod types;

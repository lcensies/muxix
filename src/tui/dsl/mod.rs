//! A general runtime layout DSL for ratatui screens.
//!
//! Screens are described as a YAML [`Node`] tree (containers, text, lists,
//! conditionals, native leaves) and rendered against a [`RenderCtx`] that
//! projects app state into bindable values. Specs can be reloaded at runtime
//! for fast UI prototyping (see [`spec`]).
//!
//! The DSL owns *composition* — layout, lists, sections, conditionals — for a
//! whole screen. Heavy interactive widgets (PTY preview, diff/patch viewers,
//! text editors) are placed by the DSL as `Node::Native` leaves and drawn by
//! the host, so they can be adopted incrementally without a rewrite.

pub mod ctx;
pub mod node;
pub mod parse;
pub mod render;
pub mod spec;
pub mod text;

pub use ctx::{FieldValue, ListItem, RenderCtx};
pub use render::render;
pub use spec::SpecSource;

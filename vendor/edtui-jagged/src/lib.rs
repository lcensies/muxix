//! # `edtui_jagged`
//!
//! `edtui_jagged` is a Rust library providing a generic container for working with an object,
//! where each element is organized into lines (rows).
//!
//! The central component of this library is the [`Jagged`] struct, which wraps a vector of
//! vectors. The outer vector represents rows, and the inner vectors represent the elements
//! within each row.
//!
//! ## Generic Parameters
//!
//! - `T`: The data type of elements stored within the jagged array.
//!
//! ## Examples
//!
//! ```rust
//! use edtui_jagged::Jagged;
//!
//! let data = vec![
//!     vec![1, 2, 3],
//!     vec![4, 5, 6],
//!     vec![7, 8, 9],
//!     vec![0],
//! ];
//!
//! let lines = Jagged::new(data);
//! ```
//!
//! The `Jagged` struct is equipped with various methods for working with the underlying data,
//! including iterators for efficient traversal and searching.
//!
//! ## Features
//!
//! - Generic container for working with jagged arrays.
//! - Convenient creation and manipulation of rows and elements.
//! - Iteration and searching utilities for enhanced data processing.
//!
//! _For more details, refer to the documentation of individual types and methods._
#![allow(clippy::module_name_repetitions)]
pub mod index;
pub mod jagged;
pub mod traits;
pub use index::Index2;
pub use jagged::Jagged;
pub use traits::JaggedIndex;

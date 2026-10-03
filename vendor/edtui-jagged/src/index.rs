//! # Index Module
//!
//! The `index` module provides types related to indexing elements within the jagged array.
use std::cmp::Ordering;

use crate::Jagged;

/// An index representing a specific position in a 2d jagged array.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Index2 {
    /// The row index.
    pub row: usize,
    /// The column index.
    pub col: usize,
}

impl Index2 {
    /// Create a new [`Index2`] with the given line and column indices.
    ///
    /// # Arguments
    ///
    /// * `row` - The row index of the position.
    /// * `col` - The column index of the position.
    ///
    /// # Examples
    ///
    /// ```
    /// use edtui_jagged::Index2;
    ///
    /// let pos = Index2::new(1, 2);
    /// assert_eq!(pos.row, 1);
    /// assert_eq!(pos.col, 2);
    /// ```
    #[must_use]
    pub fn new(row: usize, col: usize) -> Self {
        Index2 { row, col }
    }

    /// Whether the index is out of bounds
    #[must_use]
    pub fn out_of_bounds<T>(&self, jagged: &Jagged<T>) -> bool {
        let Some(max_col) = jagged.len_col(self.row) else {
            return true;
        };
        self.row >= jagged.len() || (self.col != 0 && self.col >= max_col)
    }
}

impl std::fmt::Display for Index2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.row, self.col)
    }
}

impl PartialOrd for Index2 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.row.cmp(&other.row) {
            Ordering::Less => Some(Ordering::Less),
            Ordering::Greater => Some(Ordering::Greater),
            Ordering::Equal => self.col.partial_cmp(&other.col),
        }
    }
}

/// An index representing a specific row in a jagged array.
pub struct RowIndex(pub(crate) usize);

impl RowIndex {
    /// Create a new [`RowIndex`].
    ///
    /// # Arguments
    ///
    /// * `index` - The row index of the position.
    #[must_use]
    pub fn new(index: usize) -> Self {
        RowIndex(index)
    }
}

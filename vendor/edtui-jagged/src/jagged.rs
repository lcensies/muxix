//! # Jagged Module
//!
//! The `jagged` module contains the central component of the `edtui_jagged` library,
//! the [`Jagged`] struct.
//! This struct represents a generic container for working with an object where each
//! element is organized into lines (rows).
mod helper;
mod iter;
pub mod lines;
mod match_indices;
use match_indices::MatchIndicesEq;

use crate::{
    index::RowIndex,
    traits::{JaggedRemove, JaggedSlice},
    Index2, JaggedIndex,
};
use std::{
    fmt::Debug,
    ops::{Bound, RangeBounds},
};

use self::match_indices::MatchIndices;

/// A generic container for working with an object, where each element is organized
/// into lines (rows).
///
/// The [`Jagged`] struct wraps a vector of vectors, where the outer vector represents
/// rows and the inner vectors represent the elements within each row.
///
/// # Generic Parameters
///
/// - `T`: The data type of elements stored within the jagged array.
///
/// # Examples
///
/// ```
/// use edtui_jagged::Jagged;
///
/// let data = vec![
///     vec![1, 2, 3],
///     vec![4, 5, 6],
///     vec![7, 8, 9],
///     vec![0],
/// ];
///
/// let lines = Jagged::new(data);
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Jagged<T> {
    pub(crate) data: Vec<Vec<T>>,
}

impl<T> Default for Jagged<T> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<T> Jagged<T> {
    /// Instantiates a new [`Jagged`] object.
    ///
    /// # Arguments
    ///
    /// - `data`: Data of the jagged array. Must be convertable into vec of vecs.
    ///
    /// # Examples
    ///
    /// ```
    /// use edtui_jagged::Jagged;
    ///
    /// let data = vec![
    ///     vec![1, 2, 3],
    ///     vec![4, 5, 6],
    ///     vec![7, 8, 9],
    ///     vec![0],
    /// ];
    ///
    /// let lines = Jagged::new(data);
    /// ```
    #[must_use]
    pub fn new<U>(data: U) -> Self
    where
        U: Into<Vec<Vec<T>>>,
    {
        Jagged { data: data.into() }
    }

    /// Clears the jagged array, removing all values.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Appends an element to the back of the array.
    pub fn push<U>(&mut self, slice: U)
    where
        U: JaggedSlice<T>,
    {
        slice.push_into(self);
    }

    /// Inserts an element at `position` within the rows, shifting all
    /// elements after it.
    pub fn insert<I, U>(&mut self, index: I, slice: U)
    where
        I: JaggedIndex<T>,
        U: JaggedSlice<T, Index = I>,
    {
        slice.insert_into(index, self);
    }

    /// Removes and returns the element at position index within the jagged array.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn remove<I>(&mut self, index: I) -> I::Output
    where
        I: JaggedRemove<T>,
    {
        index.remove(self)
    }

    /// Moves all the elements of `other` into `self`, leaving `other` empty.
    ///
    /// Use [`Self::merge`] if the arrays should be fused at tail and head instead.
    pub fn append(&mut self, other: &mut Self) {
        self.data.append(&mut other.data);
    }

    /// Moves all the elements of `other` into `self`, leaving `other` empty.
    ///
    /// Similar to [`Self::append`] but fuses the last vector of `self` with the
    /// first vector of `other`.
    pub fn merge(&mut self, other: &mut Self) {
        if other.data.is_empty() {
            return;
        }
        let last_row = self.last_row_index();
        if !self.data.is_empty() {
            self.data[last_row].append(&mut other.data.remove(0));
        }
        self.data.append(&mut other.data);
    }

    /// Consumes the Lines and returns the underlying nested vectors.
    pub fn into_vecs(self) -> Vec<Vec<T>> {
        self.data
    }

    /// Joins two consecutive rows together. Merge `row_index` with `row_index` + 1.
    /// # Example
    /// ```
    /// use edtui_jagged::Jagged;
    /// let mut data = Jagged::from("hello\nworld");
    /// data.join_lines(0);
    /// assert_eq!(data, Jagged::from("helloworld"));
    /// ````
    pub fn join_lines(&mut self, row_index: usize) {
        if row_index + 1 >= self.len() {
            return;
        }
        let mut row = self.data.remove(row_index + 1);
        self.data[row_index].append(&mut row);
    }

    /// Truncate lines up to the specified position.
    pub fn truncate<I>(&mut self, index: I)
    where
        I: Into<Index2>,
    {
        let index = index.into();
        if let Some(current_row) = self.get_mut(RowIndex::new(index.row)) {
            current_row.truncate(index.col);
        }
        self.data.truncate(index.row + 1);
    }

    /// Splits a `Jagged` array into two at the given index.
    ///
    /// Returns a newly allocated `Jagged` containing the elements in the range
    /// `[at, end)`. After the call, the original vector will be left containing
    /// the elements `[0, at)`.
    #[must_use]
    pub fn split_off<I>(&mut self, at: I) -> Self
    where
        I: Into<Index2>,
    {
        let at = at.into();
        if at.col == 0 {
            Self::new(self.data.split_off(at.row))
        } else {
            let mut lines = self.data.remove(at.row);
            let rest = lines.split_off(at.col);

            self.data.insert(at.row, lines);
            self.data.insert(at.row + 1, rest);

            Self::new(self.data.split_off(at.row + 1))
        }
    }

    /// Returns `true` if the object contains no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns `true` if a specific row contains no elements.
    /// Returns None if the row is out of bounds.
    #[must_use]
    pub fn is_empty_row(&self, row: usize) -> Option<bool> {
        self.len_col(row).map(|row| row == 0)
    }

    /// Get the number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Get the number of columns of a given row.
    ///
    /// # Panics
    ///
    /// Panics if `row` is out of bounds.
    #[must_use]
    pub fn len_col_unchecked(&self, row: usize) -> usize {
        self.data[row].len()
    }

    /// Get the number of columns of a given row.
    /// Returns None if the row is out of bounds.
    pub fn len_col(&self, row: usize) -> Option<usize> {
        self.data.get(row).map(std::vec::Vec::len)
    }

    // /// Find the first index.
    // /// Returns `Some(Index2)` if the matrix is not empty, otherwise `None`.
    // #[must_use]
    // pub fn first_index(&self) -> Option<Index2> {
    //     self.data
    //         .get(0)
    //         .and_then(|row| row.get(0))
    //         .map(|_| Index2::new(0, 0))
    // }

    /// Get a reference to the element at a specific position.
    /// Returns `Some(& T)` if the position is valid, otherwise `None`.
    #[must_use]
    pub fn get<I>(&self, index: I) -> Option<&I::Output>
    where
        I: JaggedIndex<T>,
    {
        index.get(self)
    }

    /// Get a mutable reference to the element at a specific position.
    /// Returns `Some(&mut T)` if the position is valid, otherwise `None`.
    #[must_use]
    pub fn get_mut<I>(&mut self, index: I) -> Option<&mut I::Output>
    where
        I: JaggedIndex<T>,
    {
        index.get_mut(self)
    }

    /// Get the next value and index based on the current position.
    /// Returns `Some((Some(&T), Index2))` if a next position exists, otherwise `None`.
    /// Returns the next value as `None` indicating an empty row.
    #[must_use]
    pub fn next<I>(&self, index: I) -> Option<(Option<&T>, Index2)>
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match (self.is_last_row(index), self.is_last_col(index)) {
            (true, true) => None,
            (false, true) => {
                let p = Index2::new(index.row + 1, 0);
                self.get(p).map_or(Some((None, p)), |v| Some((Some(v), p)))
            }
            _ => {
                let p = Index2::new(index.row, index.col + 1);
                self.get(p).map(|v| (Some(v), p))
            }
        }
    }

    /// Find the next position based on the current position.
    /// Returns `Some((Some(&T), Index2))` if a next position exists, otherwise `None`.
    /// Returns the next value as `None` indicating an empty row.
    #[must_use]
    pub fn next_mut<I>(&mut self, index: I) -> Option<(Option<&mut T>, Index2)>
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match (self.is_last_row(index), self.is_last_col(index)) {
            (true, true) => None,
            (false, true) => {
                let p = Index2::new(index.row + 1, 0);
                self.get_mut(p)
                    .map_or(Some((None, p)), |v| Some((Some(v), p)))
            }
            _ => {
                let p = Index2::new(index.row, index.col + 1);
                self.get_mut(p).map(|v| (Some(v), p))
            }
        }
    }

    /// Find the previous position based on the current position.
    /// Returns `Some((Some(&T), Index2))` if a next position exists, otherwise `None`.
    /// Returns the next value as `None` indicating an empty row.
    #[must_use]
    pub fn prev<I>(&self, index: I) -> Option<(Option<&T>, Index2)>
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match (self.is_first_row(index), self.is_first_col(index)) {
            (true, true) => None,
            (false, true) => {
                let row = index.row - 1;
                let index = Index2::new(row, self.len_col_unchecked(row).saturating_sub(1));
                self.get(index)
                    .map_or(Some((None, index)), |val| Some((Some(val), index)))
            }
            _ => {
                let index = Index2::new(index.row, index.col - 1);
                self.get(index).map(|v| (Some(v), index))
            }
        }
    }

    /// Find the previous position based on the current position.
    /// Returns `Some((&mut T, Index2))` if a previous position exists, otherwise `None`.
    #[must_use]
    pub fn prev_mut<I>(&mut self, index: I) -> Option<(&mut T, Index2)>
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match (self.is_first_row(index), self.is_first_col(index)) {
            (true, true) => None,
            (false, true) => {
                let row = index.row - 1;
                let index = Index2::new(row, self.len_col_unchecked(row).saturating_sub(1));
                self.get_mut(index).map(|val| (val, index))
            }
            _ => {
                let p = Index2::new(index.row, index.col - 1);
                self.get_mut(p).map(|v| (v, p))
            }
        }
    }

    /// Find the next position that satisfies a given predicate.
    /// Returns `Some((Option<&T>, Index2))` if a position is found that satisfies the
    /// predicate, otherwise `None`.
    #[must_use]
    pub fn next_predicate<F, I>(&self, index: I, f: F) -> Option<(Option<&T>, Index2)>
    where
        F: Fn(Option<&T>) -> bool,
        I: Into<Index2>,
    {
        let mut index = index.into();
        while let Some((val, pos)) = self.next(index) {
            if f(val) {
                return Some((val, pos));
            }
            index = pos;
        }
        None
    }

    /// Find the next position that satisfies a given predicate.
    /// Returns `Some((Option<&mut T>, Index2))` if a position is found that satisfies the
    /// predicate, otherwise `None`.
    #[must_use]
    pub fn next_predicate_mut<F, I>(&mut self, index: I, f: F) -> Option<(Option<&mut T>, Index2)>
    where
        F: Fn(Option<&T>) -> bool,
        I: Into<Index2>,
    {
        let mut index = index.into();
        while let Some((val, pos)) = self.next(index) {
            if f(val) {
                return Some((self.get_mut(pos), pos));
            }
            index = pos;
        }
        None
    }

    /// Find the previous position that satisfies a given predicate.
    /// Returns `Some((Option<&T>, Index2))` if a satisfying position is found, otherwise `None`.
    #[must_use]
    pub fn prev_predicate<F, I>(&self, index: I, f: F) -> Option<(Option<&T>, Index2)>
    where
        F: Fn(Option<&T>) -> bool,
        I: Into<Index2>,
    {
        let mut index = index.into();
        while let Some((val, next)) = self.prev(index) {
            if f(val) {
                return Some((val, next));
            }
            index = next;
        }
        None
    }

    /// Find the previous position that satisfies a given predicate.
    /// Returns `Some((Option<&mut T>, Index2))` if a satisfying position is found, otherwise `None`.
    #[must_use]
    pub fn prev_predicate_mut<F, I>(&mut self, index: I, f: F) -> Option<(Option<&mut T>, Index2)>
    where
        F: Fn(Option<&T>) -> bool,
        I: Into<Index2>,
    {
        let mut index = index.into();
        while let Some((val, pos)) = self.prev(index) {
            if f(val) {
                return Some((self.get_mut(pos), pos));
            }
            index = pos;
        }
        None
    }

    fn range_bounds<R>(&self, range: R) -> Option<(Index2, Index2)>
    where
        R: RangeBounds<Index2>,
    {
        let start = match range.start_bound() {
            Bound::Included(val) => Index2::new(val.row, val.col),
            Bound::Excluded(val) => Index2::new(val.row, val.col + 1),
            Bound::Unbounded => Index2::new(0, 0),
        };

        let end = match range.end_bound() {
            Bound::Included(val) => Index2::new(val.row, val.col),
            Bound::Excluded(val) => match (val.row, val.col) {
                (0, 0) => return None,
                (row, 0) => Index2::new(row - 1, self.last_col_index(row - 1)),
                (row, col) => Index2::new(row, col - 1),
            },
            Bound::Unbounded => {
                let last_row = self.last_row_index();
                Index2::new(last_row, self.last_col_index(last_row))
            }
        };

        Some((start, end))
    }

    /// Extracts a range of [Index2]..[Index2] and returns a newly allocated `Jagged<T>`.
    ///
    /// Returns empty data if the input range is incorrectly orderered or if both start
    /// and end position are out of bounds.
    ///
    /// See [`Jagged::get_range`], if you do not want to remove the slice from the
    /// original data.
    ///
    /// # Example
    /// ```
    /// use edtui_jagged::{Index2, Jagged};
    ///
    /// let mut data = Jagged::from("hello world!");
    /// let drained = data.extract(Index2::new(0, 0)..Index2::new(0, 5));
    /// assert_eq!(drained, Jagged::from("hello"));
    /// assert_eq!(data, Jagged::from(" world!"));
    /// ```
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn extract<R>(&mut self, range: R) -> Self
    where
        R: RangeBounds<Index2>,
    {
        // This function is a bit of a mess. Turned out it is not that easy
        // to extract slices while trying to handle out of bounds gracefully
        // Maybe it would have been better to panic on out of bounds input.

        #[inline]
        fn drain_into_jagged<U>(drain: std::vec::Drain<U>) -> Jagged<U> {
            Jagged::new(vec![drain.collect::<Vec<U>>()])
        }
        if self.is_empty() {
            return Jagged::default();
        }

        let Some((mut start, mut end)) = self.range_bounds(range) else {
            return Jagged::default();
        };

        // Handle start row out of bounds, return empty
        if start.row > self.last_row_index() {
            return Jagged::default();
        }

        let mut drained = Jagged::<T>::default();

        // Handle start col out of bounds
        let mut start_column_out_of_bounds = false;
        let max_start_col = self
            .len_col(start.row)
            .map_or(0, |line| line.saturating_sub(1));
        if start.col > max_start_col {
            start.col = max_start_col;
            start_column_out_of_bounds = true;
        }

        if start_column_out_of_bounds {
            drained.push(Vec::new());
        }

        // If start.row is now out of bounds after adjustment return empty
        if start.row > self.last_row_index() {
            return drained;
        }

        // Handle end being out of bounds on the selection end
        let mut end_column_out_of_bounds = false;
        if end.row > self.last_row_index() {
            end.row = self.last_row_index();
            let max_end_col = self.last_col_index(end.row);
            end.col = max_end_col;
            end_column_out_of_bounds = true;
        } else {
            let max_end_col = self.last_col_index(end.row);
            if end.col > max_end_col {
                end.col = max_end_col;
                end_column_out_of_bounds = true;
            }
        }

        // Determine the start from which row extraction begins
        let mut split_start: Option<usize> = None;
        let extract_from = if start.col == 0 && !start_column_out_of_bounds {
            start.row
        } else if start_column_out_of_bounds {
            start.row + 1
        } else {
            split_start = Some(start.col);
            start.row + 1
        };

        // Determine the end until which to extract rows
        let mut split_end: Option<usize> = None;
        let max_end_col = self.last_col_index(end.row);
        let extract_until = if end.col >= max_end_col {
            end.row
        } else {
            split_end = Some(end.col);
            end.row.saturating_sub(1)
        };

        if start > end || (start == end && start_column_out_of_bounds) {
            return Jagged::default();
        }

        // Handle case where entire row is extracted (no splitting)
        if start.row == end.row && split_start.is_none() && split_end.is_none() {
            drained.append(&mut self.extract_rows(start.row..=start.row));
            return drained;
        }

        // Handle case where entire extraction happens on a single line (splitting)
        if start.row == end.row {
            let row = &mut self.data[start.row];
            drained.append(&mut drain_into_jagged(row.drain(start.col..=end.col)));
            if start_column_out_of_bounds {
                self.join_lines(start.row.saturating_sub(1));
            } else if end_column_out_of_bounds {
                self.join_lines(start.row);
            }
            return drained;
        }

        // Handle case where the extraction takes place over multiple lines
        // Split the first line, if needed. Then extract rows. Finally split
        // the last line, if needed.

        if let Some(split_start) = split_start {
            let row = &mut self.data[start.row];
            drained.append(&mut drain_into_jagged(row.drain(split_start..)));
        }

        let mut drained_rows = self.extract_rows(extract_from..=extract_until);
        let num_drained_rows = drained_rows.len();
        drained.append(&mut drained_rows);

        if let Some(split_end) = split_end {
            let row = &mut self.data[end.row.saturating_sub(num_drained_rows)];
            let mut drained_row = drain_into_jagged(row.drain(..=split_end));
            drained.append(&mut drained_row);
        }

        if split_start.is_some() || start_column_out_of_bounds {
            self.join_lines(start.row);
        }

        drained
    }

    /// Extracts a range of rows and returns a newly allocated `Jagged<T>`.
    ///
    /// # Example
    /// ```
    /// use edtui_jagged::{Index2, Jagged};
    ///
    /// let mut data = Jagged::from("hello\n\nworld!");
    /// let drained = data.extract_rows(0..1);
    /// assert_eq!(drained, Jagged::from("hello"));
    /// assert_eq!(data, Jagged::from("\nworld!"));
    /// ```
    #[must_use]
    pub fn extract_rows<R>(&mut self, range: R) -> Self
    where
        R: RangeBounds<usize>,
    {
        Jagged::new(self.data.drain(range).collect::<Vec<Vec<T>>>())
    }
}

impl<T: Clone> Jagged<T> {
    // Flattens the jagged array into a single vector with optional line breaks.
    ///
    /// Returns a flattened `Vec<T>` where each row from the original structure is
    /// concatenated into a single vector. If provided, the `line_break` parameter
    /// is inserted between rows.
    pub fn flatten(&self, line_break: &Option<T>) -> Vec<T> {
        let mut flattened = Vec::new();

        for (i, row) in self.data.iter().enumerate() {
            flattened.extend_from_slice(row);
            if i < self.data.len() - 1 {
                if let Some(item) = line_break.clone() {
                    flattened.push(item);
                }
            }
        }

        flattened
    }

    /// Returns a new [`Jagged`] array by copying a range from [`Jagged`].
    ///
    /// See [`Jagged::extract`], if you want to remove the slice from the
    /// original data.
    ///
    /// Handles ranges out of bounds as such:
    /// - if start is out of bounds, add a new line at position zero and
    ///   copy from the start of next line.
    /// - if end is out of bounds, append a new line at the end of the
    ///   copied data and copy from the end of the previous row.
    #[must_use]
    pub fn copy_range<R>(&self, range: R) -> Self
    where
        R: RangeBounds<Index2>,
    {
        let mut copied_lines = Self::default();
        let Some((mut start, mut end)) = self.range_bounds(range) else {
            return Jagged::default();
        };

        // Handle start out of bounds
        if start.col > self.last_col_index(start.row) {
            copied_lines.push(vec![]);
            start = Index2::new(start.row.saturating_add(1), 0);
        }

        // Handle end out of bounds
        let mut append_newline_at_the_end = false;
        if end.col > self.last_col_index(end.row) {
            end.col = self.last_col_index(end.row);
            append_newline_at_the_end = true;
        }

        copied_lines.append(&mut self.iter().from(start).to(end).collect::<Self>());

        if append_newline_at_the_end {
            copied_lines.push(vec![]);
        }

        copied_lines
    }
}

impl<T: MatchIndicesEq> Jagged<T> {
    /// Returns an iterator that searches for disjoint matches of a pattern within the array.
    ///
    /// # Example
    ///
    /// ```
    /// use edtui_jagged::{Jagged, Index2};
    ///
    /// let jagged = Jagged::from("aabcaabc\n\naabc.");
    /// let pattern: Vec<char> = vec!['a', 'b', 'c'];
    ///
    /// let mut match_indices = jagged.match_indices(&pattern);
    /// let index = match_indices.next().map(|(_, index)| index);
    /// assert_eq!(index, Some(Index2::new(0, 1)));
    /// ```
    ///
    /// The iterator returned by this method yields tuples, where the first element
    /// is the matched slice and the second element is the corresponding index.
    #[must_use]
    pub fn match_indices<'b>(&self, pattern: &'b [T]) -> MatchIndices<'_, 'b, T> {
        MatchIndices::new(self, pattern)
    }
}

impl<T: Clone> Jagged<T> {
    /// Returns a clone of the underlying nested vectors.
    pub fn to_vecs(&self) -> Vec<Vec<T>> {
        self.data.clone()
    }
}

#[cfg(test)]
mod tests {
    use crate::traits::RowSlice;

    use super::*;

    #[test]
    fn test_push() {
        let mut a = Jagged::new(vec![vec![1, 2, 3]]);
        a.push(4);

        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3, 4]]));
    }

    #[test]
    fn test_push_row() {
        let mut a = Jagged::new(vec![vec![1, 2, 3]]);
        a.push(RowSlice::from(vec![4]));

        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3], vec![4]]));
    }

    #[test]
    fn test_push_vec() {
        let mut a = Jagged::new(vec![vec![1, 2, 3]]);
        a.push(vec![4]);

        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3], vec![4]]));
    }

    #[test]
    fn test_append() {
        let mut a = Jagged::new(vec![vec![1, 2, 3]]);
        let mut b = Jagged::new(vec![vec![4, 5, 6]]);
        a.append(&mut b);

        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]));
    }

    #[test]
    fn test_merge() {
        let mut a = Jagged::new(vec![vec![1, 2]]);
        let mut b = Jagged::new(vec![vec![3], vec![4, 5, 6]]);

        a.merge(&mut b.clone());
        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]));

        let mut a_empty = Jagged::new(vec![]);
        a_empty.merge(&mut b);
        assert_eq!(a_empty, Jagged::new(vec![vec![3], vec![4, 5, 6]]));
    }

    #[test]
    fn test_flatten() {
        // given
        let a = Jagged::new(vec![vec![1], vec![], vec![2]]);
        // when
        let flattened = a.flatten(&Some(0));
        // then
        assert_eq!(flattened, vec![1, 0, 0, 2]);
    }

    #[test]
    fn test_iter() {
        let lines = Jagged::from(
            "Hello\n\
            World",
        );
        let mut iter = lines.iter_row();

        assert_eq!(iter.next(), Some(&"Hello".chars().collect()));
        assert_eq!(iter.next(), Some(&"World".chars().collect()));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_split_off() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let mut a = Jagged::new(data);

        let b = a.split_off(Index2::new(1, 1));
        assert_eq!(a, Jagged::new(vec![vec![1, 2, 3], vec![4]]));
        assert_eq!(b, Jagged::new(vec![vec![5, 6], vec![7, 8, 9]]));
    }

    #[test]
    fn test_next() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert_eq!(
            lines.next(Index2::new(0, 0)),
            Some((Some(&2), Index2::new(0, 1)))
        );
        assert_eq!(
            lines.next(Index2::new(0, 2)),
            Some((None, Index2::new(1, 0)))
        );
        assert_eq!(lines.next(Index2::new(2, 2)), None,);
    }

    #[test]
    fn test_prev() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert_eq!(
            lines.prev(Index2::new(1, 1)),
            Some((Some(&4), Index2::new(1, 0)))
        );
        assert_eq!(
            lines.prev(Index2::new(1, 0)),
            Some((Some(&3), Index2::new(0, 2)))
        );
        assert_eq!(lines.prev(Index2::new(0, 0)), None,);
    }

    #[test]
    fn test_next_predicate() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert_eq!(
            lines.next_predicate(Index2::new(0, 2), |val| val == Some(&5)),
            Some((Some(&5), Index2::new(2, 1)))
        );
        assert_eq!(
            lines.next_predicate(Index2::new(0, 0), |val| val == Some(&99)),
            None,
        );
    }

    #[test]
    fn test_prev_predicate() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert_eq!(
            lines.prev_predicate(Index2::new(2, 2), |val| val == Some(&5)),
            Some((Some(&5), Index2::new(1, 1)))
        );
        assert_eq!(
            lines.prev_predicate(Index2::new(2, 2), |val| val == Some(&99)),
            None,
        );
    }

    #[test]
    fn test_from_str() {
        let lines = Jagged::from("H\n");

        assert_eq!(lines, Jagged::new(vec![vec!['H'], vec![]]));
    }

    #[test]
    fn test_extract() {
        // given
        let original = Jagged::from("first\n\nsecond\nthird");

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 0)..Index2::new(0, 2));

        //then
        let expected_drained = Jagged::from("fi");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("rst\n\nsecond\nthird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 0)..=Index2::new(1, 0));

        //then
        let expected_drained = Jagged::from("first\n");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("second\nthird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 2)..Index2::new(2, 2));

        //then
        let expected_drained = Jagged::from("rst\n\nse");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("ficond\nthird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(2, 2)..Index2::new(3, 2));

        //then
        let expected_drained = Jagged::from("cond\nth");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first\n\nseird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(2, 0)..=Index2::new(3, 0));

        //then
        let expected_drained = Jagged::from("second\nt");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first\n\nhird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();

        let drained = data.extract(Index2::new(1, 0)..=Index2::new(2, 1));

        //then
        let expected_drained = Jagged::from("\nse");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first\ncond\nthird");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(1, 3)..=Index2::new(2, 1));

        //then
        let expected_drained = Jagged::from("\nse");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first\ncond\nthird");
        assert_eq!(data, expected_remaining);
    }

    #[test]
    fn test_extract_out_of_bounds() {
        // given
        let original = Jagged::from("first\nsecond");

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 0)..Index2::new(0, 99));

        //then
        let expected_drained = Jagged::from("first");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("second");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 99)..Index2::new(1, 99));

        //then
        let expected_drained = Jagged::from("\nsecond");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 10)..Index2::new(11, 0));

        //then
        let expected_drained = Jagged::from("\nsecond");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("first");
        assert_eq!(data, expected_remaining);

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 1)..Index2::new(0, 99));

        //then
        let expected_drained = Jagged::from("irst");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("fsecond");
        assert_eq!(data, expected_remaining);
    }

    #[test]
    fn test_extract_empty_buffer() {
        // given
        let original = Jagged::from("");

        // when
        let mut data = original.clone();
        let drained = data.extract(Index2::new(0, 0)..=Index2::new(0, 0));

        // then
        let expected_drained = Jagged::from("");
        assert_eq!(drained, expected_drained);
        let expected_remaining = Jagged::from("");
        assert_eq!(data, expected_remaining);
    }

    #[test]
    fn test_extract_end_larger_than_start() {
        // given
        let original = Jagged::from("first\nsecond");

        // when
        let mut data = original.clone();
        let _ = data.extract(Index2::new(0, 99)..Index2::new(0, 90));
    }

    #[test]
    fn test_extract_end_equals_start() {
        // given
        let original = Jagged::from("first\nsecond");

        // when
        let mut data = original.clone();
        let _ = data.extract(Index2::new(0, 1)..Index2::new(0, 1));
    }

    #[test]
    fn test_copy_range() {
        let data = Jagged::from("Hello\nWorld");

        let start = Index2::new(0, 3);
        let end = Index2::new(1, 1);
        assert_eq!(data.copy_range(start..=end), Jagged::from("lo\nWo"));

        let start = Index2::new(0, 0);
        let end = Index2::new(0, 2);
        assert_eq!(data.copy_range(start..end), Jagged::from("He"));

        let start = Index2::new(0, 0);
        let end = Index2::new(0, 1);
        assert_eq!(data.copy_range(start..end), Jagged::from("H"));

        let start = Index2::new(0, 0);
        let end = Index2::new(0, 0);
        assert_eq!(data.copy_range(start..end), Jagged::default());
    }

    #[test]
    fn test_copy_range_out_of_bounds() {
        let data = Jagged::from("Hello\nWorld");
        let start = Index2::new(0, 5);
        let end = Index2::new(1, 1);

        assert_eq!(data.copy_range(start..=end), Jagged::from("\nWo"));
    }
}

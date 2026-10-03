use std::{char, collections::VecDeque, fmt::Debug};

use crate::{Index2, Jagged};

/// An iterator over the disjoint matches of a pattern within this array.
pub struct MatchIndices<'a, 'b, T> {
    /// The array to be search through.
    data: &'a Jagged<T>,

    /// The pattern that is to be seached for
    pattern: &'b [T],

    /// The index of the start position.
    start_index: Option<Index2>,
}

pub trait MatchIndicesEq {
    /// This method tests for `self` and `other` values to be equal, and is used
    /// by `==`.
    #[must_use]
    fn eq(&self, other: &Self) -> bool;
}
impl MatchIndicesEq for char {
    fn eq(&self, other: &Self) -> bool {
        self.to_lowercase().collect::<String>() == other.to_lowercase().collect::<String>()
    }
}

impl<'a, 'b, T: MatchIndicesEq> MatchIndices<'a, 'b, T> {
    /// Instantiates a new [`MatchIndices`] that starts from a given position.
    #[must_use]
    pub(super) fn new(data: &'a Jagged<T>, pattern: &'b [T]) -> Self {
        Self {
            data,
            pattern,
            start_index: Some(Index2::default()),
        }
    }

    fn match_found(&self, other: &VecDeque<&T>) -> bool {
        if self.pattern.len() != other.len() {
            return false;
        }
        for (a, b) in self.pattern.iter().zip(other.iter()) {
            if !a.eq(b) {
                return false;
            }
        }
        true
    }
}

impl<'a, 'b, T: MatchIndicesEq + Debug> Iterator for MatchIndices<'a, 'b, T> {
    type Item = (&'b [T], Index2);

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() || self.pattern.is_empty() {
            return None;
        }
        // If the start index is None at this point, this means that the
        // previous iteration ended at the last element of the array and
        // we can stop here prematurely.
        let start_index = self.start_index?;
        let pattern_len = self.pattern.len();
        let mut sequence_buffer = VecDeque::<&T>::new();
        for (value, index) in self.data.iter().from(start_index) {
            let Some(value) = value else { continue };
            if index.col == 0 {
                sequence_buffer.clear();
            }
            if sequence_buffer.len() >= pattern_len {
                sequence_buffer.pop_front();
            }
            sequence_buffer.push_back(value);
            if self.match_found(&sequence_buffer) {
                // We set the start index for the next iteration.The index might
                // be none in which case the next iteration will stop.
                self.start_index = self.data.next(index).map(|(_, index)| index);
                // The match was found n elements before where n is the
                // length of the pattern.
                let mut index = index;
                index.col -= pattern_len.saturating_sub(1);
                return Some((self.pattern, index));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_obj_long() -> Jagged<char> {
        Jagged::from("aaBcaabc\n\naabc.")
    }

    #[test]
    fn test_match_indices() {
        let jagged = test_obj_long();
        let pattern: Vec<char> = vec!['a', 'b', 'c'];

        let mut match_indices = jagged.match_indices(&pattern);
        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, Some(Index2::new(0, 1)));

        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, Some(Index2::new(0, 5)));

        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, Some(Index2::new(2, 1)));

        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, None);
    }

    #[test]
    fn test_match_indices_empty_array() {
        let jagged = Jagged::default();
        let pattern: Vec<char> = vec!['a', 'b', 'c'];

        let mut match_indices = jagged.match_indices(&pattern);
        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, None);
    }

    #[test]
    fn test_match_indices_empty_pattern() {
        let jagged = test_obj_long();
        let pattern: Vec<char> = vec![];

        let mut match_indices = jagged.match_indices(&pattern);
        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, None);
    }

    #[test]
    fn test_match_indices_no_match() {
        let jagged = test_obj_long();
        let pattern: Vec<char> = vec!['x', 'y'];

        let mut match_indices = jagged.match_indices(&pattern);
        let index = match_indices.next().map(|(_, index)| index);
        assert_eq!(index, None);
    }
}

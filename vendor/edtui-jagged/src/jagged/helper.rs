use crate::{index::RowIndex, Index2, Jagged};

impl<T> Jagged<T> {
    /// Returns the first row's index.
    #[must_use]
    pub fn first_row_index(&self) -> usize {
        0
    }

    /// Returns the last row's index.
    #[must_use]
    pub fn last_row_index(&self) -> usize {
        self.len().saturating_sub(1)
    }

    /// Returns the first cols's index within a row.
    #[must_use]
    pub fn first_col_index(&self, _: usize) -> usize {
        0
    }

    /// Returns the last col's index within a row.
    #[must_use]
    pub fn last_col_index(&self, row_index: usize) -> usize {
        match self.get(RowIndex::new(row_index)) {
            Some(row) => row.len().saturating_sub(1),
            None => 0,
        }
    }

    /// Returns the first row, if it exists.
    #[must_use]
    pub fn first_row(&self) -> Option<&Vec<T>> {
        self.get(RowIndex::new(self.first_row_index()))
    }

    /// Returns the last row, if it exists.
    #[must_use]
    pub fn last_row(&self) -> Option<&Vec<T>> {
        self.get(RowIndex::new(self.last_row_index()))
    }

    /// Returns the first col within a row, if it exists.
    #[must_use]
    pub fn first_col(&self, row_index: usize) -> Option<&T> {
        self.get(Index2::new(row_index, self.first_col_index(row_index)))
    }

    /// Returns the last col within a row, if it exists.
    #[must_use]
    pub fn last_col(&self, row_index: usize) -> Option<&T> {
        self.get(Index2::new(row_index, self.last_col_index(row_index)))
    }

    /// Check if a given position is the first row.
    #[must_use]
    pub fn is_first_row<I>(&self, index: I) -> bool
    where
        I: Into<Index2>,
    {
        let index = index.into();
        index.row == 0 && !self.data.is_empty()
    }

    /// Check if a given position is the last row.
    #[must_use]
    pub fn is_last_row<I>(&self, index: I) -> bool
    where
        I: Into<Index2>,
    {
        let index = index.into();
        !self.data.is_empty() && index.row == self.data.len().saturating_sub(1)
    }

    /// Check if a given position is the first column.
    #[must_use]
    pub fn is_first_col<I>(&self, index: I) -> bool
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match self.data.get(index.row) {
            Some(_) => index.col == 0,
            None => false,
        }
    }

    /// Check if a given position is the last column.
    #[must_use]
    pub fn is_last_col<I>(&self, index: I) -> bool
    where
        I: Into<Index2>,
    {
        let index = index.into();
        match self.data.get(index.row) {
            Some(row) => index.col >= row.len().saturating_sub(1),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_last_row() {
        let lines = Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(lines.last_row(), Some(&vec![4, 5, 6]));

        let lines: Jagged<usize> = Jagged::new(vec![]);
        assert_eq!(lines.last_row(), None);
    }

    #[test]
    fn test_last_col() {
        let lines = Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(lines.last_col(0), Some(&3));
        assert_eq!(lines.last_col(1), Some(&6));
        assert_eq!(lines.last_col(2), None);

        let lines: Jagged<usize> = Jagged::new(vec![]);
        assert_eq!(lines.last_col(0), None);
    }

    #[test]
    fn test_last_row_index() {
        let lines = Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(lines.last_row_index(), 1);

        let lines: Jagged<usize> = Jagged::new(vec![]);
        assert_eq!(lines.last_row_index(), 0);
    }

    #[test]
    fn test_last_col_index() {
        let lines = Jagged::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(lines.last_col_index(0), 2);
        assert_eq!(lines.last_col_index(1), 2);
        assert_eq!(lines.last_col_index(2), 0);

        let lines: Jagged<usize> = Jagged::new(vec![]);
        assert_eq!(lines.last_col_index(0), 0);
    }

    #[test]
    fn test_is_first_col() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert!(lines.is_first_col(Index2::new(0, 0)));
        assert!(lines.is_first_col(Index2::new(1, 0)));
        assert!(!lines.is_first_col(Index2::new(0, 1)));
    }

    #[test]
    fn test_is_last_col() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert!(!lines.is_last_col(Index2::new(0, 0)));
        assert!(lines.is_last_col(Index2::new(2, 2)));
    }

    #[test]
    fn test_is_first_row() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert!(lines.is_first_row(Index2::new(0, 0)));
        assert!(!lines.is_first_row(Index2::new(1, 0)));
    }

    #[test]
    fn test_is_last_row() {
        let data: Vec<Vec<i32>> = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let lines = Jagged::new(data);

        assert!(!lines.is_last_row(Index2::new(0, 0)));
        assert!(lines.is_last_row(Index2::new(2, 0)));
    }
}

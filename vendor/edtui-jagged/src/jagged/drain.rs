use crate::Index2;

use super::Jagged;

impl<T> Jagged<T> {
    /// Extracts the range from `start` to `end` (exclusive) and returns
    /// a newly allocated `Jagged<T>`.
    ///
    /// # Example
    /// ```
    /// let mut data = Jagged::from("hello world!");
    /// let drained = data.extract(&Index2::new(0, 0), &Index2::new(0, 5));
    /// assert_eq!(drained, Jagged::from("hello"));
    /// assert_eq!(data, Jagged::from(" world!"));
    /// ```
    #[must_use]
    pub fn extract(&mut self, start: &Index2, end: &Index2) -> Jagged<T> {
        let start_row = start.row;
        let end_row = end.row;
        if start > end {
            panic!("drain called with end < start");
        }
        if start.out_of_bounds(self) || end.out_of_bounds(self) {
            panic!("a drain index is out of bounds");
        }

        fn drain_into_jagged<U>(drain: std::vec::Drain<U>) -> Jagged<U> {
            Jagged::new(vec![drain.collect::<Vec<U>>()])
        }

        let mut drained = Jagged::<T>::default();
        if start_row == end_row {
            let Some(row) = self.data.get_mut(start_row) else {
                panic!("drain could not get row {}", start_row);
            };
            let mut drained_row = drain_into_jagged(row.drain(start.col..end.col));
            drained.append(&mut drained_row);
            // let x = drain.collect
            // let iter = drain.into_iter();
            // let jagged = Jagged::new(data);
            //
            // let drained_row = RowSlice::from(drain.collect::<Vec<T>>());
            // let j = Jagged::from_iter(drain.into_iter());
            // drained.push(drained_row);
        }

        drained
        // JaggedDrain {
        //     iter: JaggedIterator::new(&drained.clone()),
        // }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_obj_long() -> Jagged<char> {
        Jagged::from("hello world!\n\n123.")
    }

    #[test]
    fn test_extract() {
        // given
        let mut jagged = test_obj_long();
        // when
        let start = Index2::new(0, 0);
        let end = Index2::new(0, 2);
        let drained = jagged.extract(&start, &end);

        //then
        let expected_remaining = Jagged::from("llo world!\n\n123.");
        assert_eq!(jagged, expected_remaining);

        let expected_drained = Jagged::from("he");
        assert_eq!(drained, expected_drained);
    }
}

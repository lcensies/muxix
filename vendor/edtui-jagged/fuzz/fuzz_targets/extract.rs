#![no_main]

use edtui_jagged::{Index2, Jagged};
use libfuzzer_sys::fuzz_target;

#[derive(arbitrary::Arbitrary, Debug)]
pub struct ExtractionRange {
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
}

// run: cargo fuzz run extract
fuzz_target!(|data: Vec<ExtractionRange>| {
    for key in data {
        let start = Index2::new(key.start_row, key.start_col);
        let end = Index2::new(key.end_row, key.end_col);
        let mut data = get_input();
        let _ = data.extract(start..end);
    }
});

fn get_input() -> Jagged<char> {
    Jagged::from("first\n\nsecond\nthird")
}

use super::ReturnsValue;

pub struct Zero;
impl ReturnsValue for Zero {
    fn return_value(&self) -> usize {
        0
    }
}

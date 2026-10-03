use super::ReturnsValue;

pub struct One;
impl ReturnsValue for One {
    fn return_value(&self) -> usize {
        1
    }
}

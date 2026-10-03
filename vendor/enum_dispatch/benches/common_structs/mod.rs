use custom_derive::custom_derive;
use enum_derive::{enum_derive_util, EnumFromInner, EnumInnerAsTrait};
use enum_dispatch::enum_dispatch;

mod zero;
pub use self::zero::Zero;

mod one;
pub use self::one::One;

#[enum_dispatch]
pub trait ReturnsValue {
    fn return_value(&self) -> usize;
}

#[enum_dispatch(ReturnsValue)]
pub enum EnumDispatched {
    Zero,
    One,
}

custom_derive! {
    #[derive(EnumFromInner)]
    #[derive(EnumInnerAsTrait(pub inner -> &dyn ReturnsValue))]
    pub enum DynamicDispatched {
        Zero(Zero),
        One(One),
    }
}

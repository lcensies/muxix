//! The following benchmark tests create two trait objects and access them through one of the four
//! tested methods, repeating one million times.
//!
//! The result for `enum_dispatch` should be instant, since the return value is never used. Even
//! though this is not very representative of real code, this was done deliberately to demonstrate
//! the optimization opportunities available when using `enum_dispatch`. When using dynamic
//! dispatch, the compiler cannot perform optimizations like inlining or code removal -- those
//! become possible when using `match`-based dispatch.
//!
//! The `blackbox` benchmarks provide an example where the compiler is not able to remove code as
//! an optimization.

#![feature(test)]
extern crate test;

mod common_structs;
use crate::common_structs::{DynamicDispatched, EnumDispatched, One, ReturnsValue, Zero};

#[cfg(test)]
mod benches {
    use super::*;
    use test::Bencher;

    const ITERATIONS: usize = 1000000;

    #[bench]
    fn enumdispatch_compiler_optimized(b: &mut Bencher) {
        let dis0 = EnumDispatched::from(Zero);
        let dis1 = EnumDispatched::from(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                dis0.return_value();
                dis1.return_value();
            }
        })
    }

    #[bench]
    fn customderive_compiler_optimized(b: &mut Bencher) {
        let dis0 = DynamicDispatched::from(Zero);
        let dis1 = DynamicDispatched::from(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                dis0.inner().return_value();
                dis1.inner().return_value();
            }
        })
    }

    #[bench]
    fn boxdyn_compiler_optimized(b: &mut Bencher) {
        let dis0: Box<dyn ReturnsValue> = Box::new(Zero);
        let dis1: Box<dyn ReturnsValue> = Box::new(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                dis0.return_value();
                dis1.return_value();
            }
        })
    }

    #[bench]
    fn refdyn_compiler_optimized(b: &mut Bencher) {
        let dis0: &dyn ReturnsValue = &Zero;
        let dis1: &dyn ReturnsValue = &One;

        b.iter(|| {
            for _ in 0..ITERATIONS {
                dis0.return_value();
                dis1.return_value();
            }
        })
    }
}

fn main() {}

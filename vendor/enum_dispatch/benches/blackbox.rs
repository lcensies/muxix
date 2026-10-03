//! The following benchmark tests create two trait objects, access them through one of the four
//! tested methods, and use the result in a `test::black_box` call, repeating one million times.
//!
//! Unlike the `compiler-optimized` benchmark tests, the return values cannot be assumed unused
//! because of the `test::black_box` call. This results in virtually no change in performance for
//! the dynamic dispatched method calls, with `enum_dispatch` starting to show its real access
//! speed -- still several times faster than the alternatives.
//!
//! Real code with dynamic dispatch will likely use multiple trait objects whose types are
//! determined at runtime. That use-case is tested in the `homogeneous-vec` benchmarks.

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
    fn enumdispatch_blackbox(b: &mut Bencher) {
        let dis0 = EnumDispatched::from(Zero);
        let dis1 = EnumDispatched::from(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                test::black_box(dis0.return_value());
                test::black_box(dis1.return_value());
            }
        })
    }

    #[bench]
    fn customderive_blackbox(b: &mut Bencher) {
        let dis0 = DynamicDispatched::from(Zero);
        let dis1 = DynamicDispatched::from(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                test::black_box(dis0.inner().return_value());
                test::black_box(dis1.inner().return_value());
            }
        })
    }

    #[bench]

    fn boxdyn_blackbox(b: &mut Bencher) {
        let dis0: Box<dyn ReturnsValue> = Box::new(Zero);
        let dis1: Box<dyn ReturnsValue> = Box::new(One);

        b.iter(|| {
            for _ in 0..ITERATIONS {
                test::black_box(dis0.return_value());
                test::black_box(dis1.return_value());
            }
        })
    }

    #[bench]
    fn refdyn_blackbox(b: &mut Bencher) {
        let dis0: &dyn ReturnsValue = &Zero;
        let dis1: &dyn ReturnsValue = &One;

        b.iter(|| {
            for _ in 0..ITERATIONS {
                test::black_box(dis0.return_value());
                test::black_box(dis1.return_value());
            }
        })
    }
}

fn main() {}

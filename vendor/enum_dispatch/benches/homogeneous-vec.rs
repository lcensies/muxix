//! The following benchmark tests create a `Vec` of 1024 trait objects whose concrete types are
//! determined randomly at runtime, iterate over the `Vec` to access them through one of the four
//! tested methods, and use the result in a `test::black_box` call, repeating one million times.
//!
//! This test is most representative of real code -- it doesn't make sense to use dynamic trait
//! calls on a single object of known type! The dynamic methods take about twice as long to access,
//! but the performance for `enum_dispatch` is actually about the same as in the `homogeneous-vec`
//! benchmarks. This provides a really significant speed-up.

#![feature(test)]
extern crate test;

mod common_structs;
use crate::common_structs::{DynamicDispatched, EnumDispatched, One, ReturnsValue, Zero};

#[cfg(test)]
mod benches {
    use super::*;
    use rand::Rng;
    use test::Bencher;

    const ITERATIONS: usize = 1000000;
    const VEC_SIZE: usize = 1024;

    #[bench]
    fn enumdispatch_homogeneous_vec(b: &mut Bencher) {
        let mut rng = rand::thread_rng();

        let mut dispatches = vec![];
        for _ in 0..VEC_SIZE {
            if rng.gen() {
                dispatches.push(EnumDispatched::from(Zero));
            } else {
                dispatches.push(EnumDispatched::from(One));
            }
        }

        b.iter(|| {
            for i in 0..ITERATIONS {
                test::black_box(dispatches[i % VEC_SIZE].return_value());
            }
        })
    }

    #[bench]
    fn customderive_homogeneous_vec(b: &mut Bencher) {
        let mut rng = rand::thread_rng();

        let mut dispatches = vec![];
        for _ in 0..VEC_SIZE {
            if rng.gen() {
                dispatches.push(DynamicDispatched::from(Zero));
            } else {
                dispatches.push(DynamicDispatched::from(One));
            }
        }

        b.iter(|| {
            for i in 0..ITERATIONS {
                test::black_box(dispatches[i % VEC_SIZE].inner().return_value());
            }
        })
    }

    #[bench]
    fn boxdyn_homogeneous_vec(b: &mut Bencher) {
        let mut rng = rand::thread_rng();

        let mut dispatches: Vec<Box<dyn ReturnsValue>> = vec![];
        for _ in 0..VEC_SIZE {
            if rng.gen() {
                dispatches.push(Box::new(Zero));
            } else {
                dispatches.push(Box::new(One));
            }
        }

        b.iter(|| {
            for i in 0..ITERATIONS {
                test::black_box(dispatches[i % VEC_SIZE].return_value());
            }
        })
    }

    #[bench]
    fn refdyn_homogeneous_vec(b: &mut Bencher) {
        let mut rng = rand::thread_rng();

        let t0 = Zero;
        let t1 = One;

        let mut dispatches: Vec<&dyn ReturnsValue> = vec![];
        for _ in 0..VEC_SIZE {
            if rng.gen() {
                dispatches.push(&t0);
            } else {
                dispatches.push(&t1);
            }
        }

        b.iter(|| {
            for i in 0..ITERATIONS {
                test::black_box(dispatches[i % VEC_SIZE].return_value());
            }
        })
    }
}

fn main() {}

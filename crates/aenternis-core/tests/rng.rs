//! Integration tests for the RNG.
//!
//! Two layers of guarantees we care about:
//!
//! 1. **Determinism** — a generator built from the same inputs always
//!    produces the same stream. This is the contract the entire bit-identity
//!    harness rests on.
//! 2. **Independence** — generators built from different inputs produce
//!    *visibly* different streams. We don't need cryptographic separation,
//!    just confidence that two adjacent seeds, ticks, or coordinates don't
//!    alias.

use aenternis_core::{Coord, Rng};

#[test]
fn new_is_deterministic() {
    let mut a = Rng::new(42);
    let mut b = Rng::new(42);
    for _ in 0..1000 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn new_different_seeds_diverge() {
    let mut a = Rng::new(1);
    let mut b = Rng::new(2);
    let a_first = a.next_u32();
    let b_first = b.next_u32();
    assert_ne!(a_first, b_first);
}

#[test]
fn for_world_is_deterministic() {
    let mut a = Rng::for_world(1234);
    let mut b = Rng::for_world(1234);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn for_cell_at_tick_is_deterministic() {
    let coord = Coord::new(3, -7, 11);
    let mut a = Rng::for_cell_at_tick(0xDEAD_BEEF, 42, coord);
    let mut b = Rng::for_cell_at_tick(0xDEAD_BEEF, 42, coord);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn for_cell_at_tick_independent_across_coords() {
    let mut a = Rng::for_cell_at_tick(0, 0, Coord::new(0, 0, 0));
    let mut b = Rng::for_cell_at_tick(0, 0, Coord::new(1, 0, 0));
    let mut c = Rng::for_cell_at_tick(0, 0, Coord::new(0, 1, 0));
    let mut d = Rng::for_cell_at_tick(0, 0, Coord::new(0, 0, 1));

    let outputs: [u32; 4] = [a.next_u32(), b.next_u32(), c.next_u32(), d.next_u32()];
    // All four first outputs should be distinct.
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(outputs[i], outputs[j], "coords {i} and {j} aliased");
        }
    }
}

#[test]
fn for_cell_at_tick_independent_across_ticks() {
    let coord = Coord::new(5, 5, 5);
    let mut a = Rng::for_cell_at_tick(0, 0, coord);
    let mut b = Rng::for_cell_at_tick(0, 1, coord);
    let mut c = Rng::for_cell_at_tick(0, 2, coord);

    let outputs: [u32; 3] = [a.next_u32(), b.next_u32(), c.next_u32()];
    for i in 0..3 {
        for j in (i + 1)..3 {
            assert_ne!(outputs[i], outputs[j], "ticks {i} and {j} aliased");
        }
    }
}

#[test]
fn for_cell_at_tick_independent_across_world_seeds() {
    let coord = Coord::new(0, 0, 0);
    let mut a = Rng::for_cell_at_tick(0, 0, coord);
    let mut b = Rng::for_cell_at_tick(1, 0, coord);
    assert_ne!(a.next_u32(), b.next_u32());
}

#[test]
fn next_u32_advances_state() {
    let mut r = Rng::new(7);
    let a = r.next_u32();
    let b = r.next_u32();
    // Probability of equal back-to-back outputs from PCG ≈ 1/2^32, negligible.
    assert_ne!(a, b);
}

#[test]
fn next_f32_in_unit_range() {
    let mut r = Rng::new(99);
    for _ in 0..1000 {
        let x = r.next_f32();
        assert!(
            (0.0..1.0).contains(&x),
            "next_f32 produced {x}, out of range"
        );
    }
}

#[test]
fn next_f32_distribution_is_roughly_uniform() {
    let mut r = Rng::new(123);
    let mut buckets = [0u32; 10];
    let n = 10_000;
    for _ in 0..n {
        let x = r.next_f32();
        let idx = ((x * 10.0) as usize).min(9);
        buckets[idx] += 1;
    }
    // Each bucket should hold ~1000. Allow generous slack (≥ 600, ≤ 1400).
    for (i, &count) in buckets.iter().enumerate() {
        assert!(
            (600..=1400).contains(&count),
            "bucket {i} has {count} items, expected ~1000",
        );
    }
}

#[test]
fn stochastic_floor_zero_for_non_positive() {
    let mut r = Rng::new(0);
    assert_eq!(r.stochastic_floor(0.0), 0);
    assert_eq!(r.stochastic_floor(-0.5), 0);
    assert_eq!(r.stochastic_floor(-100.0), 0);
    assert_eq!(r.stochastic_floor(f32::NAN), 0);
    assert_eq!(r.stochastic_floor(f32::NEG_INFINITY), 0);
}

#[test]
fn stochastic_floor_integer_input_is_exact() {
    // u8 → f32 / u32 are both lossless `From` conversions, so we can
    // skip the `as` cast (and clippy::cast_precision_loss along with it).
    let mut r = Rng::new(0);
    for v in 1u8..=10 {
        assert_eq!(r.stochastic_floor(f32::from(v)), u32::from(v));
    }
}

#[test]
fn stochastic_floor_fractional_returns_neighbors() {
    let mut r = Rng::new(0);
    for _ in 0..100 {
        let result = r.stochastic_floor(2.5);
        assert!(result == 2 || result == 3, "got {result} for input 2.5");
    }
}

#[test]
fn stochastic_floor_expectation_matches_input() {
    // Empirical mean of stochastic_floor(0.3) over many samples should
    // converge to 0.3. With n = 10_000 samples we expect SE ≈ 0.005, so
    // a tolerance of ±0.05 is comfortable. `sum` stays in u32 range
    // (max here is n * 1 = 10_000), so f64::from is lossless.
    let mut r = Rng::new(42);
    let n: u32 = 10_000;
    let mut sum: u32 = 0;
    for _ in 0..n {
        sum += r.stochastic_floor(0.3);
    }
    let mean = f64::from(sum) / f64::from(n);
    assert!(
        (mean - 0.3).abs() < 0.05,
        "stochastic_floor(0.3) mean was {mean}, expected ≈ 0.3",
    );
}

#[test]
fn stochastic_floor_expectation_matches_input_high_value() {
    // Same property but at a more realistic simulation rate, e.g. 7.4.
    // Max sum here is n * 8 = 80_000, still safely under u32::MAX.
    let mut r = Rng::new(2024);
    let n: u32 = 10_000;
    let mut sum: u32 = 0;
    for _ in 0..n {
        sum += r.stochastic_floor(7.4);
    }
    let mean = f64::from(sum) / f64::from(n);
    assert!(
        (mean - 7.4).abs() < 0.1,
        "stochastic_floor(7.4) mean was {mean}, expected ≈ 7.4",
    );
}

#[test]
fn rng_clone_diverges_after_independent_advance() {
    let r = Rng::new(99);
    let mut a = r.clone();
    // After cloning into `a`, the original `r` is moved into `b`. This
    // exercises both the `Clone` impl (via `a`) and the `Move` semantics
    // (via `b`) without leaving a redundant clone for clippy to flag.
    let mut b = r;
    // Same starting state → same next output
    assert_eq!(a.next_u32(), b.next_u32());
    // Advance only `a`; the streams diverge from now on.
    let _ = a.next_u32();
    assert_ne!(a.next_u32(), b.next_u32());
}

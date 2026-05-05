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

use aenternis_core::rng::{cell_seed_xs32, cell_tick_seed_xs32};
use aenternis_core::{Coord, Rng, RngKind};

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

// ===== xorshift32 backend (matches JS prototype 9-B) ========================

#[test]
fn xs32_matches_js_reference_stream() {
    // Reference stream from JS prototype 9-B's `makeRng(1)`, verified by
    // tracing the three xorshift phases (^=<<13, ^=>>17, ^=<<5) by hand.
    // The Rust implementation is a verbatim port and must reproduce these
    // values bit-for-bit; the test guards against accidental drift in shift
    // counts or operation order.
    let mut r = Rng::new_xs32(1);
    let expected = [270_369u32, 67_634_689, 2_647_435_461];
    for &want in &expected {
        assert_eq!(r.next_u32(), want);
    }
}

#[test]
fn xs32_seed_zero_is_forced_to_one() {
    // xorshift cannot escape from all-zeros; JS forces seed 0 to 1.
    let mut a = Rng::new_xs32(0);
    let mut b = Rng::new_xs32(1);
    for _ in 0..16 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn xs32_for_cell_at_tick_is_deterministic() {
    let coord = Coord::new(3, -7, 11);
    let mut a = Rng::for_cell_at_tick_with_kind(RngKind::Xorshift32, 0xDEAD_BEEF, 42, coord);
    let mut b = Rng::for_cell_at_tick_with_kind(RngKind::Xorshift32, 0xDEAD_BEEF, 42, coord);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn xs32_for_cell_at_tick_differs_from_pcg() {
    // The two backends must produce different streams for the same inputs —
    // otherwise the comparison checkbox is meaningless.
    let coord = Coord::new(0, 0, 0);
    let mut pcg = Rng::for_cell_at_tick_with_kind(RngKind::Pcg, 1234, 0, coord);
    let mut xs = Rng::for_cell_at_tick_with_kind(RngKind::Xorshift32, 1234, 0, coord);
    let mut diverged = false;
    for _ in 0..20 {
        if pcg.next_u32() != xs.next_u32() {
            diverged = true;
            break;
        }
    }
    assert!(diverged, "PCG and Xorshift32 streams should differ");
}

#[test]
fn xs32_cell_seed_matches_js_math_imul_at_origin() {
    // Reference value from JS prototype 9-B with the `Math.imul` checkbox
    // enabled — i.e. the hash with f64 precision loss removed:
    //   cellSeed(1234, 0, 0, 0, true) === 535601943
    // The Rust port uses `wrapping_mul` (exact u32 mod 2^32), which is
    // semantically what `Math.imul(a, b) >>> 0` computes in JS. The two
    // must therefore agree to the bit. If this test fails, the divergence
    // we've been chasing in the simulation has its root here, in the
    // very first hash call.
    assert_eq!(cell_seed_xs32(1234, Coord::new(0, 0, 0)), 535_601_943);
}

#[test]
fn xs32_cell_seed_independent_across_coords() {
    // Different coordinates must produce different per-cell seeds; otherwise
    // adjacent cells would get identical thermal microstates.
    let s = cell_seed_xs32(1234, Coord::new(0, 0, 0));
    let sx = cell_seed_xs32(1234, Coord::new(1, 0, 0));
    let sy = cell_seed_xs32(1234, Coord::new(0, 1, 0));
    let sz = cell_seed_xs32(1234, Coord::new(0, 0, 1));
    assert_ne!(s, sx);
    assert_ne!(s, sy);
    assert_ne!(s, sz);
    assert_ne!(sx, sy);
    assert_ne!(sy, sz);
}

#[test]
fn xs32_cell_tick_seed_advances_with_tick() {
    let coord = Coord::new(5, -3, 2);
    let s0 = cell_tick_seed_xs32(42, 0, coord);
    let s1 = cell_tick_seed_xs32(42, 1, coord);
    let s2 = cell_tick_seed_xs32(42, 2, coord);
    assert_ne!(s0, s1);
    assert_ne!(s1, s2);
}

#[test]
fn xs32_next_f32_in_unit_range() {
    let mut r = Rng::new_xs32(99);
    for _ in 0..1000 {
        let x = r.next_f32();
        assert!(
            (0.0..1.0).contains(&x),
            "xs32 next_f32 produced {x}, out of range"
        );
    }
}

#[test]
fn xs32_stochastic_floor_expectation_matches_input() {
    // Same statistical contract as PCG path — the dispatcher in
    // `stochastic_floor` must not bias the result.
    let mut r = Rng::new_xs32(42);
    let n: u32 = 10_000;
    let mut sum: u32 = 0;
    for _ in 0..n {
        sum += r.stochastic_floor(0.3);
    }
    let mean = f64::from(sum) / f64::from(n);
    assert!(
        (mean - 0.3).abs() < 0.05,
        "xs32 stochastic_floor(0.3) mean was {mean}, expected ≈ 0.3",
    );
}

// ===== f64 precision path (matches JS prototype 9-B) =======================

#[test]
fn next_f64_in_unit_range() {
    let mut r = Rng::new(123);
    for _ in 0..1000 {
        let x = r.next_f64();
        assert!(
            (0.0..1.0).contains(&x),
            "next_f64 produced {x}, out of range",
        );
    }
}

#[test]
#[allow(clippy::float_cmp)] // injectivity check, exact equality is the contract
fn next_f64_uses_full_32_bit_entropy() {
    // The conversion is `u32 / 2^32` in `f64`; bits and divisor are exact
    // in `f64`, so two distinct `u32` outputs must produce two distinct
    // `f64` values (no collision through rounding). Exact `!=` is exactly
    // what we want here — the lint is normally right that `f64 == f64` is
    // suspicious, but for an injectivity check it's the correct comparison.
    let mut r1 = Rng::new(1);
    let mut r2 = Rng::new(1);
    let _a = r1.next_u32(); // advance r1 to differ from r2
    let f1 = r1.next_f64();
    let f2 = r2.next_f64();
    assert_ne!(f1, f2);
}

#[test]
fn stochastic_floor_f64_expectation_matches_input() {
    // Same contract as f32 path: `mean → frac` over many samples.
    let mut r = Rng::new(42);
    let n: u32 = 10_000;
    let mut sum: u32 = 0;
    for _ in 0..n {
        sum += r.stochastic_floor_f64(0.3);
    }
    let mean = f64::from(sum) / f64::from(n);
    assert!(
        (mean - 0.3).abs() < 0.05,
        "stochastic_floor_f64(0.3) mean was {mean}, expected ≈ 0.3",
    );
}

#[test]
fn stochastic_floor_f64_zero_for_non_positive() {
    let mut r = Rng::new(0);
    assert_eq!(r.stochastic_floor_f64(0.0), 0);
    assert_eq!(r.stochastic_floor_f64(-0.5), 0);
    assert_eq!(r.stochastic_floor_f64(f64::NAN), 0);
    assert_eq!(r.stochastic_floor_f64(f64::NEG_INFINITY), 0);
}

#[test]
fn f32_and_f64_paths_diverge_on_constructed_boundary() {
    // The disagreement window for `frac = 0.15` lives in `u32 bits` space
    // between `0x26666666` (where `r_f64 < 0.14999999999...` flips to
    // `>=`) and `0x266666FF` (where the 24-bit-truncated `r_f32` is still
    // below the f32 representation `0.15000000596...`). Pick a value
    // squarely inside that ~150-wide window and verify both paths see
    // opposite sides of their respective `frac` boundaries — this is the
    // precision difference, isolated from the RNG stream.
    let bits = 0x2666_6690_u32;

    #[allow(clippy::cast_precision_loss)]
    let r_f32 = ((bits >> 8) as f32) * (1.0 / 16_777_216.0);
    let r_f64 = f64::from(bits) / 4_294_967_296.0;

    let frac_f32 = 0.15_f32;
    let frac_f64 = 0.15_f64;

    // f32 representation of 0.15 ≈ 0.15000000596; r_f32 ≈ 0.14999998.
    assert!(
        r_f32 < frac_f32,
        "f32 path: r_f32 = {r_f32} should be < {frac_f32}",
    );
    // f64 representation of 0.15 ≈ 0.14999999999999999; r_f64 ≈ 0.15000004.
    assert!(
        r_f64 >= frac_f64,
        "f64 path: r_f64 = {r_f64} should be >= {frac_f64}",
    );
}

//! Integration tests for the RNG.
//!
//! Three layers of guarantees:
//!
//! 1. **Determinism** — a generator built from the same inputs always
//!    produces the same stream. This is the contract the bit-identity
//!    harness against JS prototype 9-B rests on.
//! 2. **Independence** — generators built from different inputs produce
//!    *visibly* different streams. Not cryptographic separation, just
//!    confidence that two adjacent seeds, ticks, or coordinates don't
//!    alias.
//! 3. **Bit-identity snapshots** — concrete `u32` outputs locked in for
//!    fixed inputs, so any structural change to the xorshift mixer or
//!    the `cell_seed` / `cell_tick_seed` hash chains breaks at least
//!    one assertion. Change-detector tests by design.

use aenternis_core::rng::{cell_seed, cell_tick_seed};
use aenternis_core::{Coord, Rng};

// ----- determinism / independence -------------------------------------------

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
    assert_ne!(a.next_u32(), b.next_u32());
}

#[test]
fn new_seed_zero_is_forced_to_one() {
    // xorshift cannot escape from all-zeros; JS forces seed 0 to 1.
    let mut a = Rng::new(0);
    let mut b = Rng::new(1);
    for _ in 0..16 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn for_cell_at_tick_is_deterministic() {
    let coord = Coord::new(3, -7, 11);
    let mut a = Rng::for_cell_at_tick(0xDEAD_BEEF, 42, coord, 0);
    let mut b = Rng::for_cell_at_tick(0xDEAD_BEEF, 42, coord, 0);
    for _ in 0..100 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

#[test]
fn for_cell_at_tick_independent_across_coords() {
    let mut a = Rng::for_cell_at_tick(0, 0, Coord::new(0, 0, 0), 0);
    let mut b = Rng::for_cell_at_tick(0, 0, Coord::new(1, 0, 0), 0);
    let mut c = Rng::for_cell_at_tick(0, 0, Coord::new(0, 1, 0), 0);
    let mut d = Rng::for_cell_at_tick(0, 0, Coord::new(0, 0, 1), 0);

    let outputs: [u32; 4] = [a.next_u32(), b.next_u32(), c.next_u32(), d.next_u32()];
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(outputs[i], outputs[j], "coords {i} and {j} aliased");
        }
    }
}

#[test]
fn for_cell_at_tick_independent_across_ticks() {
    let coord = Coord::new(5, 5, 5);
    let mut a = Rng::for_cell_at_tick(0, 0, coord, 0);
    let mut b = Rng::for_cell_at_tick(0, 1, coord, 0);
    let mut c = Rng::for_cell_at_tick(0, 2, coord, 0);

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
    let mut a = Rng::for_cell_at_tick(0, 0, coord, 0);
    let mut b = Rng::for_cell_at_tick(1, 0, coord, 0);
    assert_ne!(a.next_u32(), b.next_u32());
}

#[test]
fn for_cell_at_tick_independent_across_domains() {
    // Two different `domain` values for the same `(world_seed, tick,
    // coord)` must produce visibly distinct streams. This is the
    // contract that lets stochastic operations within one tick draw
    // independently without correlating their outputs.
    let coord = Coord::new(7, -2, 3);
    let mut a = Rng::for_cell_at_tick(0xCAFE, 11, coord, 0);
    let mut b = Rng::for_cell_at_tick(0xCAFE, 11, coord, 1);
    let mut c = Rng::for_cell_at_tick(0xCAFE, 11, coord, 2);
    let outputs: [u32; 3] = [a.next_u32(), b.next_u32(), c.next_u32()];
    for i in 0..3 {
        for j in (i + 1)..3 {
            assert_ne!(outputs[i], outputs[j], "domains {i} and {j} aliased");
        }
    }
}

#[test]
fn for_cell_at_tick_domain_zero_is_pre_domain_stream() {
    // `domain == 0` is the default and must be bit-identical to the
    // pre-domain hash output. The reference values below are the same
    // ones pinned in `for_cell_at_tick_reference_stream_for_nonzero_inputs`
    // and were captured before the `domain` parameter existed —
    // domain=0 must reproduce them exactly.
    let mut r = Rng::for_cell_at_tick(0xDEAD_BEEF, 5, Coord::new(3, -7, 11), 0);
    let expected = [
        2_677_452_818_u32,
        2_064_535_512,
        3_965_174_498,
        2_507_838_471,
    ];
    for (i, &want) in expected.iter().enumerate() {
        assert_eq!(r.next_u32(), want, "domain=0 stream drifted at [{i}]");
    }
}

#[test]
fn next_u32_advances_state() {
    let mut r = Rng::new(7);
    let a = r.next_u32();
    let b = r.next_u32();
    assert_ne!(a, b);
}

#[test]
fn rng_clone_diverges_after_independent_advance() {
    let r = Rng::new(99);
    let mut a = r.clone();
    let mut b = r;
    assert_eq!(a.next_u32(), b.next_u32());
    let _ = a.next_u32();
    assert_ne!(a.next_u32(), b.next_u32());
}

// ----- next_f64 --------------------------------------------------------------

#[test]
fn next_f64_in_unit_range() {
    let mut r = Rng::new(123);
    for _ in 0..1000 {
        let x = r.next_f64();
        assert!(
            (0.0..1.0).contains(&x),
            "next_f64 produced {x}, out of range"
        );
    }
}

#[test]
#[allow(clippy::float_cmp)] // injectivity check, exact equality is the contract
fn next_f64_uses_full_32_bit_entropy() {
    // The conversion is `u32 / 2^32` in `f64`; bits and divisor are
    // exact in `f64`, so two distinct `u32` outputs must produce two
    // distinct `f64` values (no collision through rounding).
    let mut r1 = Rng::new(1);
    let mut r2 = Rng::new(1);
    let _a = r1.next_u32();
    let f1 = r1.next_f64();
    let f2 = r2.next_f64();
    assert_ne!(f1, f2);
}

// ----- stochastic_floor ------------------------------------------------------

#[test]
fn stochastic_floor_zero_for_non_positive() {
    let mut r = Rng::new(1);
    assert_eq!(r.stochastic_floor(0.0), 0);
    assert_eq!(r.stochastic_floor(-0.5), 0);
    assert_eq!(r.stochastic_floor(-100.0), 0);
    assert_eq!(r.stochastic_floor(f64::NAN), 0);
    assert_eq!(r.stochastic_floor(f64::NEG_INFINITY), 0);
}

#[test]
fn stochastic_floor_integer_input_is_exact() {
    let mut r = Rng::new(1);
    for v in 1u32..=10 {
        assert_eq!(r.stochastic_floor(f64::from(v)), v);
    }
}

#[test]
fn stochastic_floor_fractional_returns_neighbors() {
    let mut r = Rng::new(1);
    for _ in 0..100 {
        let result = r.stochastic_floor(2.5);
        assert!(result == 2 || result == 3, "got {result} for input 2.5");
    }
}

#[test]
fn stochastic_floor_expectation_matches_input() {
    // Empirical mean over many samples should converge to the input.
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
fn stochastic_floor_uses_subtraction_for_fractional_part() {
    // `frac = value - whole` must use subtraction, not addition. With
    // value = 2.5 and whole = 2.0:
    //   native frac = 0.5  → returns 2 when r >= 0.5, else 3
    //   `-` → `+` mutated frac = 4.5 → r < 4.5 always true → always 3
    //
    // Seed 12345 produces next_f64() ≈ 0.777, so the native path takes
    // the `r < frac` = false branch and returns exactly 2. The mutation
    // always returns 3, so this test fires deterministically.
    let mut r = Rng::new(12_345);
    assert_eq!(r.stochastic_floor(2.5), 2);
}

// ----- bit-identity snapshots ------------------------------------------------

#[test]
fn xorshift32_matches_js_reference_stream() {
    // Reference stream from JS prototype 9-B's `makeRng(1)`, verified by
    // tracing the three xorshift phases (^=<<13, ^=>>17, ^=<<5) by hand.
    // Any drift in shift counts or operation order breaks this.
    let mut r = Rng::new(1);
    let expected = [270_369u32, 67_634_689, 2_647_435_461];
    for (i, &want) in expected.iter().enumerate() {
        assert_eq!(r.next_u32(), want, "xs32 output[{i}] drifted");
    }
}

#[test]
fn xorshift32_seed_42_reference_stream() {
    // Catches `^` ↔ `|` and `>>` ↔ `<<` mutations in `next_u32` that
    // pass the determinism/distribution behavioral tests but produce
    // a different bit pattern.
    let mut r = Rng::new(42);
    let expected = [11_355_432_u32, 2_836_018_348, 476_557_059, 3_648_046_016];
    for (i, &want) in expected.iter().enumerate() {
        assert_eq!(r.next_u32(), want, "xs32 seed=42 output[{i}] drifted");
    }
}

#[test]
fn for_cell_at_tick_reference_stream_for_nonzero_inputs() {
    // Uses world_seed, tick, and all coord components non-zero so the
    // `cell_tick_seed` hash mix has every input slot active. `domain
    // == 0` is the pre-domain reference path, so this snapshot remains
    // a stability anchor for the JS-9B-parity stream.
    let mut r = Rng::for_cell_at_tick(0xDEAD_BEEF, 5, Coord::new(3, -7, 11), 0);
    let expected = [
        2_677_452_818_u32,
        2_064_535_512,
        3_965_174_498,
        2_507_838_471,
    ];
    for (i, &want) in expected.iter().enumerate() {
        assert_eq!(r.next_u32(), want, "for_cell_at_tick output[{i}] drifted");
    }
}

#[test]
fn cell_seed_matches_js_math_imul_at_origin() {
    // JS prototype 9-B with the `Math.imul` checkbox enabled — i.e. the
    // hash with f64 precision loss removed — gives `cellSeed(1234, 0, 0, 0)
    // === 535601943`. The Rust port uses `wrapping_mul` (exact u32 mod
    // 2^32), which is semantically what `Math.imul(a, b) >>> 0` computes.
    assert_eq!(cell_seed(1234, Coord::new(0, 0, 0)), 535_601_943);
}

#[test]
fn cell_seed_independent_across_coords() {
    let s = cell_seed(1234, Coord::new(0, 0, 0));
    let sx = cell_seed(1234, Coord::new(1, 0, 0));
    let sy = cell_seed(1234, Coord::new(0, 1, 0));
    let sz = cell_seed(1234, Coord::new(0, 0, 1));
    assert_ne!(s, sx);
    assert_ne!(s, sy);
    assert_ne!(s, sz);
    assert_ne!(sx, sy);
    assert_ne!(sy, sz);
}

#[test]
fn cell_tick_seed_matches_reference_for_nonzero_inputs() {
    // The hash mixes via `h ^= h >> 16` (last line of `cell_tick_seed`
    // for `domain == 0`). Pinning exact outputs catches shift-direction
    // and operator mutations that pass coarser determinism tests.
    // `domain == 0` is required for these reference values — the salted
    // path is checked separately.
    assert_eq!(cell_tick_seed(0, 0, Coord::new(0, 0, 0), 0), 3_127_886_501);
    assert_eq!(
        cell_tick_seed(1234, 3, Coord::new(2, 5, 7), 0),
        3_577_743_044
    );
}

#[test]
fn cell_tick_seed_advances_with_tick() {
    let coord = Coord::new(5, -3, 2);
    let s0 = cell_tick_seed(42, 0, coord, 0);
    let s1 = cell_tick_seed(42, 1, coord, 0);
    let s2 = cell_tick_seed(42, 2, coord, 0);
    assert_ne!(s0, s1);
    assert_ne!(s1, s2);
}

#[test]
fn cell_tick_seed_domain_separation() {
    // Different domains yield different seeds for the same
    // `(world_seed, tick, coord)`. This is the salt's contract.
    let coord = Coord::new(2, -1, 3);
    let s0 = cell_tick_seed(0xBEEF, 7, coord, 0);
    let s1 = cell_tick_seed(0xBEEF, 7, coord, 1);
    let s2 = cell_tick_seed(0xBEEF, 7, coord, 2);
    assert_ne!(s0, s1, "domain 0 and 1 collided");
    assert_ne!(s1, s2, "domain 1 and 2 collided");
    assert_ne!(s0, s2, "domain 0 and 2 collided");
}

#[test]
fn cell_tick_seed_salted_path_matches_reference() {
    // Reference snapshot for `domain != 0`. Pins the post-salt mix
    // (`h.wrapping_add(domain).wrapping_mul(K) ^ (h >> 15)`) to exact
    // output values, so any drift in the shift direction, the XOR
    // operator, or the multiplier breaks at least one assertion.
    // Two different domains for the same `(world_seed, tick, coord)`
    // diverge to confirm the salt's diffusion across the seed.
    assert_eq!(cell_tick_seed(0, 0, Coord::new(0, 0, 0), 1), 292_749_432);
    assert_eq!(
        cell_tick_seed(1234, 3, Coord::new(2, 5, 7), 1),
        2_729_484_746
    );
    assert_eq!(cell_tick_seed(1234, 3, Coord::new(2, 5, 7), 2), 31_935_216);
}

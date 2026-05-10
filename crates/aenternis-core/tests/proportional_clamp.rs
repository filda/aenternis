//! Direct unit tests for [`cell::proportional_clamp`].
//!
//! `proportional_clamp` is the in-place sibling of
//! [`crate::tick::combined_clamped`] (called inside
//! `compute_natural_rates` when a cell's natural-rate sum exceeds its
//! energy budget). Same Largest-Remainder algorithm, same statistical
//! isotropy contract — see the contracts test for `combined_clamped`
//! for the long-form derivation. These tests pin down behavior at
//! specific hand-constructed inputs and verify the property contracts
//! (conservation, non-exceedance, fast path, statistical balance) for
//! `proportional_clamp` independently.

use aenternis_core::cell::proportional_clamp;
use aenternis_core::Coord;

const D: usize = 6;
const ZERO: [u32; D] = [0; D];

// All cases below use a fixed `(world_seed, rng_tick, coord)` triple
// so that the function is purely a function of `(rates, cap)` here.
// The leftover tie-break inside the clamp path still uses RNG, just
// seeded reproducibly.
fn pc(rates: &mut [u32; D], cap: u32) {
    proportional_clamp(rates, cap, 0, 0, Coord::ORIGIN);
}

// ----- early-out paths -------------------------------------------------------

#[test]
fn no_op_when_total_below_cap() {
    let mut rates = [1, 2, 3, 0, 0, 0];
    pc(&mut rates, 100);
    assert_eq!(rates, [1, 2, 3, 0, 0, 0]);
}

#[test]
fn no_op_when_total_equals_cap() {
    let mut rates = [2, 2, 2, 0, 0, 0];
    pc(&mut rates, 6);
    assert_eq!(rates, [2, 2, 2, 0, 0, 0]);
}

#[test]
fn no_op_when_all_rates_zero_even_with_zero_cap() {
    // total == 0 short-circuits the function before any division happens.
    let mut rates = ZERO;
    pc(&mut rates, 0);
    assert_eq!(rates, ZERO);
}

// ----- clamp + leftover distribution -----------------------------------------

#[test]
fn clamp_distributes_leftover_across_directions_with_remainder() {
    // total = 10, cap = 8, scale = 0.8.
    // [5, 1, 1, 1, 1, 1] → floor [4, 0, 0, 0, 0, 0], leftover = 4.
    // Five tail directions tie at frac 0.8; Largest-Remainder gives +1
    // to four of them. The exact loser among the ties is RNG-determined
    // (and *must* be — that's what the algorithm exists to do), so we
    // assert only the structural shape: head stays 4, four tails become
    // 1, exactly one tail stays 0, post-clamp sum equals cap.
    let mut rates = [5, 1, 1, 1, 1, 1];
    pc(&mut rates, 8);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 8);
    assert_eq!(rates[0], 4, "head direction should keep its floor of 4");
    let tail_ones = rates[1..].iter().filter(|&&v| v == 1).count();
    let tail_zeros = rates[1..].iter().filter(|&&v| v == 0).count();
    assert_eq!(tail_ones, 4);
    assert_eq!(tail_zeros, 1);
}

#[test]
fn clamp_distributes_leftover_isotropically_across_ties() {
    // total = 5, cap = 4, scale = 0.8.
    // [2, 2, 1, 0, 0, 0] → floor [1, 1, 0, 0, 0, 0], leftover = 2.
    // fracs [0.6, 0.6, 0.8, 0, 0, 0]: idx 2 wins first +1 (highest
    // frac), then one of {idx 0, idx 1} gets the second +1 — shuffle
    // decides which.
    let mut rates = [2, 2, 1, 0, 0, 0];
    pc(&mut rates, 4);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 4);
    assert_eq!(rates[2], 1, "idx with highest frac gets +1");
    assert_eq!(rates[3..], [0, 0, 0]);
    let head_pair = (rates[0], rates[1]);
    assert!(
        head_pair == (2, 1) || head_pair == (1, 2),
        "expected one of (2,1) or (1,2), got {head_pair:?}",
    );
}

#[test]
fn clamp_distributes_leftover_to_combined_positive_directions_even_when_all_floor_to_zero() {
    // Every rate floors to zero (1 * 5/6 = 0.833 → 0). All six tied at
    // frac 0.833 — five of them get +1 via shuffle; one stays at 0.
    // Post-clamp sum reaches cap exactly (5).
    // (Pre-fix behaviour: forfeited the leftover and returned all
    // zeros — a conservation violation under symmetric input.)
    let mut rates = [1, 1, 1, 1, 1, 1];
    pc(&mut rates, 5);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 5);
    let ones = rates.iter().filter(|&&v| v == 1).count();
    let zeros = rates.iter().filter(|&&v| v == 0).count();
    assert_eq!(ones, 5);
    assert_eq!(zeros, 1);
    // Per-direction non-exceedance: clamped[i] <= original[i] = 1.
    for &v in &rates {
        assert!(v <= 1, "non-exceedance violated: {v} > 1");
    }
}

#[test]
fn clamp_keeps_zero_directions_zero_when_some_survive_floor() {
    // total = 10, cap = 7, scale = 0.7 → floor [3, 3, 0, 0, 0, 0],
    // fracs [0.5, 0.5, 0, 0, 0, 0], leftover = 1. Goes to one of
    // {idx 0, idx 1}. Dirs 2..5 stay 0 (their original was 0).
    let mut rates = [5, 5, 0, 0, 0, 0];
    pc(&mut rates, 7);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 7);
    assert_eq!(rates[2..], [0, 0, 0, 0]);
    let head_pair = (rates[0], rates[1]);
    assert!(
        head_pair == (4, 3) || head_pair == (3, 4),
        "expected one of (4,3) or (3,4), got {head_pair:?}",
    );
}

#[test]
fn clamp_post_sum_equals_cap_when_at_least_one_direction_survives_floor() {
    // Single non-zero direction → no tie-break needed, deterministic.
    let mut rates = [10, 0, 0, 0, 0, 0];
    pc(&mut rates, 7);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 7);
}

// ----- property contracts ---------------------------------------------------

#[test]
fn clamp_post_sum_never_exceeds_cap() {
    let cases: &[([u32; D], u32)] = &[
        ([10, 0, 0, 0, 0, 0], 5),
        ([3, 3, 3, 3, 3, 3], 10),
        ([100, 50, 25, 12, 6, 3], 50),
        ([7, 0, 5, 0, 3, 0], 8),
    ];
    for &(initial, cap) in cases {
        let mut rates = initial;
        pc(&mut rates, cap);
        let sum: u32 = rates.iter().sum();
        assert!(
            sum <= cap,
            "post-clamp sum {sum} > cap {cap} for initial={initial:?}",
        );
    }
}

#[test]
fn clamp_non_exceedance_per_direction() {
    // After clamp, every direction's rate must be <= its original
    // rate. The leftover distribution may push frac-0 directions but
    // never above their original (since orig >= 1 means floor >= 0 and
    // leftover-add gives at most +1 ≤ orig when orig was >= 1; orig =
    // 0 has frac = 0 which sorts last and never gets +1 within the
    // first `leftover` slots when other directions have higher frac).
    let cases: &[([u32; D], u32)] = &[
        ([10, 0, 0, 0, 0, 0], 5),
        ([5, 1, 1, 1, 1, 1], 8),
        ([5, 5, 0, 0, 0, 0], 7),
        ([100, 50, 25, 12, 6, 3], 50),
    ];
    for &(initial, cap) in cases {
        let mut rates = initial;
        pc(&mut rates, cap);
        for i in 0..D {
            assert!(
                rates[i] <= initial[i],
                "non-exceedance violated at dir {i} for initial={initial:?} cap={cap}: \
                 clamped={}, orig={}",
                rates[i],
                initial[i],
            );
        }
    }
}

#[test]
fn clamp_determinism_repeated_calls_produce_same_output() {
    // Same `(rates, cap, world_seed, rng_tick, coord)` → bit-equal
    // output. Pure function.
    let initial = [5, 1, 1, 1, 1, 1];
    let cap = 8;
    let mut a = initial;
    let mut b = initial;
    let mut c = initial;
    proportional_clamp(&mut a, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
    proportional_clamp(&mut b, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
    proportional_clamp(&mut c, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn clamp_per_direction_balance_is_uniform_under_symmetric_input() {
    // Statistical isotropy: under perfectly symmetric input with a
    // forced leftover (rates = [1; 6], cap = 5 → leftover = 5, every
    // direction ties), each direction loses the tie-break with
    // probability 1/6. Per-bin zero count is `Binomial(n, 1/6)` with
    // mean `n/6` and stdev `sqrt(5n/36)`; ±5σ is loose enough to never
    // flake under fair distribution and tight enough to catch any
    // structural index bias. `n` bounded at 6_000 to keep all f64
    // casts lossless.
    let n: u32 = 6_000;
    let mut zero_counts: [u32; D] = [0; D];
    for s in 0..n {
        let s_i32 = i32::try_from(s).expect("loop index fits in i32");
        let coord = Coord::new(s_i32, s_i32 >> 3, s_i32 >> 5);
        let mut rates = [1u32; D];
        proportional_clamp(&mut rates, 5, u64::from(s), u64::from(s), coord);
        let zeros: usize = rates.iter().filter(|&&v| v == 0).count();
        assert_eq!(
            zeros, 1,
            "expected exactly one zero per call, got {zeros} in {rates:?}"
        );
        for (i, &v) in rates.iter().enumerate() {
            if v == 0 {
                zero_counts[i] += 1;
            }
        }
    }
    let mean = f64::from(n) / 6.0;
    let std_dev = (f64::from(n) * 5.0 / 36.0).sqrt();
    let tolerance = 5.0 * std_dev;
    for (i, &count) in zero_counts.iter().enumerate() {
        let dev = (f64::from(count) - mean).abs();
        assert!(
            dev <= tolerance,
            "direction {i} zero-count {count} deviates from mean {mean} by {dev} > 5σ ({tolerance:.1}); \
             full counts {zero_counts:?}",
        );
    }
}

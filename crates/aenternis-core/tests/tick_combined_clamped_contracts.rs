//! Property-level contracts for [`tick::combined_clamped`].
//!
//! These tests pin down the *contract* of `combined_clamped`:
//! conservation, per-direction non-exceedance, determinism, fast-path
//! identity, and **statistical isotropy** under direction permutation.
//! In contrast to the point-by-point assertions in
//! `tick_combined_clamped.rs` that pin specific outputs for
//! hand-constructed inputs.
//!
//! ## Why statistical isotropy, not strict per-call isotropy
//!
//! A naive contract would be: for any permutation π,
//! `combined_clamped(π(r), π(a), cap, ws, t, c) ==
//! π(combined_clamped(r, a, cap, ws, t, c))` (per-call equivariance
//! under direction relabeling). That contract is **mathematically
//! incompatible** with the others (exact conservation, integer
//! outputs, per-direction non-exceedance). When the leftover after
//! floor is a positive integer smaller than the size of the largest
//! fractional-tie group, the algorithm must pick a *strict subset* of
//! tied indices to receive `+1`. Any deterministic subset-selector
//! `select(tie_group, k, rng_state)` either depends on the indices
//! themselves (positional tie-break, sorted tie-break, shuffled
//! tie-break) — under `π` the membership of the tied group changes by
//! `π_inv`, while the selector's RNG-derived ordering does *not*
//! relabel correspondingly, so equivariance fails on at least one π —
//! or it depends only on `|tie_group|` and `k`, but then the output
//! cannot identify *which* indices to pick at all.
//!
//! What the algorithm *does* achieve is **statistical isotropy**:
//! averaged over many `(world_seed, tick, coord)` triples, every
//! direction wins the tie-break with equal probability. That's the
//! operationally relevant form — it's what prevents any direction
//! from accumulating systematic emission bias across the simulation.
//! [`per_direction_balance_is_uniform_under_symmetric_input`] is the
//! formal test of this contract; under uniform tie-break each
//! direction's "loses the tie-break" count is `Binomial(n, 1/6)`
//! with mean `n/6`, and the test fails any structural index-bias.

use aenternis_core::tick::combined_clamped;
use aenternis_core::Coord;

const D: usize = 6;

// ---- conservation ----------------------------------------------------------

/// `sum(clamped) == min(total, cap)` — exact, every time the clamp path
/// runs. Fast path trivially preserves the same identity (`sum == total
/// <= cap`).
#[test]
fn conservation_sum_equals_min_total_cap() {
    let cases: &[([u32; D], [u32; D], u32)] = &[
        ([5, 1, 1, 1, 1, 1], [0; D], 8),
        ([2, 2, 1, 0, 0, 0], [0; D], 4),
        ([5, 5, 0, 0, 0, 0], [0; D], 7),
        ([1, 1, 1, 1, 1, 1], [0; D], 5),
        ([10, 0, 0, 0, 0, 0], [0; D], 5),
        ([3, 3, 3, 3, 3, 3], [0; D], 10),
        ([100, 50, 25, 12, 6, 3], [0; D], 50),
        ([7, 0, 5, 0, 3, 0], [0; D], 8),
        ([1, 2, 3, 4, 5, 6], [6, 5, 4, 3, 2, 1], 10),
        ([0; D], [0; D], 100),             // total == 0
        ([1, 2, 3, 0, 0, 0], [0; D], 100), // total < cap
    ];
    for &(rates, active, cap) in cases {
        let total: u64 = rates
            .iter()
            .zip(active.iter())
            .map(|(r, a)| u64::from(*r) + u64::from(*a))
            .sum();
        let expected = total.min(u64::from(cap));
        let result = combined_clamped(&rates, &active, cap, 0, 0, Coord::ORIGIN);
        let sum: u64 = result.iter().map(|v| u64::from(*v)).sum();
        assert_eq!(
            sum, expected,
            "conservation violated for rates={rates:?} active={active:?} cap={cap}: \
             sum={sum}, expected min(total={total}, cap={cap})={expected}",
        );
    }
}

// ---- per-direction non-exceedance ------------------------------------------

/// `clamped[i] <= combined[i] = rates[i] + active_outflow[i]` for every
/// direction. The algorithm must never invent flow into a direction
/// beyond what that direction's combined input was.
#[test]
fn non_exceedance_per_direction() {
    let cases: &[([u32; D], [u32; D], u32)] = &[
        ([5, 1, 1, 1, 1, 1], [0; D], 8),
        ([2, 2, 1, 0, 0, 0], [0; D], 4),
        ([5, 5, 0, 0, 0, 0], [0; D], 7),
        ([1, 1, 1, 1, 1, 1], [0; D], 5),
        ([0, 0, 0, 0, 0, 1], [0; D], 100), // single-direction, fast path
        ([7, 0, 5, 0, 3, 0], [0; D], 8),
        ([1, 2, 3, 4, 5, 6], [6, 5, 4, 3, 2, 1], 10),
    ];
    for &(rates, active, cap) in cases {
        let combined: [u64; D] =
            std::array::from_fn(|i| u64::from(rates[i]) + u64::from(active[i]));
        let result = combined_clamped(&rates, &active, cap, 0, 0, Coord::ORIGIN);
        for i in 0..D {
            assert!(
                u64::from(result[i]) <= combined[i],
                "non-exceedance violated at dir {i} for rates={rates:?} active={active:?} cap={cap}: \
                 clamped={}, combined={}",
                result[i],
                combined[i],
            );
        }
    }
}

// ---- determinism -----------------------------------------------------------

/// Same arguments → bit-equal output, every time. Pure function.
#[test]
fn determinism_repeated_calls_are_bit_equal() {
    let cases: &[([u32; D], [u32; D], u32)] = &[
        ([5, 1, 1, 1, 1, 1], [0; D], 8),
        ([2, 2, 1, 0, 0, 0], [0; D], 4),
        ([5, 5, 0, 0, 0, 0], [0; D], 7),
        ([1, 1, 1, 1, 1, 1], [0; D], 5),
        ([100, 50, 25, 12, 6, 3], [0; D], 50),
    ];
    for &(rates, active, cap) in cases {
        let a = combined_clamped(&rates, &active, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
        let b = combined_clamped(&rates, &active, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
        let c = combined_clamped(&rates, &active, cap, 0xCAFE_F00D, 7, Coord::new(3, -2, 5));
        assert_eq!(
            a, b,
            "non-determinism: rates={rates:?} active={active:?} cap={cap}"
        );
        assert_eq!(
            b, c,
            "non-determinism: rates={rates:?} active={active:?} cap={cap}"
        );
    }
}

// ---- fast path identity ----------------------------------------------------

/// `total <= cap` → `clamped[i] == combined[i]` for every direction.
/// No clamp, no leftover redistribution.
#[test]
fn fast_path_returns_combined_identity_when_total_below_or_equal_cap() {
    let cases: &[([u32; D], [u32; D], u32)] = &[
        ([1, 2, 3, 0, 0, 0], [0; D], 100),             // total < cap
        ([2, 2, 2, 0, 0, 0], [0; D], 6),               // total == cap
        ([0; D], [0; D], 0),                           // both zero
        ([1, 0, 0, 0, 0, 0], [2, 0, 0, 0, 0, 0], 100), // both arrays contribute
        ([0; D], [0; D], 100),                         // total == 0
    ];
    for &(rates, active, cap) in cases {
        let combined: [u32; D] = std::array::from_fn(|i| rates[i] + active[i]);
        let result = combined_clamped(&rates, &active, cap, 0, 0, Coord::ORIGIN);
        assert_eq!(
            result, combined,
            "fast path identity violated for rates={rates:?} active={active:?} cap={cap}",
        );
    }
}

// ---- per-direction emission balance ----------------------------------------

/// Statistical doublet of the isotropy test: under perfectly symmetric
/// input (`rates[i] == R` for every direction, `cap` chosen so leftover
/// is a fixed positive integer per call), the leftover-distribution
/// algorithm should produce a per-direction `+1` count that's
/// indistinguishable from uniform across `(world_seed, tick, coord)`.
///
/// Concretely: for `rates = [1; 6]` and `cap = 5`, every call produces
/// five `1`s and one `0` (leftover = 5; one direction loses every
/// shuffle round). Under uniform tie-break, each direction loses with
/// probability `1/6`. Over `N` independent samples each per-direction
/// `zero` count is `Binomial(N, 1/6)`, with mean `N/6` and standard
/// deviation `sqrt(5 N / 36)`. A `±5σ` per-bin tolerance is loose
/// enough to never flake under fair distribution and tight enough to
/// catch any structural index bias.
#[test]
fn per_direction_balance_is_uniform_under_symmetric_input() {
    // `n` is bounded at 6_000, well below any relevant cast precision
    // limit — `f64::from(u32)` is lossless and `i32::try_from` always
    // succeeds.
    let n: u32 = 6_000;
    let mut zero_counts: [u32; D] = [0; D];
    let rates = [1u32; D];
    let active = [0u32; D];
    let cap = 5u32;
    // Vary `(world_seed, tick, coord)` so each call hits an independent
    // RNG stream — the per-cell-per-tick keying is what we need to
    // exercise here. Simple coordinate sweep is fine: the seed/tick
    // hash mixes everything.
    for s in 0..n {
        let s_i32 = i32::try_from(s).expect("loop index fits in i32");
        let coord = Coord::new(s_i32, s_i32 >> 3, s_i32 >> 5);
        let result = combined_clamped(&rates, &active, cap, u64::from(s), u64::from(s), coord);
        // Sanity: under the symmetric input there's exactly one zero
        // per call.
        let zeros: usize = result.iter().filter(|&&v| v == 0).count();
        assert_eq!(
            zeros, 1,
            "expected exactly one zero per call, got {zeros} in {result:?}"
        );
        for (i, &v) in result.iter().enumerate() {
            if v == 0 {
                zero_counts[i] += 1;
            }
        }
    }
    // Expected mean per bin: n/6. Standard deviation under
    // Binomial(n, 1/6): sqrt(n * 5/36) = sqrt(n) * sqrt(5)/6.
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

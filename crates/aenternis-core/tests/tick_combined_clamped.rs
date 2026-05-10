//! Direct unit tests for [`tick::combined_clamped`].
//!
//! `combined_clamped` is exercised end-to-end by every tick test in
//! the suite, but those go through `compute_natural_rates →
//! lay_out_pointers → collect_outflow` and only check coarse
//! invariants (conservation, determinism). The tests here pin down
//! the exact post-clamp values where they're deterministic
//! (fast path, single-direction clamp, cap = 0), and check
//! per-direction property invariants where the Largest-Remainder
//! tie-break makes the *exact* identity of the +1-receiving direction
//! RNG-dependent (any specific direction-index assertion would
//! re-introduce the bias the algorithm exists to remove).
//!
//! The full property-level contract (isotropy under direction
//! permutation, conservation, non-exceedance, determinism) lives in
//! `tick_combined_clamped_contracts.rs`.

use aenternis_core::tick::combined_clamped;
use aenternis_core::Coord;

const ZERO: [u32; 6] = [0; 6];

// All cases below use a fixed `(world_seed, rng_tick, coord)` triple so
// that the function is purely a function of `(rates, active, cap)`
// here. The tie-break inside the leftover distribution still uses RNG,
// just seeded reproducibly.
fn cc(rates: &[u32; 6], active: &[u32; 6], cap: u32) -> [u32; 6] {
    combined_clamped(rates, active, cap, 0, 0, Coord::ORIGIN)
}

// ----- no-clamp path ---------------------------------------------------------

#[test]
fn no_clamp_when_total_below_cap_returns_combined_unchanged() {
    let rates = [1, 2, 3, 0, 0, 0];
    let active = [0, 0, 0, 0, 0, 0];
    assert_eq!(cc(&rates, &active, 100), [1, 2, 3, 0, 0, 0]);
}

#[test]
fn no_clamp_when_total_equals_cap_returns_combined_unchanged() {
    // Boundary: total == cap, no clamp needed.
    let rates = [2, 2, 2, 0, 0, 0];
    let active = [0, 0, 0, 0, 0, 0];
    assert_eq!(cc(&rates, &active, 6), [2, 2, 2, 0, 0, 0]);
}

#[test]
fn no_clamp_combines_rates_and_active_outflow() {
    // Both arrays contribute to combined; cap is high enough that no clamp runs.
    let rates = [1, 0, 0, 0, 0, 0];
    let active = [2, 0, 0, 0, 0, 0];
    assert_eq!(cc(&rates, &active, 100), [3, 0, 0, 0, 0, 0]);
}

// ----- clamp path: leftover distribution -------------------------------------

#[test]
fn clamp_distributes_leftover_across_directions_with_remainder() {
    // total = 10, cap = 8, scale = 0.8.
    // combined * scale: [4.0, 0.8, 0.8, 0.8, 0.8, 0.8] → floor [4, 0, 0, 0, 0, 0].
    // new_total = 4, leftover = 4. Five tail directions tie at frac 0.8;
    // the Largest-Remainder algorithm gives +1 to four of them. The exact
    // identity of the loser depends on the per-cell RNG shuffle (and
    // *must* — that's what the algorithm exists to do), so we only
    // assert the structural shape: head stays 4, four tails become 1,
    // exactly one tail stays 0, and the post-clamp sum matches cap.
    let rates = [5, 1, 1, 1, 1, 1];
    let result = cc(&rates, &ZERO, 8);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 8);
    assert_eq!(result[0], 4, "head direction should keep its floor of 4");
    let tail_ones = result[1..].iter().filter(|&&v| v == 1).count();
    let tail_zeros = result[1..].iter().filter(|&&v| v == 0).count();
    assert_eq!(tail_ones, 4, "four of the five tied tails get +1");
    assert_eq!(tail_zeros, 1, "exactly one tied tail loses the tie-break");
}

#[test]
fn clamp_distributes_leftover_isotropically_across_ties() {
    // total = 5, cap = 4, scale = 0.8.
    // combined * scale: [1.6, 1.6, 0.8, 0, 0, 0] → floor [1, 1, 0, 0, 0, 0].
    // fracs: [0.6, 0.6, 0.8, 0, 0, 0]. leftover = 2.
    // Largest-Remainder picks idx 2 first (frac 0.8), then one of
    // {idx 0, idx 1} for the second +1 — shuffle decides which.
    // Either way: idx 2 ends at 1, exactly one of {0, 1} ends at 2,
    // the other stays at 1, and dirs 3..6 stay at 0.
    let rates = [2, 2, 1, 0, 0, 0];
    let result = cc(&rates, &ZERO, 4);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 4);
    assert_eq!(result[2], 1, "idx with highest frac gets +1");
    assert_eq!(result[3..], [0, 0, 0]);
    let head_pair = (result[0], result[1]);
    assert!(
        head_pair == (2, 1) || head_pair == (1, 2),
        "expected one of (2,1) or (1,2), got {head_pair:?}",
    );
}

#[test]
fn clamp_distributes_leftover_to_combined_positive_directions_even_when_all_floor_to_zero() {
    // Every direction has rate 1 → scale = 5/6 ≈ 0.833.
    // 1 * 0.833 = 0.833 → floor 0 in every slot, frac 0.833 in every slot.
    // leftover = 5. All six tied → five of them get +1 via shuffle.
    // Post-clamp sum reaches cap exactly (5), with one direction left at 0.
    // (Pre-fix behaviour: forfeited the leftover and returned all zeros —
    // a conservation violation under symmetric input.)
    let rates = [1, 1, 1, 1, 1, 1];
    let result = cc(&rates, &ZERO, 5);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 5);
    let ones = result.iter().filter(|&&v| v == 1).count();
    let zeros = result.iter().filter(|&&v| v == 0).count();
    assert_eq!(ones, 5);
    assert_eq!(zeros, 1);
    // Per-direction non-exceedance: clamped[i] <= combined[i] = 1 always.
    for &v in &result {
        assert!(v <= 1, "non-exceedance violated: {v} > 1");
    }
}

#[test]
fn clamp_with_cap_zero_returns_all_zero() {
    // No matter the rates, cap = 0 forces every direction to 0.
    let rates = [10, 20, 30, 40, 50, 60];
    let active = [1, 2, 3, 4, 5, 6];
    assert_eq!(cc(&rates, &active, 0), ZERO);
}

#[test]
fn clamp_keeps_combined_zero_directions_at_zero() {
    // Two non-zero directions, four zero. Floor rounding leaves leftover
    // that must only go to directions whose combined input was non-zero
    // (non-exceedance: clamped[i] <= combined[i]).
    // total = 10, cap = 7, scale = 0.7.
    // [5, 5, 0, 0, 0, 0] → [3.5, 3.5, 0, 0, 0, 0] → floor [3, 3, 0, 0, 0, 0].
    // fracs [0.5, 0.5, 0, 0, 0, 0]. leftover = 1 → goes to one of {0, 1}
    // (shuffle picks). Dirs 2..5 stay at 0 because their combined is 0.
    let rates = [5, 5, 0, 0, 0, 0];
    let result = cc(&rates, &ZERO, 7);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 7);
    assert_eq!(result[2..], [0, 0, 0, 0]);
    let head_pair = (result[0], result[1]);
    assert!(
        head_pair == (4, 3) || head_pair == (3, 4),
        "expected one of (4,3) or (3,4), got {head_pair:?}",
    );
}

#[test]
fn clamp_active_outflow_dominates_rates_when_combined_exceeds_cap() {
    // rates contribute 5, active_outflow contributes 3, cap = 5 → must clamp
    // back to 5 in a single direction. No tie-break needed.
    let rates = [5, 0, 0, 0, 0, 0];
    let active = [3, 0, 0, 0, 0, 0];
    let result = cc(&rates, &active, 5);
    assert_eq!(result, [5, 0, 0, 0, 0, 0]);
}

#[test]
fn clamp_post_sum_never_exceeds_cap() {
    // Property: across a handful of asymmetric inputs, post-clamp sum
    // never exceeds cap. (Catches `+= 1` → `-= 1` style mutations that
    // would over- or undershoot the cap.)
    let cases: &[([u32; 6], u32)] = &[
        ([10, 0, 0, 0, 0, 0], 5),
        ([3, 3, 3, 3, 3, 3], 10),
        ([100, 50, 25, 12, 6, 3], 50),
        ([7, 0, 5, 0, 3, 0], 8),
    ];
    for &(rates, cap) in cases {
        let result = cc(&rates, &ZERO, cap);
        let sum: u32 = result.iter().sum();
        assert!(
            sum <= cap,
            "post-clamp sum {sum} > cap {cap} for rates={rates:?}",
        );
    }
}

#[test]
fn clamp_post_sum_hits_cap_exactly_when_at_least_one_direction_survives_floor() {
    // When floor doesn't zero every direction, leftover goes back into a
    // surviving slot until the budget is exact.
    let rates = [10, 0, 0, 0, 0, 0];
    let result = cc(&rates, &ZERO, 7);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 7);
}

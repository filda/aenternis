//! Direct unit tests for [`tick::combined_clamped`].
//!
//! `combined_clamped` is exercised end-to-end by every tick test in
//! the suite, but those go through `compute_natural_rates →
//! lay_out_pointers → collect_outflow` and only check coarse
//! invariants (conservation, determinism). The tests here pin down
//! the exact post-clamp values for hand-constructed inputs so that
//! comparison-operator and increment-operator mutations in the
//! leftover-redistribution loop have observable consequences.

use aenternis_core::tick::combined_clamped;

const ZERO: [u32; 6] = [0; 6];

// ----- no-clamp path ---------------------------------------------------------

#[test]
fn no_clamp_when_total_below_cap_returns_combined_unchanged() {
    let rates = [1, 2, 3, 0, 0, 0];
    let active = [0, 0, 0, 0, 0, 0];
    assert_eq!(combined_clamped(&rates, &active, 100), [1, 2, 3, 0, 0, 0]);
}

#[test]
fn no_clamp_when_total_equals_cap_returns_combined_unchanged() {
    // Boundary: total == cap, no clamp needed.
    let rates = [2, 2, 2, 0, 0, 0];
    let active = [0, 0, 0, 0, 0, 0];
    assert_eq!(combined_clamped(&rates, &active, 6), [2, 2, 2, 0, 0, 0]);
}

#[test]
fn no_clamp_combines_rates_and_active_outflow() {
    // Both arrays contribute to combined; cap is high enough that no clamp runs.
    let rates = [1, 0, 0, 0, 0, 0];
    let active = [2, 0, 0, 0, 0, 0];
    assert_eq!(combined_clamped(&rates, &active, 100), [3, 0, 0, 0, 0, 0]);
}

// ----- clamp path: leftover distribution -------------------------------------

#[test]
fn clamp_funnels_leftover_into_first_nonzero_direction() {
    // total = 10, cap = 8, scale = 0.8.
    // combined * scale: [4.0, 0.8, 0.8, 0.8, 0.8, 0.8] → floor [4, 0, 0, 0, 0, 0].
    // new_total = 4, leftover = 4 → all four go to clamped[0] (only > 0).
    // Final sum = 8 (= cap), zeros stay zero.
    let rates = [5, 1, 1, 1, 1, 1];
    let result = combined_clamped(&rates, &ZERO, 8);
    assert_eq!(result, [8, 0, 0, 0, 0, 0]);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 8);
}

#[test]
fn clamp_iterates_outer_loop_when_multiple_nonzero_directions_share_leftover() {
    // total = 5, cap = 4, scale = 0.8.
    // combined * scale: [1.6, 1.6, 0.8, 0, 0, 0] → floor [1, 1, 0, 0, 0, 0].
    // new_total = 2, leftover = 2.
    // Outer loop iter 1: clamped[0]=1 → 2, leftover=1, break inner.
    // Outer loop iter 2: clamped[0]=2 → 3, leftover=0, break inner.
    // Final: [3, 1, 0, 0, 0, 0], sum = 4.
    let rates = [2, 2, 1, 0, 0, 0];
    let result = combined_clamped(&rates, &ZERO, 4);
    assert_eq!(result, [3, 1, 0, 0, 0, 0]);
}

#[test]
fn clamp_preserves_zero_directions_when_distributing_leftover() {
    // Every direction has rate 1 → scale = 5/6 ≈ 0.833.
    // 1 * 0.833 = 0.833 → floor 0 in every slot.
    // new_total = 0, leftover = 5. The `*r > 0` filter rejects every
    // entry (all zeros), so the inner `if !added break` exits and the
    // leftover is forfeited. Final result is all-zero — by design,
    // floor-rounding loss never invents flow into a direction the
    // proportional split assigned zero to.
    let rates = [1, 1, 1, 1, 1, 1];
    let result = combined_clamped(&rates, &ZERO, 5);
    assert_eq!(result, ZERO);
}

#[test]
fn clamp_with_cap_zero_returns_all_zero() {
    // No matter the rates, cap = 0 forces every direction to 0.
    let rates = [10, 20, 30, 40, 50, 60];
    let active = [1, 2, 3, 4, 5, 6];
    assert_eq!(combined_clamped(&rates, &active, 0), ZERO);
}

#[test]
fn clamp_distributes_leftover_only_to_originally_positive_directions() {
    // Two non-zero directions, four zero. Floor rounding leaves leftover
    // that must only go to the non-zero entries.
    // total = 10, cap = 7, scale = 0.7.
    // [5, 5, 0, 0, 0, 0] → [3.5, 3.5, 0, 0, 0, 0] → floor [3, 3, 0, 0, 0, 0].
    // new_total = 6, leftover = 1 → clamped[0] = 4. Final [4, 3, 0, 0, 0, 0].
    let rates = [5, 5, 0, 0, 0, 0];
    let result = combined_clamped(&rates, &ZERO, 7);
    assert_eq!(result, [4, 3, 0, 0, 0, 0]);
}

#[test]
fn clamp_active_outflow_dominates_rates_when_combined_exceeds_cap() {
    // rates contribute 5, active_outflow contributes 3, cap = 5 → must clamp
    // back to 5 in a single direction.
    let rates = [5, 0, 0, 0, 0, 0];
    let active = [3, 0, 0, 0, 0, 0];
    let result = combined_clamped(&rates, &active, 5);
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
        let result = combined_clamped(&rates, &ZERO, cap);
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
    let result = combined_clamped(&rates, &ZERO, 7);
    let sum: u32 = result.iter().sum();
    assert_eq!(sum, 7);
}

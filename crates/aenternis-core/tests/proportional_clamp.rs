//! Direct unit tests for [`cell::proportional_clamp`].
//!
//! `proportional_clamp` is the in-place sibling of `combined_clamped`
//! (used inside `compute_natural_rates` when a cell's natural-rate
//! sum exceeds its energy budget). End-to-end tick tests cover its
//! aggregate behavior; these unit tests pin down the leftover-loop
//! comparison and the early-out guard so mutations there have
//! observable effects.

use aenternis_core::cell::proportional_clamp;

const ZERO: [u32; 6] = [0; 6];

// ----- early-out paths -------------------------------------------------------

#[test]
fn no_op_when_total_below_cap() {
    let mut rates = [1, 2, 3, 0, 0, 0];
    proportional_clamp(&mut rates, 100);
    assert_eq!(rates, [1, 2, 3, 0, 0, 0]);
}

#[test]
fn no_op_when_total_equals_cap() {
    let mut rates = [2, 2, 2, 0, 0, 0];
    proportional_clamp(&mut rates, 6);
    assert_eq!(rates, [2, 2, 2, 0, 0, 0]);
}

#[test]
fn no_op_when_all_rates_zero_even_with_zero_cap() {
    // total == 0 short-circuits the function before any division happens.
    let mut rates = ZERO;
    proportional_clamp(&mut rates, 0);
    assert_eq!(rates, ZERO);
}

// ----- clamp + leftover distribution -----------------------------------------

#[test]
fn clamp_funnels_leftover_into_first_nonzero_direction() {
    // total = 10, cap = 8, scale = 0.8.
    // [5, 1, 1, 1, 1, 1] → floor [4, 0, 0, 0, 0, 0], leftover = 4.
    // All 4 go to rates[0] (the only positive entry).
    let mut rates = [5, 1, 1, 1, 1, 1];
    proportional_clamp(&mut rates, 8);
    assert_eq!(rates, [8, 0, 0, 0, 0, 0]);
}

#[test]
fn clamp_iterates_outer_loop_when_leftover_exceeds_first_increment() {
    // Multi-positive case: outer while loop must iterate until leftover hits 0.
    // total = 5, cap = 4 → floor [1, 1, 0, 0, 0, 0], leftover = 2.
    // Iter 1: rates[0] 1→2. Iter 2: rates[0] 2→3. Final: [3, 1, 0, ...].
    let mut rates = [2, 2, 1, 0, 0, 0];
    proportional_clamp(&mut rates, 4);
    assert_eq!(rates, [3, 1, 0, 0, 0, 0]);
}

#[test]
fn clamp_preserves_zero_directions_when_floor_zeroes_everything() {
    // Every direction floors to zero (1 * 5/6 = 0.833 → 0). The leftover
    // distribution loop's `*r > 0` filter must reject every entry, then
    // the `if !added break` short-circuit must exit. Otherwise the
    // leftover would leak into directions that should stay zero.
    let mut rates = [1, 1, 1, 1, 1, 1];
    proportional_clamp(&mut rates, 5);
    assert_eq!(rates, ZERO);
}

#[test]
fn clamp_keeps_zero_directions_zero_when_some_survive_floor() {
    // total = 10, cap = 7, scale = 0.7 → [3, 3, 0, 0, 0, 0], leftover = 1
    // → rates[0] 3→4. Zero directions stay zero.
    let mut rates = [5, 5, 0, 0, 0, 0];
    proportional_clamp(&mut rates, 7);
    assert_eq!(rates, [4, 3, 0, 0, 0, 0]);
}

#[test]
fn clamp_post_sum_equals_cap_when_at_least_one_direction_survives_floor() {
    // When floor doesn't kill every direction, the leftover loop drives
    // the post-clamp sum exactly to `cap`.
    let mut rates = [10, 0, 0, 0, 0, 0];
    proportional_clamp(&mut rates, 7);
    let sum: u32 = rates.iter().sum();
    assert_eq!(sum, 7);
}

#[test]
fn clamp_post_sum_never_exceeds_cap() {
    // Catch any mutation that would over-distribute leftover (e.g.
    // `if !added break` dropped, or `&&` flipped to `||`).
    let cases: &[([u32; 6], u32)] = &[
        ([10, 0, 0, 0, 0, 0], 5),
        ([3, 3, 3, 3, 3, 3], 10),
        ([100, 50, 25, 12, 6, 3], 50),
        ([7, 0, 5, 0, 3, 0], 8),
    ];
    for &(initial, cap) in cases {
        let mut rates = initial;
        proportional_clamp(&mut rates, cap);
        let sum: u32 = rates.iter().sum();
        assert!(
            sum <= cap,
            "post-clamp sum {sum} > cap {cap} for initial={initial:?}",
        );
    }
}

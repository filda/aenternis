//! Integration tests for the cell module.
//!
//! Two layers of guarantees:
//!
//! 1. **State invariants** — `energy()` always equals `memory.len()`,
//!    `end_of_tick` resets exactly the transient fields and nothing else,
//!    `append_slots` and `shrink_from_end` saturate at sensible boundaries.
//! 2. **Layout semantics** — `lay_out_pointers` walks the canonical end-order,
//!    skips overridden directions without consuming their budget, and
//!    `proportional_clamp` produces a deterministic post-clamp distribution
//!    that sums to exactly `min(original_sum, cap)`.

use aenternis_core::cell::{proportional_clamp, LAYOUT_ORDER};
use aenternis_core::{Cell, Coord, Direction};

#[test]
fn new_returns_empty_cell() {
    let c = Cell::new();
    assert_eq!(c.energy(), 0);
    assert!(c.is_empty());
    assert!(c.memory().is_empty());
    assert_eq!(c.pointers, [0; 6]);
    assert_eq!(c.rates, [0; 6]);
    assert_eq!(c.active_outflow, [0; 6]);
    assert_eq!(c.pointer_override, [false; 6]);
    assert_eq!(c.inflow, [0; 6]);
    assert_eq!(c.pc, 0);
    assert_eq!(c.origin_tag, 0);
    assert_eq!(c.appearance, 0);
}

#[test]
fn default_matches_new() {
    assert_eq!(Cell::default(), Cell::new());
}

#[test]
fn with_memory_sets_only_memory() {
    let c = Cell::with_memory(vec![1, 2, 3, 4, 5]);
    assert_eq!(c.memory(), &[1, 2, 3, 4, 5][..]);
    assert_eq!(c.energy(), 5);
    assert!(!c.is_empty());
    // Everything else stays at default.
    assert_eq!(c.pointers, [0; 6]);
    assert_eq!(c.rates, [0; 6]);
    assert_eq!(c.pc, 0);
}

#[test]
fn energy_tracks_memory_length() {
    let c = Cell::with_memory(vec![0; 42]);
    assert_eq!(c.energy(), 42);
}

#[test]
fn is_empty_only_for_zero_memory() {
    assert!(Cell::new().is_empty());
    assert!(!Cell::with_memory(vec![0]).is_empty());
}

#[test]
fn total_rate_sums_directions() {
    let mut c = Cell::new();
    c.rates = [1, 2, 3, 4, 5, 6];
    assert_eq!(c.total_rate(), 21);
}

#[test]
fn total_rate_is_zero_for_default() {
    assert_eq!(Cell::new().total_rate(), 0);
}

#[test]
fn total_active_outflow_sums_directions() {
    let mut c = Cell::new();
    c.active_outflow = [10, 20, 30, 0, 0, 5];
    assert_eq!(c.total_active_outflow(), 65);
}

#[test]
fn total_active_outflow_is_zero_for_default() {
    assert_eq!(Cell::new().total_active_outflow(), 0);
}

// ----- lay_out_pointers -----

#[test]
fn lay_out_balanced_rates_partitions_memory() {
    // rates [1, 2, 3, 4, 5, 6] sum to 21; memory size 21.
    // Walk order from end: zn(6), zp(5), yn(4), yp(3), xn(2), xp(1).
    // cursor: 21 → 15 (zn) → 10 (zp) → 6 (yn) → 3 (yp) → 1 (xn) → 0 (xp).
    let mut c = Cell::with_memory(vec![0; 21]);
    let consumption = [1u32, 2, 3, 4, 5, 6];
    c.lay_out_pointers(&consumption);
    assert_eq!(c.pointers[Direction::Xp.index()], 0);
    assert_eq!(c.pointers[Direction::Xn.index()], 1);
    assert_eq!(c.pointers[Direction::Yp.index()], 3);
    assert_eq!(c.pointers[Direction::Yn.index()], 6);
    assert_eq!(c.pointers[Direction::Zp.index()], 10);
    assert_eq!(c.pointers[Direction::Zn.index()], 15);
}

#[test]
fn lay_out_zero_rates_puts_all_pointers_at_end() {
    let mut c = Cell::with_memory(vec![0; 16]);
    c.lay_out_pointers(&[0; 6]);
    for &d in &Direction::ALL {
        assert_eq!(c.pointers[d.index()], 16);
    }
}

#[test]
fn lay_out_empty_memory_puts_all_pointers_at_zero() {
    let mut c = Cell::new();
    c.lay_out_pointers(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(c.pointers, [0; 6]);
}

#[test]
fn lay_out_skips_overridden_directions() {
    // memory size 21, rates [1, 2, 3, 4, 5, 6], but Yp (index 2) is
    // overridden to point at slot 18. The cursor must skip Yp's
    // consumption budget — Yn / Xn / Xp therefore land further up.
    let mut c = Cell::with_memory(vec![0; 21]);
    c.pointer_override[Direction::Yp.index()] = true;
    c.pointers[Direction::Yp.index()] = 18;

    c.lay_out_pointers(&[1, 2, 3, 4, 5, 6]);

    // Walk: cursor 21 → 15 (zn) → 10 (zp) → 6 (yn). Skip Yp (override).
    //         → 4 (xn, was 6 - 2) → 3 (xp, was 4 - 1).
    assert_eq!(c.pointers[Direction::Zn.index()], 15);
    assert_eq!(c.pointers[Direction::Zp.index()], 10);
    assert_eq!(c.pointers[Direction::Yn.index()], 6);
    assert_eq!(c.pointers[Direction::Yp.index()], 18); // override preserved
    assert_eq!(c.pointers[Direction::Xn.index()], 4);
    assert_eq!(c.pointers[Direction::Xp.index()], 3);
}

#[test]
fn lay_out_with_all_directions_overridden_is_a_noop() {
    let mut c = Cell::with_memory(vec![0; 21]);
    c.pointer_override = [true; 6];
    let original = [9u32, 9, 9, 9, 9, 9];
    c.pointers = original;
    c.lay_out_pointers(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(c.pointers, original);
}

#[test]
fn lay_out_saturates_at_zero_when_consumption_exceeds_energy() {
    // memory size 5, rates summing to 21 → cursor goes negative, must
    // saturate at zero rather than panic / underflow.
    let mut c = Cell::with_memory(vec![0; 5]);
    c.lay_out_pointers(&[1, 2, 3, 4, 5, 6]);
    // After saturation every pointer is 0.
    for &d in &Direction::ALL {
        assert_eq!(c.pointers[d.index()], 0);
    }
}

#[test]
fn layout_order_constant_matches_reverse_canonical() {
    // LAYOUT_ORDER must walk from highest-address direction (zn) to
    // lowest (xp) — this is the load-bearing invariant for layout math.
    assert_eq!(
        LAYOUT_ORDER,
        [
            Direction::Zn,
            Direction::Zp,
            Direction::Yn,
            Direction::Yp,
            Direction::Xn,
            Direction::Xp,
        ]
    );
}

// ----- end_of_tick -----

#[test]
fn end_of_tick_resets_overrides_and_active_outflow() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    c.pointer_override = [true; 6];
    c.active_outflow = [5, 10, 15, 20, 25, 30];

    c.end_of_tick();

    assert_eq!(c.pointer_override, [false; 6]);
    assert_eq!(c.active_outflow, [0; 6]);
}

#[test]
fn end_of_tick_does_not_touch_persistent_state() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    c.pointers = [9; 6];
    c.rates = [4; 6];
    c.pc = 7;
    c.origin_tag = 0xCAFE;
    c.appearance = 0xBABE;
    c.pointer_override = [true; 6];
    c.active_outflow = [1; 6];

    c.end_of_tick();

    assert_eq!(c.memory(), &[1, 2, 3][..]);
    assert_eq!(c.pointers, [9; 6]);
    assert_eq!(c.rates, [4; 6]);
    assert_eq!(c.pc, 7);
    assert_eq!(c.origin_tag, 0xCAFE);
    assert_eq!(c.appearance, 0xBABE);
}

// ----- shrink_from_end -----

#[test]
fn shrink_from_end_drops_count_slots() {
    let mut c = Cell::with_memory(vec![1, 2, 3, 4, 5]);
    c.shrink_from_end(2);
    assert_eq!(c.memory(), &[1, 2, 3][..]);
    assert_eq!(c.energy(), 3);
}

#[test]
fn shrink_from_end_zero_is_noop() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    c.shrink_from_end(0);
    assert_eq!(c.memory(), &[1, 2, 3][..]);
}

#[test]
fn shrink_from_end_saturates_at_full_length() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    c.shrink_from_end(99);
    assert!(c.is_empty());
    assert_eq!(c.energy(), 0);
}

#[test]
fn shrink_from_end_saturates_on_empty() {
    let mut c = Cell::new();
    c.shrink_from_end(5);
    assert!(c.is_empty());
}

// ----- append_slots -----

#[test]
fn append_slots_no_cap_takes_all() {
    let mut c = Cell::with_memory(vec![1, 2]);
    let taken = c.append_slots(&[3, 4, 5], None);
    assert_eq!(taken, 3);
    assert_eq!(c.memory(), &[1, 2, 3, 4, 5][..]);
}

#[test]
fn append_slots_under_cap_takes_all() {
    let mut c = Cell::with_memory(vec![1]);
    let taken = c.append_slots(&[2, 3], Some(10));
    assert_eq!(taken, 2);
    assert_eq!(c.memory(), &[1, 2, 3][..]);
}

#[test]
fn append_slots_truncates_at_cap() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    let taken = c.append_slots(&[4, 5, 6, 7], Some(5));
    assert_eq!(taken, 2);
    assert_eq!(c.memory(), &[1, 2, 3, 4, 5][..]);
}

#[test]
fn append_slots_with_cap_at_or_below_current_takes_nothing() {
    let mut c = Cell::with_memory(vec![1, 2, 3]);
    let taken = c.append_slots(&[4, 5], Some(3));
    assert_eq!(taken, 0);
    assert_eq!(c.memory(), &[1, 2, 3][..]);

    let taken = c.append_slots(&[4, 5], Some(2));
    assert_eq!(taken, 0);
    assert_eq!(c.memory(), &[1, 2, 3][..]);
}

#[test]
fn append_slots_empty_input_is_noop() {
    let mut c = Cell::with_memory(vec![1, 2]);
    let taken = c.append_slots(&[], None);
    assert_eq!(taken, 0);
    assert_eq!(c.memory(), &[1, 2][..]);
}

// ----- proportional_clamp -----

// Helper: drop the `(world_seed, rng_tick, coord)` boilerplate for the
// cases below where we only care about `(rates, cap)` outputs. Bias-
// independent assertions (sum, non-exceedance) hold for any RNG seed;
// tie-break-dependent specific outputs are pinned via the fixed
// `(0, 0, ORIGIN)` triple, same as in `tests/proportional_clamp.rs`.
fn pc(rates: &mut [u32; 6], cap: u32) {
    proportional_clamp(rates, cap, 0, 0, Coord::ORIGIN);
}

#[test]
fn proportional_clamp_under_cap_is_noop() {
    let mut rates = [1u32, 2, 3, 4, 5, 6];
    pc(&mut rates, 100);
    assert_eq!(rates, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn proportional_clamp_at_cap_is_noop() {
    let mut rates = [1u32, 2, 3, 4, 5, 6];
    pc(&mut rates, 21);
    assert_eq!(rates, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn proportional_clamp_zero_total_is_noop() {
    let mut rates = [0u32; 6];
    pc(&mut rates, 10);
    assert_eq!(rates, [0; 6]);
}

#[test]
fn proportional_clamp_exact_proportions_preserved() {
    // rates [10, 20, 30, 40, 50, 60] sum to 210; cap 21.
    // Each rate scales by exactly 10x → [1, 2, 3, 4, 5, 6]. Floors are
    // exact (no fractional remainder), so leftover = 0 and the
    // tie-break path is irrelevant here.
    let mut rates = [10u32, 20, 30, 40, 50, 60];
    pc(&mut rates, 21);
    assert_eq!(rates.iter().copied().fold(0u32, u32::saturating_add), 21);
    assert_eq!(rates, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn proportional_clamp_distributes_leftover_to_match_cap() {
    // rates [1, 1, 1, 1, 1, 1] sum to 6; cap 4. Floor 4/6 ≈ 0.667 →
    // every rate floors to 0, frac = 0.667 in every slot. Leftover = 4
    // is distributed to four of the six tied indices via the per-cell
    // shuffled tie-break — post-clamp sum reaches cap exactly, two
    // indices stay at 0 (RNG decides which two).
    let mut rates = [1u32; 6];
    pc(&mut rates, 4);
    let total: u32 = rates.iter().copied().fold(0u32, u32::saturating_add);
    assert_eq!(total, 4);
    let ones = rates.iter().filter(|&&v| v == 1).count();
    let zeros = rates.iter().filter(|&&v| v == 0).count();
    assert_eq!(ones, 4);
    assert_eq!(zeros, 2);
    // Per-direction non-exceedance: clamped[i] <= original[i] = 1.
    for &v in &rates {
        assert!(v <= 1);
    }
}

#[test]
fn proportional_clamp_distributes_leftover_when_some_nonzero() {
    // rates [10, 10, 0, 0, 0, 0] sum to 20; cap 7.
    // 10 * 7 / 20 = 3.5 → floor 3 each; total 6; leftover 1.
    // fracs [0.5, 0.5, 0, 0, 0, 0] — idx 0 and idx 1 tied for first
    // place; the +1 goes to one of them via shuffle. Dirs 2..5 stay 0.
    let mut rates = [10u32, 10, 0, 0, 0, 0];
    pc(&mut rates, 7);
    assert_eq!(rates.iter().copied().fold(0u32, u32::saturating_add), 7);
    assert_eq!(rates[2..], [0, 0, 0, 0]);
    let head_pair = (rates[0], rates[1]);
    assert!(
        head_pair == (4, 3) || head_pair == (3, 4),
        "expected one of (4,3) or (3,4), got {head_pair:?}",
    );
}

#[test]
fn proportional_clamp_to_zero_cap_zeroes_everything() {
    let mut rates = [3u32, 5, 7, 11, 13, 17];
    pc(&mut rates, 0);
    assert_eq!(rates, [0; 6]);
}

#[test]
fn proportional_clamp_post_total_never_exceeds_cap() {
    // Property check across a handful of arbitrary rate vectors.
    let cases = [
        ([5u32, 5, 5, 5, 5, 5], 17u32),
        ([100, 1, 1, 1, 1, 1], 50),
        ([0, 0, 0, 1, 0, 0], 1),
        ([7, 7, 7, 7, 7, 7], 13),
        ([1000, 0, 0, 0, 0, 0], 100),
    ];
    for (rates, cap) in cases {
        let mut r = rates;
        let original_total: u32 = rates.iter().copied().fold(0u32, u32::saturating_add);
        pc(&mut r, cap);
        let new_total: u32 = r.iter().copied().fold(0u32, u32::saturating_add);
        assert!(
            new_total <= cap,
            "case {rates:?} cap={cap}: post total {new_total} > cap"
        );
        assert!(
            new_total <= original_total,
            "case {rates:?}: post total grew"
        );
    }
}

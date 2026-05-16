//! Tick-level isotropy invariants — integration counterparts to the
//! per-call statistical-isotropy contract in
//! `tests/tick_combined_clamped_contracts.rs`.
//!
//! Per-call contracts hold that `apportion_with_shuffle` distributes
//! the leftover across `Direction::ALL` uniformly on average. These
//! tests check that the guarantee **composes through the full diffusion
//! pipeline** — `compute_natural_rates` → `lay_out_pointers` →
//! `collect_outflow` → `apply_outflow` → `gc_empty` — without something
//! further down the pipeline re-introducing a directional bias. Without
//! these, a future refactor could break per-tick isotropy at a level
//! that the function-level contracts can't see, and the bug would only
//! show up visually after thousands of ticks (the pattern that
//! originally motivated the isotropy work in `docs/optimalizace-2026-05.md`).
//!
//! Two angles:
//!
//! 1. **`expansion_is_isotropic_over_many_ticks`** — big bang from
//!    origin with the symmetric initial state, run N ticks, assert that
//!    the world's bbox extents along the three axes are within a small
//!    fraction of their mean. A historical `+X` lalok would show here
//!    as `x_extent` exceeding `y_extent` / `z_extent`.
//!
//! 2. **`per_direction_inflow_is_balanced_over_many_ticks`** —
//!    accumulate `cell.inflow[d]` summed across all cells after each
//!    tick. After N ticks, the six directional totals must be within a
//!    statistical tolerance of equal. Tighter signal than bbox extent
//!    (which counts only the leading edge of expansion) — the inflow
//!    accumulator counts every slot that crossed every face every tick.
//!
//! ## Why `step_diffusion`, not `step`
//!
//! These tests use [`tick::step_diffusion`] so the CPU phase doesn't
//! run. A big-bang's origin cell is seeded with RNG-derived memory
//! which the VM interprets as opcodes; for some seeds the resulting
//! program executes asymmetric `port` instructions that pump active
//! outflow into one specific direction, producing a per-seed bias
//! that has nothing to do with the diffusion isotropy this test is
//! supposed to guard. `step_diffusion` runs the diffusion pipeline
//! end-to-end without ever calling `cpu_phase`, isolating the
//! invariant we care about. The full `step` path's emission isotropy
//! under non-degenerate programs is a separate concern best caught at
//! the program / VM level.

use aenternis_core::{tick, Direction, SparseWorld};

const SEED: u64 = 17;
const ENERGY: u32 = 10_000;
const TICKS: u32 = 200;
const COEFF: f64 = 1.0;

#[test]
fn expansion_is_isotropic_over_many_ticks() {
    let mut w = SparseWorld::big_bang(SEED, ENERGY);
    for _ in 0..TICKS {
        tick::step_diffusion(&mut w, COEFF);
    }

    // `bounding_box` returns `(x_min, x_max, y_min, y_max, z_min, z_max)` —
    // axis-grouped, not min-first.
    let (x_min, x_max, y_min, y_max, z_min, z_max) =
        w.bounding_box().expect("expanded world has cells");
    let x_ext = f64::from(x_max - x_min);
    let y_ext = f64::from(y_max - y_min);
    let z_ext = f64::from(z_max - z_min);
    let mean_ext = (x_ext + y_ext + z_ext) / 3.0;
    assert!(
        mean_ext > 5.0,
        "test setup expanded too little for the variance bound to be meaningful: extents = ({x_ext}, {y_ext}, {z_ext})",
    );
    let max_dev = [
        (x_ext - mean_ext).abs(),
        (y_ext - mean_ext).abs(),
        (z_ext - mean_ext).abs(),
    ]
    .into_iter()
    .fold(0.0_f64, f64::max);

    // Random-walk dispersion of the leading edge scales as O(sqrt(N))
    // in the symmetric regime; the historical bias was O(N). For
    // N=200 the noise floor is ~14 cells of dispersion and the bias
    // signal was ~200. A 15% tolerance on mean extent leaves headroom
    // for legitimate stochastic noise while catching anything that
    // re-introduces O(N) drift.
    let tolerance = mean_ext * 0.15;
    assert!(
        max_dev <= tolerance,
        "bbox extents diverge — x = {x_ext}, y = {y_ext}, z = {z_ext}, mean = {mean_ext:.2}, max_dev = {max_dev:.2}, tolerance = {tolerance:.2}",
    );
}

#[test]
fn per_direction_inflow_is_balanced_over_many_ticks() {
    let mut w = SparseWorld::big_bang(SEED, ENERGY);

    let mut totals: [u64; Direction::COUNT] = [0; Direction::COUNT];
    for _ in 0..TICKS {
        tick::step_diffusion(&mut w, COEFF);
        // `cell.inflow[d]` accumulates slot count entering face `d`
        // during the just-completed apply_outflow. Sum across all live
        // cells to get the world's per-direction inflow for this tick;
        // add into the cumulative.
        for (_, cell) in w.iter() {
            for (bin, &count) in totals.iter_mut().zip(cell.inflow.iter()) {
                *bin = bin.saturating_add(u64::from(count));
            }
        }
    }

    let sum: u64 = totals.iter().sum();
    assert!(
        sum > 1_000,
        "test setup produced too little inflow ({sum}) for the variance bound to be meaningful",
    );
    // Per-bin counts stay under `2^32` for the test parameters used
    // here (200 ticks × 10k energy / 6 directions ≪ 2^32), so the
    // `u64 → f64` cast is precision-safe even though clippy can't
    // prove it from the type alone.
    #[allow(clippy::cast_precision_loss)]
    let sum_f = sum as f64;
    let mean = sum_f / f64::from(u32::try_from(Direction::COUNT).expect("6 fits in u32"));
    #[allow(clippy::cast_precision_loss)]
    let max_dev = totals
        .iter()
        .map(|&t| (t as f64 - mean).abs())
        .fold(0.0_f64, f64::max);

    // For ~uniform distribution of `sum` slots over 6 bins, the per-
    // bin variance is roughly `sum / 6 * (1 - 1/6) = 5 * sum / 36`,
    // giving a standard deviation of `sqrt(5 * sum / 36)`. Allowing 8
    // sigma leaves the test essentially false-positive-free under
    // legitimate randomness while still catching any O(N) systematic
    // bias (which would be many tens of sigma at these accumulated
    // counts).
    let sigma = (5.0 * sum_f / 36.0).sqrt();
    let tolerance = 8.0 * sigma;
    assert!(
        max_dev <= tolerance,
        "per-direction inflow totals diverge — {totals:?}, sum = {sum}, mean = {mean:.1}, max_dev = {max_dev:.1}, 8σ = {tolerance:.1}",
    );
}

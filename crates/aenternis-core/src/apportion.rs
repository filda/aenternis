//! Largest-Remainder apportionment with a shuffled tie-break.
//!
//! Shared algorithmic core for [`crate::tick::combined_clamped`] and
//! [`crate::cell::proportional_clamp`]. Both functions previously held
//! an open-coded copy of the same Hamilton/Hare distribution + Fisher-
//! Yates tie-break; this module centralizes the implementation so a
//! future algorithmic fix (e.g. the deterministic-round work tracked in
//! `docs/plan-deterministicky-round.md`) touches one place, not two.
//!
//! ## Algorithm
//!
//! Given non-negative per-direction `values: [u64; 6]` and an integer
//! `cap: u32`, return `[u32; 6]` whose sum is exactly `min(sum(values),
//! cap)` and whose per-direction outputs do not exceed the corresponding
//! input. When `sum(values) <= cap` the inputs are returned as-is (the
//! "fast path"); otherwise values are scaled by `cap / sum(values)`,
//! floored to integers, and the leftover gap (`cap - sum(floors)`,
//! always in `0..=5`) is closed by giving `+1` to the indices with the
//! largest fractional remainders. Ties between equal remainders are
//! broken by a Fisher-Yates shuffle of `[0..6]` seeded from `(world_seed,
//! rng_tick, coord, domain)`, so the macro-scale leftover distribution
//! is independent of `Direction::ALL`'s canonical ordering.
//!
//! ## Why `f64` not exact integer arithmetic
//!
//! The clamp scales via `f64` (`cap / total`, then `value * scale`)
//! rather than via exact `u64` integer division. The two paths disagree
//! at boundary values (truncation vs round-to-nearest-then-floor); the
//! `f64` path is the frozen choice because it has the per-cell
//! stochastic-rounding stream baked into it. `total` reaches at most
//! ~`2^35.6` (six times `(u32::MAX + u32::MAX)` for `combined_clamped`'s
//! worst case), well under `f64`'s `2^53` exact-integer ceiling, so
//! the cast is precision-safe within its operating range.
//!
//! ## Statistical isotropy, not per-call equivariance
//!
//! Strict equivariance under direction permutation is provably
//! incompatible with exact conservation + integer outputs + per-direction
//! non-exceedance; see
//! `crates/aenternis-core/tests/tick_combined_clamped_contracts.rs`
//! for the long-form argument. What this algorithm achieves is
//! **statistical isotropy**: averaged over many `(world_seed, rng_tick,
//! coord)` triples each direction wins the tie-break with equal
//! probability, so the macro emission balance over a populated world is
//! uniform across `Direction::ALL`.

use crate::{Coord, Direction, Rng};

/// RNG domain salt for [`crate::tick::combined_clamped`]'s leftover-
/// distribution tie-break. Distinct from the default domain (`0`) used
/// by [`crate::tick::compute_natural_rates`] so the two streams cannot
/// correlate even when they share `(world_seed, rng_tick, coord)`.
pub(crate) const COMBINED_CLAMPED_RNG_DOMAIN: u32 = 1;

/// RNG domain salt for [`crate::cell::proportional_clamp`]'s leftover-
/// distribution tie-break. Distinct from the default domain (`0`) used
/// by `compute_natural_rates` for `stochastic_floor` draws and from
/// [`COMBINED_CLAMPED_RNG_DOMAIN`] (`1`), so all three streams stay
/// uncorrelated even when they share `(world_seed, rng_tick, coord)`.
pub(crate) const PROPORTIONAL_CLAMP_RNG_DOMAIN: u32 = 2;

/// Largest-Remainder apportionment with a Fisher-Yates tie-break.
///
/// Returns `[u32; 6]` whose sum equals `min(sum(values), cap)` and
/// where `result[i] <= values[i]` (per-direction non-exceedance). The
/// fast path (`sum(values) <= cap`) skips RNG and sort entirely and
/// returns `values` cast lossless to `u32` — each entry is bounded by
/// `cap <= u32::MAX` so the cast is safe.
///
/// `domain` separates RNG streams for callers that share
/// `(world_seed, rng_tick, coord)` — see the module-level docs for the
/// reserved values.
pub(crate) fn apportion_with_shuffle(
    values: &[u64; Direction::COUNT],
    cap: u32,
    world_seed: u64,
    rng_tick: u64,
    coord: Coord,
    domain: u32,
) -> [u32; Direction::COUNT] {
    let total: u64 = values.iter().sum();
    let cap64 = u64::from(cap);
    if total <= cap64 {
        // Each `values[i] <= total <= cap <= u32::MAX`, so `as u32` is
        // lossless here.
        #[allow(clippy::cast_possible_truncation)]
        return std::array::from_fn(|i| values[i] as u32);
    }
    // Clamp via `f64` — see the module docs for why this is the frozen
    // choice and not `u64` integer division. `total` reaches at most
    // ~`2^35.6` (six times `2 * u32::MAX` for the combined wrapper's
    // worst case), well under `f64`'s `2^53` exact-integer ceiling.
    // (No `From<u64> for f64` impl exists, so the cast is `as f64`
    // rather than the lossless `From`.)
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let scale = f64::from(cap) / total as f64;
    let mut clamped: [u32; Direction::COUNT] = [0; Direction::COUNT];
    let mut frac: [f64; Direction::COUNT] = [0.0; Direction::COUNT];
    let mut new_total: u32 = 0;
    for i in 0..Direction::COUNT {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let value_f = values[i] as f64;
        let scaled = value_f * scale;
        let floored = scaled.floor();
        frac[i] = scaled - floored;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let val = floored as u32;
        clamped[i] = val;
        new_total = new_total.saturating_add(val);
    }
    // Distribute leftover by Largest-Remainder with shuffled tie-break.
    // `cap >= new_total` always holds: each `floored ≤ value * scale`
    // and `sum(value * scale) = cap`, so `new_total ≤ cap`. The shuffle
    // + sort runs unconditionally even when `leftover == 0` (the rare
    // case where `f64` rounding leaves `new_total == cap` exactly): a
    // `take(0)` below makes that path a no-op without an observable-
    // equivalent `if leftover > 0` early-skip that mutation tests would
    // correctly flag as redundant.
    let leftover = cap.saturating_sub(new_total) as usize;
    let mut order: [usize; Direction::COUNT] = [0, 1, 2, 3, 4, 5];
    let mut rng = Rng::for_cell_at_tick(world_seed, rng_tick, coord, domain);
    // Fisher-Yates shuffle of `order`. Indices `i` in `(1..6).rev()`
    // pick a uniformly-distributed swap target in `0..=i` from the RNG.
    // After this loop `order` is a uniformly-random permutation of
    // `[0, 1, 2, 3, 4, 5]` deterministic in `(world_seed, rng_tick,
    // coord, domain)`.
    for i in (1..Direction::COUNT).rev() {
        // `next_u32() as usize % (i + 1)` — unbiased enough at this
        // tiny range; the modulo bias for a 32-bit draw over `2..=6`
        // is below `2^-29`, and we are already shuffling six elements.
        let j = (rng.next_u32() as usize) % (i + 1);
        order.swap(i, j);
    }
    // Stable sort `order` by `frac` descending — equal remainders keep
    // their (already-shuffled) relative order, so the tie-break is
    // independent of `Direction::ALL`'s canonical ordering.
    order.sort_by(|&a, &b| {
        frac[b]
            .partial_cmp(&frac[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for &idx in order.iter().take(leftover) {
        clamped[idx] = clamped[idx].saturating_add(1);
    }
    clamped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apportion_exact_proportions_no_leftover() {
        // values = [3; 6], cap = 6 → scale = 1/3, every value scales
        // to exactly 1.0 → floor 1, frac 0.0, new_total = 6, leftover
        // = 0. Tie-break path is a no-op (`take(0)`). Result is the
        // all-ones vector regardless of seed.
        let values = [3u64; Direction::COUNT];
        let result =
            apportion_with_shuffle(&values, 6, 0, 0, Coord::ORIGIN, COMBINED_CLAMPED_RNG_DOMAIN);
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 6);
        assert!(
            result.iter().all(|&v| v == 1),
            "expected all 1s, got {result:?}",
        );
    }
}

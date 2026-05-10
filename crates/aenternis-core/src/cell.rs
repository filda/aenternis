//! Cells: the elementary unit of the simulation world.
//!
//! Every cell carries
//!
//! - a [`memory`](Cell::memory) vector of 32-bit slots — where program code
//!   and data live, indistinguishable to the engine
//! - six [`pointers`](Cell::pointers) and six [`rates`](Cell::rates), one per
//!   direction, that drive emission across faces in the next tick
//! - an [`active_outflow`](Cell::active_outflow) buffer for `port` ignition
//!   that is reset at the end of every tick
//! - a [`pointer_override`](Cell::pointer_override) flag set per direction
//!   for one tick by `setp` / `setpv`, also reset at end of tick
//! - a [`pc`](Cell::pc) program counter
//! - two UI-only fields ([`origin_tag`](Cell::origin_tag),
//!   [`appearance`](Cell::appearance)) that the engine never consults
//!
//! The cardinal invariant is **`cell.energy() == cell.memory.len() as u32`** —
//! one unit of energy *is* one slot of memory. The two are alternative names
//! for the same quantity.

use crate::{Coord, Direction, Rng};

/// Order in which pointers are laid out from the end of memory.
///
/// `zn` gets the highest addresses, `xp` the lowest. Anything that walks the
/// pointer layout must use this order — see `docs/mechanics.md`.
pub const LAYOUT_ORDER: [Direction; Direction::COUNT] = [
    Direction::Zn,
    Direction::Zp,
    Direction::Yn,
    Direction::Yp,
    Direction::Xn,
    Direction::Xp,
];

/// A single cell of the simulation world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Cell memory. Each entry is a 32-bit slot; one slot also represents
    /// one unit of energy. `memory.len() as u32` is the cell's energy.
    pub memory: Vec<u32>,

    /// Directional pointers. `pointers[d]` is the start index in
    /// [`memory`](Self::memory) of the slot range that direction `d` will
    /// emit next tick.
    pub pointers: [u32; Direction::COUNT],

    /// Combined rates per direction (natural + active) decided in the
    /// previous tick's layout. Used as the per-direction emission budget.
    pub rates: [u32; Direction::COUNT],

    /// Active outflow accumulated from `port` instructions in this tick's
    /// CPU phase. Reset by [`end_of_tick`](Self::end_of_tick) after the
    /// outflow phase.
    pub active_outflow: [u32; Direction::COUNT],

    /// `pointer_override[d]` is `true` iff the program overrode pointer `d`
    /// via `setp` or `setpv` this tick. Reset by
    /// [`end_of_tick`](Self::end_of_tick).
    pub pointer_override: [bool; Direction::COUNT],

    /// Per-direction count of slots received in the most recently
    /// completed tick. Written by `tick::apply_outflow`, read by the
    /// `sinflow` opcode. Persists across `end_of_tick` so the next
    /// tick's CPU phase can observe it.
    pub inflow: [u32; Direction::COUNT],

    /// Program counter — index into [`memory`](Self::memory).
    pub pc: u32,

    /// UI lineage marker. Does not influence physics.
    pub origin_tag: u32,

    /// UI appearance / war paint. Does not influence physics.
    pub appearance: u32,
}

impl Cell {
    /// Build an empty cell — no memory, no program, all directional state
    /// at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            memory: Vec::new(),
            pointers: [0; Direction::COUNT],
            rates: [0; Direction::COUNT],
            active_outflow: [0; Direction::COUNT],
            pointer_override: [false; Direction::COUNT],
            inflow: [0; Direction::COUNT],
            pc: 0,
            origin_tag: 0,
            appearance: 0,
        }
    }

    /// Build a cell that already carries the given program / data slots.
    ///
    /// Pointers are not laid out; call
    /// [`lay_out_pointers`](Self::lay_out_pointers) once rates are decided.
    #[must_use]
    pub fn with_memory(memory: Vec<u32>) -> Self {
        let mut c = Self::new();
        c.memory = memory;
        c
    }

    /// Cell energy = `memory.len()`, narrowed to `u32`.
    #[must_use]
    pub fn energy(&self) -> u32 {
        self.memory.len() as u32
    }

    /// `true` iff the cell holds no energy / memory.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.memory.is_empty()
    }

    /// Sum of [`rates`](Self::rates) across all directions.
    #[must_use]
    pub fn total_rate(&self) -> u32 {
        self.rates.iter().copied().fold(0u32, u32::saturating_add)
    }

    /// Sum of [`active_outflow`](Self::active_outflow) across all directions.
    #[must_use]
    pub fn total_active_outflow(&self) -> u32 {
        self.active_outflow
            .iter()
            .copied()
            .fold(0u32, u32::saturating_add)
    }

    /// Lay out pointers from the end of memory using the given per-direction
    /// `consumption` budget.
    ///
    /// Walk order is `zn, zp, yn, yp, xn, xp` (canonical end-order); each
    /// step decrements the cursor by `consumption[d]` and assigns the
    /// post-walk position to `pointers[d]`.
    ///
    /// **Overridden directions are skipped.** Their pointer keeps the value
    /// the program set with `setp` / `setpv`, and the cursor walk does not
    /// advance through their consumption — subsequent (lower-address)
    /// pointers therefore land where they would land if the override didn't
    /// exist.
    ///
    /// Used in two contexts:
    ///
    /// - end-of-tick natural-rate layout, where overrides have been reset
    ///   to all-`false`, so the walk advances through every direction
    /// - sub-tick reflow with combined rates, where overrides are live
    pub fn lay_out_pointers(&mut self, consumption: &[u32; Direction::COUNT]) {
        let mut cursor = self.energy();
        for d in LAYOUT_ORDER {
            let i = d.index();
            if self.pointer_override[i] {
                continue;
            }
            cursor = cursor.saturating_sub(consumption[i]);
            self.pointers[i] = cursor;
        }
    }

    /// Reset transient per-tick state: pointer overrides and active outflow.
    /// Called after the outflow phase of every tick.
    pub const fn end_of_tick(&mut self) {
        self.pointer_override = [false; Direction::COUNT];
        self.active_outflow = [0; Direction::COUNT];
    }

    /// Shrink memory from the end by `count` slots. Saturates if `count`
    /// exceeds the current length.
    pub fn shrink_from_end(&mut self, count: u32) {
        let drop = (count as usize).min(self.memory.len());
        self.memory.truncate(self.memory.len() - drop);
    }

    /// Append `slots` to the end of memory, optionally capped.
    ///
    /// If `cap` is `Some(n)`, at most `n - memory.len()` slots are taken
    /// (truncating the input). Returns the number of slots actually
    /// appended.
    pub fn append_slots(&mut self, slots: &[u32], cap: Option<u32>) -> usize {
        let mut to_take = slots.len();
        if let Some(cap) = cap {
            let cap_usize = cap as usize;
            let room = cap_usize.saturating_sub(self.memory.len());
            to_take = to_take.min(room);
        }
        self.memory.extend_from_slice(&slots[..to_take]);
        to_take
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

/// RNG domain salt for [`proportional_clamp`]'s leftover-distribution
/// tie-break. Distinct from the default domain (`0`) used by
/// `compute_natural_rates` for `stochastic_floor` draws and from
/// `tick::COMBINED_CLAMPED_RNG_DOMAIN` (`1`), so all three streams stay
/// uncorrelated even when they share `(world_seed, tick, coord)`.
pub(crate) const PROPORTIONAL_CLAMP_RNG_DOMAIN: u32 = 2;

/// Scale `rates` down in place so their sum does not exceed `cap`.
///
/// Used when the combined per-direction rate exceeds the cell's current
/// memory size — proportional clamping ensures total outflow never exceeds
/// the available memory budget. Floor-rounded scaling can lose up to
/// `DIRS - 1` units to rounding; the leftover is distributed back via
/// Largest-Remainder apportionment with a per-cell-deterministic
/// Fisher-Yates tie-break, so the post-clamp sum is exactly
/// `min(original_sum, cap)` *and* the choice of which directions absorb
/// the leftover does not bias toward any specific face of the
/// `Direction::ALL` ordering.
///
/// **Statistical isotropy.** Across many `(world_seed, rng_tick,
/// coord)` triples each direction wins/loses the leftover tie-break
/// with equal probability, so the macro emission balance is uniform
/// across `Direction::ALL`. (Strict per-call equivariance under
/// direction permutation is provably incompatible with exact
/// conservation + integer outputs + per-direction non-exceedance, so it
/// is not part of the contract; the structural argument is the same as
/// for [`crate::tick::combined_clamped`].)
///
/// **Determinism:** the leftover tie-break is keyed by `(world_seed,
/// rng_tick, coord, PROPORTIONAL_CLAMP_RNG_DOMAIN)`, so the result is
/// independent of cell allocation order or other ambient state. See
/// [`PROPORTIONAL_CLAMP_RNG_DOMAIN`] for the salt's purpose.
///
/// **Total in `u64`:** the sum of six `u32` rates can exceed `u32::MAX`
/// when `port` accumulates `active_outflow` close to its saturation
/// boundary across multiple directions. Computing the sum (and the
/// scaling denominator) in `u64` is the only way to keep the
/// `rates[d] * cap / total` ratio mathematically correct in that case;
/// a `u32` saturating sum would underestimate `total` and let the post-
/// clamp sum spill over `cap`.
pub fn proportional_clamp(
    rates: &mut [u32; Direction::COUNT],
    cap: u32,
    world_seed: u64,
    rng_tick: u64,
    coord: Coord,
) {
    let total: u64 = rates.iter().copied().map(u64::from).sum();
    let cap64 = u64::from(cap);
    if total <= cap64 || total == 0 {
        return;
    }
    // Floor-rounded scale via `f64` arithmetic to bit-match JS prototype
    // 9-B's `Math.floor(rate * (cap / total))`. Pure `u64` integer
    // division would be exact but produces results that disagree with
    // JS at boundary cases — JS `cap / total` is rounded-to-nearest
    // f64, then `rate * scale` carries that rounding through, then
    // `Math.floor` may step the result down by 1 vs the integer-exact
    // value. Replicating that path keeps the dominance / intrusion
    // distribution of inflows aligned with 9-B; without it, the
    // last-mile equivalence diff bleeds 1351-1890-slot shifts between
    // adjacent directions per cell. Rates and `cap` always fit in
    // `f64` exactly (well under `2^53`), so this path is safe.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    {
        // `total` is `u64` and can reach 6 * u32::MAX ≈ 2^34.6 — well
        // under f64's 2^53 precision boundary, so `as f64` is exact for
        // any value we'll see. (No `From<u64> for f64` impl exists, so
        // we can't use the lossless cast here.) `cap` and `r` are u32,
        // so `f64::from` is safe and idiomatic.
        let scale = f64::from(cap) / total as f64;
        let mut frac: [f64; Direction::COUNT] = [0.0; Direction::COUNT];
        let mut new_total: u32 = 0;
        for (i, r) in rates.iter_mut().enumerate() {
            let scaled = f64::from(*r) * scale;
            let floored = scaled.floor();
            frac[i] = scaled - floored;
            let scaled_u32 = floored as u32;
            *r = scaled_u32;
            new_total = new_total.saturating_add(scaled_u32);
        }
        // Distribute leftover by Largest-Remainder with shuffled
        // tie-break (same algorithm as `tick::combined_clamped`; see
        // its docstring for the structural argument). The shuffle +
        // sort runs unconditionally even when `leftover == 0` — the
        // `take(0)` below is a no-op and we trade a rare extra branch
        // for a structure that's exhaustively mutation-tested.
        let leftover = cap.saturating_sub(new_total) as usize;
        let mut order: [usize; Direction::COUNT] = [0, 1, 2, 3, 4, 5];
        let mut rng =
            Rng::for_cell_at_tick(world_seed, rng_tick, coord, PROPORTIONAL_CLAMP_RNG_DOMAIN);
        for i in (1..Direction::COUNT).rev() {
            let j = (rng.next_u32() as usize) % (i + 1);
            order.swap(i, j);
        }
        order.sort_by(|&a, &b| {
            frac[b]
                .partial_cmp(&frac[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for &idx in order.iter().take(leftover) {
            rates[idx] = rates[idx].saturating_add(1);
        }
    }
}

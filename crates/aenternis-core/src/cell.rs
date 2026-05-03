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

use crate::Direction;

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
    pub fn end_of_tick(&mut self) {
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

/// Scale `rates` down in place so their sum does not exceed `cap`.
///
/// Used when the combined per-direction rate exceeds the cell's current
/// memory size — proportional clamping ensures total outflow never exceeds
/// the available memory budget. Floor-rounded scaling can lose up to
/// `DIRS - 1` units to rounding; the leftover is distributed back to
/// non-zero directions in canonical order so the post-clamp sum is exactly
/// `min(original_sum, cap)`.
///
/// **Determinism:** the leftover-distribution loop walks directions in the
/// order they appear in the array, so the result is independent of cell
/// allocation order or other ambient state.
pub fn proportional_clamp(rates: &mut [u32; Direction::COUNT], cap: u32) {
    let total: u32 = rates.iter().copied().fold(0u32, u32::saturating_add);
    if total <= cap || total == 0 {
        return;
    }
    // Floor-rounded scale: rates[d] = floor(rates[d] * cap / total).
    // Compute in u64 to avoid overflow during multiplication.
    let mut new_total: u32 = 0;
    for r in &mut *rates {
        let scaled = (u64::from(*r) * u64::from(cap)) / u64::from(total);
        *r = scaled as u32;
        new_total = new_total.saturating_add(*r);
    }
    // Distribute leftover (floor rounding may have lost up to DIRS - 1 units).
    let mut leftover = cap.saturating_sub(new_total);
    while leftover > 0 {
        let mut added = false;
        for r in &mut *rates {
            if *r > 0 && leftover > 0 {
                *r += 1;
                leftover -= 1;
                added = true;
                break;
            }
        }
        if !added {
            break;
        }
    }
}

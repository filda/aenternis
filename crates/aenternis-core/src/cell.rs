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

use crate::apportion::{apportion_with_shuffle, PROPORTIONAL_CLAMP_RNG_DOMAIN};
use crate::{Coord, Direction};

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
    ///
    /// **Access through the [`Cell::memory`] / [`Cell::memory_mut`] /
    /// [`Cell::extend_memory`] / [`Cell::replace_memory`] methods**, not
    /// directly. The field is `pub(crate)` so internal modules (vm,
    /// tick, world::sparse) can still build a `Cell` literal; outside
    /// the crate the methods are the only entry point. This indirection
    /// is the seam for migrating storage out of the `Cell` and into a
    /// world-owned arena (Phase 2 of the arena refactor) — once that
    /// lands, these methods reroute to `&world.arena[start..start+len]`
    /// without touching every call site.
    pub(crate) memory: Vec<u32>,

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

    /// Read-only view of the cell's memory slots. Length equals
    /// [`Cell::energy`].
    ///
    /// The seam for the arena refactor: today this returns a `&[u32]`
    /// over the cell-owned `Vec`; once storage moves to a world-owned
    /// arena (Phase 2) it'll return a `&[u32]` over the arena range
    /// `[mem_start .. mem_start + mem_len]`. Callers see the same type
    /// either way.
    #[must_use]
    pub fn memory(&self) -> &[u32] {
        &self.memory
    }

    /// Mutable view of the cell's memory slots. Length cannot change
    /// through this handle — use [`Cell::extend_memory`],
    /// [`Cell::shrink_from_end`], [`Cell::push_memory_slot`], or
    /// [`Cell::replace_memory`] for length changes.
    #[must_use]
    pub fn memory_mut(&mut self) -> &mut [u32] {
        &mut self.memory
    }

    /// Length of the cell's memory in slots. Same value as
    /// [`Cell::energy`] but `usize`-typed for indexing arithmetic.
    #[must_use]
    pub fn memory_len(&self) -> usize {
        self.memory.len()
    }

    /// Copy out the slot at `i`. Cheap `u32` read; avoids the borrow
    /// the slice-indexing form would hold, which matters in VM
    /// arithmetic opcodes that read several slots then write one.
    #[must_use]
    pub fn memory_slot(&self, i: usize) -> u32 {
        self.memory[i]
    }

    /// Write the slot at `i`. Pairs with [`Cell::memory_slot`] so VM
    /// opcodes can do `set_memory_slot(dst, memory_slot(src))` without
    /// fighting the borrow checker.
    pub fn set_memory_slot(&mut self, i: usize, v: u32) {
        self.memory[i] = v;
    }

    /// Append a single slot to the end of memory, growing energy by 1.
    pub fn push_memory_slot(&mut self, slot: u32) {
        self.memory.push(slot);
    }

    /// Append `slots` to the end of memory, growing energy by
    /// `slots.len()`. The bulk variant of [`Cell::push_memory_slot`];
    /// underlying `Vec::extend_from_slice` reserves capacity in one
    /// shot, which matters for the arena rewrite when a single inflow
    /// may carry hundreds of slots.
    pub fn extend_memory(&mut self, slots: &[u32]) {
        self.memory.extend_from_slice(slots);
    }

    /// Replace the entire memory buffer, returning the previous one.
    ///
    /// Used by the rope-merge in [`crate::tick::apply_outflow`] to
    /// swap a freshly-rebuilt `Vec<u32>` into place without an extra
    /// allocation. The returned `Vec` becomes the caller's scratch on
    /// the next iteration.
    ///
    /// **Note for the arena refactor (Phase 2):** this signature will
    /// change to take a `&[u32]` and write into the arena, since the
    /// arena-era cell no longer owns its `Vec`. Callers using this
    /// method today should expect to be touched then; for now the
    /// behaviour is bit-identical to `mem::swap(&mut cell.memory,
    /// &mut new)`.
    #[must_use]
    pub fn replace_memory(&mut self, mut new_memory: Vec<u32>) -> Vec<u32> {
        std::mem::swap(&mut self.memory, &mut new_memory);
        new_memory
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

/// Scale `rates` down in place so their sum does not exceed `cap`.
///
/// Used when the combined per-direction rate exceeds the cell's current
/// memory size — proportional clamping ensures total outflow never exceeds
/// the available memory budget. The algorithmic core (proportional `f64`
/// scale + Largest-Remainder leftover distribution with a Fisher-Yates
/// tie-break) lives in [`crate::apportion::apportion_with_shuffle`];
/// see that module for the JS bit-parity argument, the statistical-
/// isotropy contract, and the `f64`-precision bounds. This wrapper only
/// widens `rates` to `[u64; 6]` and writes the result back in place.
pub fn proportional_clamp(
    rates: &mut [u32; Direction::COUNT],
    cap: u32,
    world_seed: u64,
    rng_tick: u64,
    coord: Coord,
) {
    let values: [u64; Direction::COUNT] = std::array::from_fn(|i| u64::from(rates[i]));
    *rates = apportion_with_shuffle(
        &values,
        cap,
        world_seed,
        rng_tick,
        coord,
        PROPORTIONAL_CLAMP_RNG_DOMAIN,
    );
}

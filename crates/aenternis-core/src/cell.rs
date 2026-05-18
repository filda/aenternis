//! Cells: the elementary unit of the simulation world.
//!
//! Every cell carries
//!
//! - a memory range (`mem_start`, `mem_len`) pointing into the
//!   world's arena — 32-bit slots where program code and data live,
//!   indistinguishable to the engine
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
//! The cardinal invariant is **`cell.energy() == cell.mem_len`** —
//! one unit of energy *is* one slot of memory, and `mem_len` is the
//! authoritative count.
//!
//! ## Arena indirection
//!
//! Cells no longer own their memory; the storage lives in the
//! world's [`Arena`](crate::world::arena::Arena). A cell carries
//! only the `(mem_start, mem_len)` indices, so the struct is a
//! fixed ~128 B regardless of energy. Accessor methods that need
//! to read or write slots take an `&Arena` / `&mut Arena`
//! parameter — there's no way to mutate a cell's slots without
//! going through the world's arena.
//!
//! Why: the old per-cell `Vec<u32>` triggered ~250 k churning
//! mallocs per tick on a 1 M-energy world and fragmented the
//! global allocator until a 5 MB contiguous request failed at
//! tick 2200. See `docs/optimalizace-2026-05.md` for the
//! diagnosis and the multi-phase rewrite plan. Phases 1-4 of
//! that plan are in; Phase 3's double-buffer arena
//! ([`crate::SparseWorld::arena`] paired with
//! [`crate::SparseWorld::arena_next`]) is what
//! [`crate::tick::apply_outflow`] writes through, and it
//! sidesteps every per-cell mutator that used to live on `Cell`
//! (`extend_memory`, `replace_memory`, `shrink_from_end`, etc.)
//! by computing each new cell's range up front and writing
//! straight into the staging arena's slice.

use crate::apportion::{apportion_with_shuffle, PROPORTIONAL_CLAMP_RNG_DOMAIN};
use crate::world::arena::Arena;
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
///
/// Holds only the metadata that fits in a fixed-size struct —
/// directional state, scalar fields, and the `(mem_start, mem_len)`
/// indices into the world's arena where the cell's actual `u32`
/// slots live. To touch those slots, accessor methods take an
/// [`Arena`] reference; nothing in this struct lets you bypass that.
///
/// `PartialEq` / `Eq` compare only metadata. Two cells are equal
/// iff their metadata fields agree — they may or may not point at
/// the same arena range, and the contents at those ranges don't
/// enter into the equality. For a content-aware comparison, read
/// both cells' slices via [`Cell::memory`] and compare those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Start of the cell's slot range in the world's arena. Has
    /// no meaning when [`Cell::mem_len`] is `0` (sentinel empty
    /// cell) — by convention zeroed out then so two empty cells
    /// compare equal.
    pub(crate) mem_start: u32,

    /// Length of the cell's slot range in the arena. Equals
    /// [`Cell::energy`].
    pub(crate) mem_len: u32,

    /// Directional pointers. `pointers[d]` is the start index
    /// (within the cell's `mem_len`-slot range) of the slot range
    /// that direction `d` will emit next tick.
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

    /// Program counter — index into the cell's `mem_len`-slot range.
    pub pc: u32,

    /// UI lineage marker. Does not influence physics.
    pub origin_tag: u32,

    /// UI appearance / war paint. Does not influence physics.
    pub appearance: u32,
}

impl Cell {
    /// Build an empty cell — no memory, no program, all directional state
    /// at zero. The cell has no claim on any arena range yet; use
    /// [`Cell::with_memory`] (or [`SparseWorld::alloc_cell`] /
    /// [`SparseWorld::insert_with_memory`] at the world level) to put
    /// it into an arena with a memory range attached.
    ///
    /// [`SparseWorld::alloc_cell`]: crate::SparseWorld::alloc_cell
    /// [`SparseWorld::insert_with_memory`]: crate::SparseWorld::insert_with_memory
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mem_start: 0,
            mem_len: 0,
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

    /// Build a cell with the given program / data slots, allocating
    /// space for them in `arena` and copying the contents in.
    ///
    /// Pointers are not laid out; call
    /// [`lay_out_pointers`](Self::lay_out_pointers) once rates are decided.
    ///
    /// The returned cell's `(mem_start, mem_len)` are valid *only*
    /// for the arena passed in. Inserting it into a different
    /// world's [`Cells`](crate::world::Cells) container would
    /// silently corrupt that other world.
    ///
    /// **Accepted-as-unkillable mutant** (`cargo mutants 27.0.0`):
    ///
    /// - `> 0` → `>= 0` on `if len > 0`: the guard avoids a single
    ///   `arena.alloc(0) → arena.slice_mut(0, 0).copy_from_slice(&[])`
    ///   round trip when there's nothing to copy. With `>= 0` the
    ///   code runs the same alloc-and-empty-copy sequence, but
    ///   `alloc(0)` already returns `0` without touching the free
    ///   list and the empty `copy_from_slice` is a no-op. Observable
    ///   output is identical.
    #[must_use]
    pub fn with_memory(arena: &mut Arena, slots: &[u32]) -> Self {
        let mut c = Self::new();
        let len = u32::try_from(slots.len()).unwrap_or(u32::MAX);
        if len > 0 {
            c.mem_start = arena.alloc(len);
            c.mem_len = len;
            arena.slice_mut(c.mem_start, len).copy_from_slice(slots);
        }
        c
    }

    /// Cell energy = `mem_len`. The struct-level authoritative value;
    /// the arena slice length always matches this when the cell is
    /// live.
    #[must_use]
    pub const fn energy(&self) -> u32 {
        self.mem_len
    }

    /// `true` iff the cell holds no energy / memory.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.mem_len == 0
    }

    /// Read-only view of the cell's memory slots from the arena.
    /// Length equals [`Cell::energy`].
    ///
    /// The caller must pass the arena the cell was allocated in;
    /// passing a foreign arena yields a slice over unrelated bytes
    /// (no UB — `(start, len)` is bounds-checked — but the
    /// contents are nonsense).
    #[must_use]
    pub fn memory<'a>(&self, arena: &'a Arena) -> &'a [u32] {
        if self.mem_len == 0 {
            &[]
        } else {
            arena.slice(self.mem_start, self.mem_len)
        }
    }

    /// Length of the cell's memory in slots. Same value as
    /// [`Cell::energy`] but `usize`-typed for indexing arithmetic.
    /// Reads the cached [`Cell::mem_len`] — does not consult the
    /// arena.
    #[must_use]
    pub const fn memory_len(&self) -> usize {
        self.mem_len as usize
    }

    /// Copy out the slot at `i` from the arena. Cheap `u32` read;
    /// avoids the borrow that the slice-indexing form would hold,
    /// which matters in VM arithmetic opcodes that read several
    /// slots then write one.
    #[must_use]
    pub fn memory_slot(&self, arena: &Arena, i: usize) -> u32 {
        arena.get(self.mem_start, i as u32)
    }

    /// Write the slot at `i` in the arena. Pairs with
    /// [`Cell::memory_slot`] so VM opcodes can do
    /// `set_memory_slot(arena, dst, memory_slot(arena, src))` without
    /// fighting the borrow checker.
    pub fn set_memory_slot(&self, arena: &mut Arena, i: usize, v: u32) {
        arena.set(self.mem_start, i as u32, v);
    }

    /// Free the cell's memory range back to the arena, leaving the
    /// cell empty (`mem_len` = 0). Called by world-level removal /
    /// `gc_empty` paths before discarding the metadata.
    pub fn free_memory(&mut self, arena: &mut Arena) {
        arena.free(self.mem_start, self.mem_len);
        self.mem_start = 0;
        self.mem_len = 0;
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

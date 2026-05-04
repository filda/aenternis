//! Sparse world: cells exist only where energy is non-zero.
//!
//! The sparse model (verified in JS prototype 9, see `docs/prototype-09-plan.md`)
//! drops the toroidal grid entirely. The world has no fixed bounding box;
//! its size at any moment is bounded above by the total energy, since one
//! unit of energy occupies one slot of one cell, and a cell with zero
//! slots stops existing.
//!
//! ## Invariants
//!
//! - Every cell in the map has at least one slot of memory (= energy).
//!   The garbage collector ([`gc_empty`](SparseWorld::gc_empty)) enforces
//!   this between ticks.
//! - Iteration order is **deterministic** — the underlying `BTreeMap` walks
//!   coordinates in `(x, y, z)` lexicographic order. This is load-bearing
//!   for snapshot tests and for the bit-identity harness against JS.
//! - The big bang places its single initial cell at [`Coord::ORIGIN`].
//!   World expansion grows outward from there.
//!
//! ## Note on `HashMap`
//!
//! We deliberately use `BTreeMap`, not `HashMap`. Rust's `HashMap` randomizes
//! its hasher per process (DoS-defense default), which would make iteration
//! order non-deterministic across runs and break bit-identity testing.
//! A perf-tuned `FxHashMap` plus an explicit sort step might come later,
//! but only if profiling proves it's worth the extra moving parts.

use std::collections::btree_map::{Iter, IterMut, Keys};
use std::collections::BTreeMap;

use crate::{Cell, Coord, Direction, Rng};

/// Sparse world container.
#[derive(Debug, Clone)]
pub struct SparseWorld {
    /// Cells indexed by coordinate. Iteration order is the `BTreeMap`
    /// canonical order: `(x, y, z)` lexicographic.
    pub cells: BTreeMap<Coord, Cell>,

    /// Seed used for any deterministic randomness in this world. Combined
    /// with the current tick and per-cell coords by [`Rng::for_cell_at_tick`]
    /// to produce per-cell streams.
    pub world_seed: u64,

    /// Current tick count. Starts at zero, monotonically increasing.
    pub tick: u64,

    /// Threshold for collision-as-soft-mixing (`docs/mechanics.md`).
    /// `dominance = clamp(1 - target_E / (attacker_E_post_burn *
    /// move_threshold), 0, 1)`. Default `2.0`.
    pub move_threshold: f32,
}

impl SparseWorld {
    /// Default value for [`SparseWorld::move_threshold`].
    pub const DEFAULT_MOVE_THRESHOLD: f32 = 2.0;

    /// Build an empty world. No cells exist yet; the caller is responsible
    /// for inserting any initial state (typically via [`big_bang`](Self::big_bang)).
    /// `move_threshold` defaults to [`Self::DEFAULT_MOVE_THRESHOLD`].
    #[must_use]
    pub const fn new(world_seed: u64) -> Self {
        Self {
            cells: BTreeMap::new(),
            world_seed,
            tick: 0,
            move_threshold: Self::DEFAULT_MOVE_THRESHOLD,
        }
    }

    /// Build a world initialized as a big bang — one cell at [`Coord::ORIGIN`]
    /// holding the entire energy budget.
    ///
    /// The origin cell's `origin_tag` and its memory slots are drawn from
    /// **the same per-cell-at-tick stream** keyed by `(world_seed, 0, ORIGIN)`,
    /// matching prototype 9's `makeCell` / `bigBang` pair: first `next_u32()`
    /// for the tag, then `energy` more for the memory slots. Same seed and
    /// same energy produce the same initial state on every run, bit-identical
    /// across host platforms.
    ///
    /// `energy == 0` produces an empty world (no cell at the origin), since
    /// a cell with zero energy does not exist by the world invariant.
    #[must_use]
    pub fn big_bang(world_seed: u64, energy: u32) -> Self {
        Self::big_bang_with_program(world_seed, energy, &[])
    }

    /// Big bang with a programmer-supplied prefix written into the origin
    /// cell's memory. The first `min(program.len(), energy)` slots are
    /// taken verbatim from `program`; the remaining slots (if `energy >
    /// program.len()`) are filled from the per-cell-at-tick RNG stream.
    ///
    /// Matches prototype 9's `bigBang(eTotal, programSlots)` semantics:
    /// the RNG is **not** advanced for slots covered by the program, so
    /// for a fixed seed, `big_bang_with_program(seed, n, &[a, b, c])` and
    /// `big_bang_with_program(seed, n, &[d, e, f])` produce identical
    /// memory at indices 3..n.
    ///
    /// `program.len() > energy` truncates: extra slots are discarded.
    /// An empty `program` is exactly equivalent to [`SparseWorld::big_bang`].
    #[must_use]
    pub fn big_bang_with_program(world_seed: u64, energy: u32, program: &[u32]) -> Self {
        let mut world = Self::new(world_seed);
        if energy == 0 {
            return world;
        }
        let mut rng = Rng::for_cell_at_tick(world_seed, 0, Coord::ORIGIN);
        let origin_tag = rng.next_u32();

        let energy_usize = energy as usize;
        let n_program = program.len().min(energy_usize);
        let mut memory = Vec::with_capacity(energy_usize);
        memory.extend_from_slice(&program[..n_program]);
        for _ in n_program..energy_usize {
            memory.push(rng.next_u32());
        }

        let mut cell = Cell::with_memory(memory);
        cell.origin_tag = origin_tag;
        world.cells.insert(Coord::ORIGIN, cell);
        world
    }

    /// Get a mutable reference to the cell at `coord`, allocating an empty
    /// one if it does not yet exist.
    ///
    /// Newly allocated cells start empty (`energy == 0`) but with an
    /// `origin_tag` deterministically derived from `(world_seed, coord)`,
    /// so two runs of the same simulation produce the same tag at every
    /// allocation site — even for cells that came into being mid-run via
    /// alloc-on-write during the inflow phase.
    pub fn get_or_alloc(&mut self, coord: Coord) -> &mut Cell {
        let world_seed = self.world_seed;
        self.cells
            .entry(coord)
            .or_insert_with(|| fresh_cell(world_seed, coord))
    }

    /// Number of cells currently in the world.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// `true` iff there are no cells at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// `true` iff a cell exists at `coord`.
    #[must_use]
    pub fn contains(&self, coord: Coord) -> bool {
        self.cells.contains_key(&coord)
    }

    /// Borrow the cell at `coord`, if any.
    #[must_use]
    pub fn get(&self, coord: Coord) -> Option<&Cell> {
        self.cells.get(&coord)
    }

    /// Mutably borrow the cell at `coord`, if any.
    pub fn get_mut(&mut self, coord: Coord) -> Option<&mut Cell> {
        self.cells.get_mut(&coord)
    }

    /// Insert (or replace) a cell at `coord`. Returns the previous cell if
    /// one existed there, mirroring `BTreeMap::insert`.
    pub fn insert(&mut self, coord: Coord, cell: Cell) -> Option<Cell> {
        self.cells.insert(coord, cell)
    }

    /// Remove the cell at `coord`, returning it if it was there.
    pub fn remove(&mut self, coord: Coord) -> Option<Cell> {
        self.cells.remove(&coord)
    }

    /// Borrow the orthogonal neighbor of `coord` in `direction`, if any.
    #[must_use]
    pub fn neighbor(&self, coord: Coord, direction: Direction) -> Option<&Cell> {
        self.cells.get(&coord.neighbor(direction))
    }

    /// Energy of the orthogonal neighbor of `coord` in `direction`. Returns
    /// `0` if the neighbor does not exist — the natural "empty space" value
    /// for diffusion gradients.
    #[must_use]
    pub fn neighbor_energy(&self, coord: Coord, direction: Direction) -> u32 {
        self.neighbor(coord, direction).map_or(0, Cell::energy)
    }

    /// Sum of all cell energies in the world. The result is `u64` to
    /// accommodate worlds with `E_total` close to `u32::MAX` without
    /// overflow during summation (and without needing saturating math).
    #[must_use]
    pub fn total_energy(&self) -> u64 {
        self.cells.values().map(|c| u64::from(c.energy())).sum()
    }

    /// Drop every cell whose memory is empty (energy == 0). This is the
    /// garbage-collection step in the per-tick cycle described in
    /// `docs/mechanics.md` — the sparse-world counterpart of "the cell
    /// stops existing once it holds no energy."
    pub fn gc_empty(&mut self) {
        self.cells.retain(|_, cell| !cell.is_empty());
    }

    /// Iterate over `(coord, cell)` pairs in canonical order.
    pub fn iter(&self) -> Iter<'_, Coord, Cell> {
        self.cells.iter()
    }

    /// Mutably iterate over `(coord, cell)` pairs in canonical order.
    pub fn iter_mut(&mut self) -> IterMut<'_, Coord, Cell> {
        self.cells.iter_mut()
    }

    /// Iterate over cell coordinates in canonical order.
    pub fn coords(&self) -> Keys<'_, Coord, Cell> {
        self.cells.keys()
    }
}

/// Build an empty cell whose `origin_tag` is the first `u32` from the
/// per-cell-at-tick stream `(world_seed, 0, coord)`. Used by
/// [`SparseWorld::get_or_alloc`] for the alloc-on-write path.
fn fresh_cell(world_seed: u64, coord: Coord) -> Cell {
    let mut rng = Rng::for_cell_at_tick(world_seed, 0, coord);
    let mut cell = Cell::new();
    cell.origin_tag = rng.next_u32();
    cell
}

// `IntoIterator` impls so that `for (coord, cell) in &world` and
// `for (coord, cell) in &mut world` work — clippy::iter_without_into_iter
// complains otherwise.

impl<'a> IntoIterator for &'a SparseWorld {
    type Item = (&'a Coord, &'a Cell);
    type IntoIter = Iter<'a, Coord, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl<'a> IntoIterator for &'a mut SparseWorld {
    type Item = (&'a Coord, &'a mut Cell);
    type IntoIter = IterMut<'a, Coord, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter_mut()
    }
}

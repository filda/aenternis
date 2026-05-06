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
//! - Iteration order is **stable per run** — the underlying `FxHashMap`
//!   uses a deterministic, non-randomized hasher, so the same insertion
//!   sequence walks the same way every run. The order is *not* the
//!   `(x, y, z)` lex order; APIs that need that contract (such as
//!   [`SparseWorld::sorted_iter`] and the WASM `cellsSnapshot`) sort
//!   explicitly at the boundary.
//! - The big bang places its single initial cell at [`Coord::ORIGIN`].
//!   World expansion grows outward from there.
//!
//! ## Why `FxHashMap`
//!
//! Profiling [`crate::tick::step`] on a sparse world with a few thousand
//! cells showed ~60 % of CPU time in `BTreeMap::get` and `Coord::cmp`
//! (the `cpu_phase` and `compute_natural_rates` neighbor lookups call
//! `world.cells.get(...)` six times per cell per tick). `FxHashMap`
//! collapses each lookup from `O(log n)` tree descent to a single hash
//! plus probe, with much better cache behaviour. The hasher
//! ([`rustc_hash::FxBuildHasher`]) is non-randomized so iteration order
//! is reproducible across runs of the same binary — sufficient for the
//! bit-identity harness against JS, which compares per-cell state at
//! known coordinates rather than relying on iteration order.

use std::collections::hash_map::{Iter, IterMut, Keys};

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::rng::cell_seed_xs32;
use crate::{Cell, Coord, Direction, Rng, RngKind};

/// Sparse world container.
///
/// The four `legacy_*` bool fields are independent diagnostic toggles
/// that select between Rust-native semantics and JS prototype 9-B
/// compat for each phase that has a known divergence (RNG-tick offset,
/// `f64` precision in stochastic-floor, `port` wrapping, opcode set).
/// Refactoring them into a single state enum would conflate orthogonal
/// concerns; `clippy::struct_excessive_bools` is silenced here on
/// purpose.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct SparseWorld {
    /// Cells indexed by coordinate. Iteration order is the `FxHashMap`
    /// internal hash order — stable across runs for the same insertion
    /// sequence but **not** lex by `(x, y, z)`. Use
    /// [`SparseWorld::sorted_iter`] when canonical order is required.
    pub cells: FxHashMap<Coord, Cell>,

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

    /// Which RNG backend powers per-cell-tick streams and origin-tag
    /// derivation. Default [`RngKind::Pcg`] for production; switch to
    /// [`RngKind::Xorshift32`] for bit-identity comparison against JS
    /// prototype 9-B. Mutating after [`Self::big_bang`] runs leaves the
    /// initial cell state inconsistent — set this on the world *before*
    /// the first cell is allocated, or via [`Self::big_bang_with_kind`].
    pub rng_kind: RngKind,

    /// When `true`, [`crate::tick::compute_natural_rates`] keys the
    /// per-cell-tick RNG with `tick - 1` instead of `tick` (saturating
    /// at zero for the very first step). This matches JS prototype
    /// 9-B's quirk of running `recomputeAllLayouts()` at the end of
    /// `step()` *before* incrementing `this.tick`, so layouts that
    /// drive step #N's CPU phase were computed with `rng_tick = N - 2`.
    /// Default `false` (Rust native: layout for tick N uses `rng_tick`
    /// = N). Toggling mid-run is safe — the change applies on the next
    /// tick.
    pub legacy_tick_offset: bool,

    /// When `true`, [`crate::tick::compute_natural_rates`] computes the
    /// stochastic-floor comparison in `f64` using all 32 bits of the
    /// RNG output (matches JS prototype 9-B, where `Number` is `f64`
    /// natively). When `false` (default), Rust truncates the RNG output
    /// to 24 bits and runs the comparison in `f32` — clean for
    /// production but leaves a sub-ULP gap around `frac` boundary
    /// values where the two paths can disagree per draw. Toggling
    /// mid-run is safe; the change applies on the next tick.
    pub legacy_full_precision: bool,

    /// When `true`, the `port` opcode accumulates into `active_outflow`
    /// with **wrapping** addition (modulo `2^32`), matching JS prototype
    /// 9-B's `(activeOutflow + arg1) >>> 0`. When `false` (default),
    /// the addition saturates at `u32::MAX` — safer for production code
    /// but produces a *much* more symmetric outflow pattern when noise
    /// memory triggers many `port` opcodes in one tick: every triggered
    /// direction saturates, so the proportional clamp splits emission
    /// evenly. With wrap, individual directions hold residual values
    /// from `mod 2^32` arithmetic and the dominant direction wins,
    /// which is what 9-B's visible asymmetric expansion comes from.
    /// Toggling mid-run is safe.
    pub legacy_port_wrap: bool,

    /// When `true`, the VM treats opcodes `0x14` (`sinflow`), `0x15`
    /// (`sself`), and `0x16` (`srate`) as **unknown** — same as any
    /// `> 0x16` byte: PC advances by 1 slot and nothing else happens.
    /// JS prototype 9-B only defines 20 opcodes (`0x00..=0x13`), so
    /// the three Rust additions don't exist there; without this flag,
    /// noise memory that happens to encode one of those bytes triggers
    /// a sinflow / sself / srate execution in Rust but a single-slot
    /// nop in JS. Over a tick of 9830 random instructions that's
    /// ~115 PC-walk divergences per cell, more than enough to drift
    /// the post-clamp outflow distribution.
    /// Toggling mid-run is safe.
    pub legacy_opcode_set: bool,
}

impl SparseWorld {
    /// Default value for [`SparseWorld::move_threshold`].
    pub const DEFAULT_MOVE_THRESHOLD: f32 = 2.0;

    /// Build an empty world. No cells exist yet; the caller is responsible
    /// for inserting any initial state (typically via [`big_bang`](Self::big_bang)).
    /// `move_threshold` defaults to [`Self::DEFAULT_MOVE_THRESHOLD`] and
    /// `rng_kind` defaults to [`RngKind::Pcg`].
    #[must_use]
    pub const fn new(world_seed: u64) -> Self {
        Self {
            cells: FxHashMap::with_hasher(FxBuildHasher),
            world_seed,
            tick: 0,
            move_threshold: Self::DEFAULT_MOVE_THRESHOLD,
            rng_kind: RngKind::Pcg,
            legacy_tick_offset: false,
            legacy_full_precision: false,
            legacy_port_wrap: false,
            legacy_opcode_set: false,
        }
    }

    /// Same as [`Self::new`] but with an explicit RNG backend choice.
    #[must_use]
    pub const fn new_with_kind(world_seed: u64, rng_kind: RngKind) -> Self {
        Self {
            cells: FxHashMap::with_hasher(FxBuildHasher),
            world_seed,
            tick: 0,
            move_threshold: Self::DEFAULT_MOVE_THRESHOLD,
            rng_kind,
            legacy_tick_offset: false,
            legacy_full_precision: false,
            legacy_port_wrap: false,
            legacy_opcode_set: false,
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
        Self::big_bang_with_program_and_kind(world_seed, energy, program, RngKind::Pcg)
    }

    /// Big bang with both a program and an explicit RNG backend. The
    /// `Xorshift32` path matches JS prototype 9-B's `bigBang` semantics
    /// bit-for-bit: origin cell's `origin_tag` is `cellSeed(seed, ORIGIN)`
    /// directly (not the first RNG draw, as PCG does), and the noise
    /// suffix in memory is the `xorshift32(cellSeed)` stream.
    #[must_use]
    pub fn big_bang_with_program_and_kind(
        world_seed: u64,
        energy: u32,
        program: &[u32],
        rng_kind: RngKind,
    ) -> Self {
        let mut world = Self::new_with_kind(world_seed, rng_kind);
        if energy == 0 {
            return world;
        }

        let (origin_tag, mut noise_rng) = match rng_kind {
            RngKind::Pcg => {
                let mut rng = Rng::for_cell_at_tick(world_seed, 0, Coord::ORIGIN);
                let tag = rng.next_u32();
                (tag, rng)
            }
            RngKind::Xorshift32 => {
                // JS prototype 9-B: `originTag = cellSeed(seed, x, y, z)`
                // (the seed value itself), and `cell.rng = makeRng(seed)`
                // is a separate xorshift32 stream from that same seed —
                // the tag is *not* the first draw, it's the seed value.
                let tag = cell_seed_xs32(world_seed, Coord::ORIGIN);
                let rng = Rng::new_xs32(tag);
                (tag, rng)
            }
        };

        let energy_usize = energy as usize;
        let n_program = program.len().min(energy_usize);
        let mut memory = Vec::with_capacity(energy_usize);
        memory.extend_from_slice(&program[..n_program]);
        for _ in n_program..energy_usize {
            memory.push(noise_rng.next_u32());
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
        let rng_kind = self.rng_kind;
        self.cells
            .entry(coord)
            .or_insert_with(|| fresh_cell(world_seed, coord, rng_kind))
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

    /// Bounding box across all live cells, as
    /// `(x_min, x_max, y_min, y_max, z_min, z_max)`. Returns `None` when
    /// the world is empty.
    ///
    /// `O(n)` in the cell count — walks the whole map. Cheap enough at
    /// the prototype's million-cell scale (one tick of `step` is already
    /// `O(n)`); upgrade to a maintained-on-write bbox if profiling ever
    /// flags this.
    #[must_use]
    pub fn bounding_box(&self) -> Option<(i32, i32, i32, i32, i32, i32)> {
        // Single-pass fold over coords. Delegating min/max to the stdlib
        // means there are no inline `<` / `>` comparisons left for a
        // mutator to flip — the ordering logic lives in `i32::min` /
        // `i32::max`, which are tested by stdlib itself.
        self.cells.keys().fold(None, |acc, c| {
            Some(acc.map_or(
                (c.x, c.x, c.y, c.y, c.z, c.z),
                |(x_min, x_max, y_min, y_max, z_min, z_max)| {
                    (
                        x_min.min(c.x),
                        x_max.max(c.x),
                        y_min.min(c.y),
                        y_max.max(c.y),
                        z_min.min(c.z),
                        z_max.max(c.z),
                    )
                },
            ))
        })
    }

    /// Drop every cell whose memory is empty (energy == 0). This is the
    /// garbage-collection step in the per-tick cycle described in
    /// `docs/mechanics.md` — the sparse-world counterpart of "the cell
    /// stops existing once it holds no energy."
    pub fn gc_empty(&mut self) {
        self.cells.retain(|_, cell| !cell.is_empty());
    }

    /// Iterate over `(coord, cell)` pairs in `FxHashMap` hash order
    /// (deterministic per run, not lex). For canonical lex order, use
    /// [`Self::sorted_iter`].
    #[must_use]
    pub fn iter(&self) -> Iter<'_, Coord, Cell> {
        self.cells.iter()
    }

    /// Mutably iterate over `(coord, cell)` pairs in hash order.
    pub fn iter_mut(&mut self) -> IterMut<'_, Coord, Cell> {
        self.cells.iter_mut()
    }

    /// Iterate over cell coordinates in hash order.
    #[must_use]
    pub fn coords(&self) -> Keys<'_, Coord, Cell> {
        self.cells.keys()
    }

    /// Iterate over `(coord, cell)` pairs in `(x, y, z)` lex order.
    ///
    /// Allocates a `Vec` of references on each call to do the sort, so
    /// avoid in tight inner loops — it's intended for snapshot/export
    /// boundaries (see `aenternis-wasm`'s `cellsSnapshot`) and for tests
    /// that pin canonical iteration order.
    #[must_use]
    pub fn sorted_iter(&self) -> std::vec::IntoIter<(&Coord, &Cell)> {
        let mut entries: Vec<_> = self.cells.iter().collect();
        entries.sort_unstable_by_key(|(c, _)| **c);
        entries.into_iter()
    }
}

/// Build an empty cell whose `origin_tag` is deterministic in
/// `(world_seed, coord)`. Used by [`SparseWorld::get_or_alloc`] for the
/// alloc-on-write path during inflow.
///
/// The two backends derive the tag differently — see
/// [`SparseWorld::big_bang_with_program_and_kind`] for the rationale:
/// PCG draws the tag from a fresh per-cell-tick stream, `Xorshift32`
/// uses the JS `cellSeed` value verbatim as the tag.
const fn fresh_cell(world_seed: u64, coord: Coord, rng_kind: RngKind) -> Cell {
    let origin_tag = match rng_kind {
        RngKind::Pcg => {
            let mut rng = Rng::for_cell_at_tick(world_seed, 0, coord);
            rng.next_u32()
        }
        RngKind::Xorshift32 => cell_seed_xs32(world_seed, coord),
    };
    let mut cell = Cell::new();
    cell.origin_tag = origin_tag;
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

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

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::genesis::{generate_into, GenesisConfig};
use crate::rng::cell_seed;
use crate::world::{Arena, Cells};
use crate::{Cell, Coord, Direction, Rng};

/// Sparse world container.
///
/// All formerly-toggleable simulation behaviours (`rng_kind`,
/// `legacy_tick_offset`, `legacy_full_precision`, `legacy_port_wrap`,
/// `legacy_opcode_set`) are now hardcoded — the comparison work that
/// motivated those switches is done and the always-on path is the only
/// one that runs.
#[derive(Debug, Clone)]
pub struct SparseWorld {
    /// Cells indexed by coordinate. Iteration order is the
    /// `coord_to_slot` hashmap order (immutable iteration) or slot
    /// order (mutable / parallel iteration) — stable across runs
    /// for the same insertion sequence but **not** lex by
    /// `(x, y, z)`. Use [`SparseWorld::sorted_iter`] when canonical
    /// order is required.
    ///
    /// Visible inside the crate (`pub(crate)`) so the tick loop's
    /// per-cell closures can mutate cells directly; external callers
    /// go through [`SparseWorld::get`] / [`SparseWorld::iter`] /
    /// [`SparseWorld::iter_mut`].
    pub(crate) cells: Cells,

    /// Current arena holding every cell's memory slots.
    ///
    /// Cells store `(mem_start, mem_len)` indices into this buffer;
    /// per-cell `Vec<u32>`s do not exist. Pre-allocated at
    /// [`SparseWorld::big_bang`] to the world's total energy so
    /// no growth happens during `step`. Test paths that go through
    /// [`SparseWorld::new`] start with a zero-capacity arena that
    /// grows on demand (the `Arena::alloc` slow path); production
    /// paths via `big_bang` never trigger that growth.
    ///
    /// Paired with [`SparseWorld::arena_next`]: each
    /// [`crate::tick::apply_outflow`] reads from this arena and
    /// writes the next tick's compacted state into `arena_next`,
    /// then swaps. Bump-only allocation per tick, no in-place
    /// free-list churn — the structural fragmentation fix of the
    /// Phase 3 arena refactor (`docs/optimalizace-2026-05.md`).
    ///
    /// Accessed `pub(crate)` so tick-phase split-borrows can pair
    /// `&mut self.cells` with `&self.arena` (immutable phases) or
    /// `&mut self.arena` (sequential phases) without an
    /// intermediate accessor.
    pub(crate) arena: Arena,

    /// Staging arena written by [`crate::tick::apply_outflow`] —
    /// each tick the post-outflow / post-inflow cell layout is
    /// computed, prefix-summed for offsets, and copied into here
    /// before [`std::mem::swap`] makes it the new `arena`.
    ///
    /// Held at the same capacity as [`SparseWorld::arena`] so a
    /// `swap` is a constant-time pointer flip. Pre-allocated by
    /// `big_bang`; the [`Arena::clear`] call at the top of each
    /// `apply_outflow` resets it to one big free range so the
    /// bump allocator inside `apply_outflow` starts from zero.
    pub(crate) arena_next: Arena,

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

    /// Gravity coupling strength — how hard energy is pulled toward
    /// local mass. Enters the per-direction drive in
    /// [`crate::tick::compute_natural_rates`] as `gravity * (M_nbr − M_c)`.
    /// **Default `0.0`**: with gravity and [`Self::pressure`] both zero the
    /// rate path takes a frozen fast path that is byte-for-byte the
    /// pre-gravity code, so existing baselines need no re-bless. See
    /// `docs/gravity-plan.md`.
    pub gravity: f64,

    /// Mass coupling — the fraction of a cell's energy that behaves as
    /// gravitational mass (`m = gravity_alpha · E`). Folded into the
    /// neighborhood mass `M`. Default `0.0`.
    pub gravity_alpha: f64,

    /// Cutoff radius `R` for the gravitational potential
    /// `M(c) = gravity_alpha · Σ_{0<|d|≤R} E(c+d) / |d|` (a `1/r` kernel,
    /// so the force falls off as `~1/r²`). `R = 1` reduces to the six
    /// face neighbors (a purely local density); larger `R` lets distant
    /// mass attract across voids — genuine long-range gravity — at an
    /// `O(N·R³)` cost. Default `1`. Inactive while [`Self::gravity`] is
    /// `0.0`. See `docs/gravity-plan.md`.
    pub gravity_radius: i32,

    /// Pressure amplitude — the outward counter-force that grows with
    /// density. Enters the drive as `Π(E_c) − Π(E_nbr)` where
    /// `Π(E) = pressure · eref · (E/eref)^γ`. Default `0.0`.
    pub pressure: f64,

    /// Polytropic index γ for the pressure law `Π(E) ∝ (E/eref)^γ`.
    /// Restricted at runtime to portable values
    /// `{1.0, 1.5, 2.0, 2.5, 3.0}` (evaluated via multiply/`sqrt` chains,
    /// all IEEE correctly-rounded) so the rate path stays bit-for-bit
    /// reproducible across native and wasm. Arbitrary γ would need a
    /// non-portable `powf` and is out of scope. Default `2.0`.
    pub pressure_gamma: f64,

    /// Reference energy `eref` for the pressure law — the density at
    /// which `Π = pressure · eref`. Default `1.0`. Inactive while
    /// [`Self::pressure`] is `0.0`.
    pub pressure_eref: f64,

    /// Density-coupled mutation ceiling. The per-slot, per-tick bit-flip
    /// probability for a cell of energy `E` is
    /// `mutation_strength · E / (E + mutation_half_density)` — a saturating
    /// curve: ~0 for a tiny cell (a 1-slot program does nothing anyway),
    /// rising toward `mutation_strength` as density grows. So gravity
    /// wells (dense cores) become the "mutagenic cauldrons" of
    /// `docs/gravity-plan.md` while dispersed / player-scale cells stay
    /// gentle. A bit flip changes a slot's *value*, never the slot count,
    /// so energy is conserved. **Default `0.0` = off**: the mutation phase
    /// is then a strict no-op (no RNG drawn), leaving all baselines
    /// unchanged. Expected range `[0, 1]` (`1` = up to ~100 % at the core).
    pub mutation_strength: f64,

    /// Half-saturation density `K` for the mutation curve: the cell energy
    /// at which the flip probability reaches `mutation_strength / 2`.
    /// **High** (default `40_000`) so only gravity-concentrated dense cores
    /// mutate hard and a player's few-thousand-energy entity stays gentle.
    /// `K = 0` makes the flip probability density-independent (`= strength`
    /// for any non-empty cell). Inactive while [`Self::mutation_strength`]
    /// is `0.0`. See `docs/gravity-plan.md`.
    pub mutation_half_density: f64,

    /// Per-tick scratch: neighbor-energy snapshot indexed by cell coord.
    /// Built once at the start of [`crate::tick::step`] and shared
    /// between `compute_natural_rates` and `cpu_phase`, both of which
    /// would otherwise build their own. Cleared (not freed) between
    /// ticks so the backing storage is reused across the whole run.
    pub(crate) scratch_neighbor_energies: FxHashMap<Coord, [u32; Direction::COUNT]>,

    /// Per-tick scratch: gravitational mass `M = gravity_alpha · Σ E_nbr`
    /// indexed by cell coord, where the sum is over the cell's six
    /// orthogonal neighbors (cutoff radius R = 1, so `|d| = 1` for every
    /// term and the `1/|d|` kernel is the identity). Built from
    /// [`Self::scratch_neighbor_energies`] at the top of
    /// [`crate::tick::compute_natural_rates`] **only when
    /// [`Self::gravity`] is non-zero** — left empty (and thus zero-cost)
    /// on the gravity-off fast path. A void coord has no entry, so its
    /// mass reads as `0.0`, which makes gravity hold energy against the
    /// open boundary rather than leak it (see `docs/gravity-plan.md`).
    /// Cleared (not freed) between ticks like the other scratch maps.
    pub(crate) scratch_mass: FxHashMap<Coord, f64>,

    /// Per-tick scratch: outflow buffer used by
    /// [`crate::tick::collect_outflow`]. Reused across ticks so the
    /// per-direction `Vec<u32>` capacities stay allocated even when
    /// rates fluctuate inside their typical range — at a few hundred
    /// thousand cells this avoids `~6 × n_cells` `Vec` allocations per
    /// tick and was the single biggest win of the parallelization
    /// pass.
    ///
    /// [`crate::tick::step`] pulls this field out via [`std::mem::take`]
    /// during the outflow phase (so `&mut world.cells` and `&Outflow`
    /// can coexist for [`crate::tick::apply_outflow`]) and puts it back
    /// before the tick ends.
    pub(crate) scratch_outflow: FxHashMap<Coord, [Vec<u32>; Direction::COUNT]>,

    /// Per-tick scratch: per-target inflow lists used by
    /// [`crate::tick::apply_outflow`] phase 2/3. The value `Vec`
    /// capacity is reused across ticks — at ~200 k targets per tick
    /// this avoids ~200 k `Vec::with_capacity(0).reserve(N)` cycles
    /// on the per-target inflow buffer.
    ///
    /// Same `mem::take` pattern as [`Self::scratch_outflow`]: the
    /// outflow phase pulls it out so the per-target apply can hold
    /// `&mut world.cells` while still reading the populated inflow
    /// lists, then puts it back.
    pub(crate) scratch_inflows_by_target: FxHashMap<Coord, Vec<crate::tick::InflowEntry>>,

    /// Per-tick scratch: per-source total outflow used by
    /// [`crate::tick::apply_outflow`] phase 1. Same `mem::take` /
    /// `clear()` reuse pattern as the other scratch maps — at
    /// ~700 k sources a freshly-built `FxHashMap::default()` plus
    /// `reserve(N)` rounds up to ~1 M hashbrown slots × 17 bytes,
    /// which is a 17 MB churn per tick and was the alloc that hit
    /// `__rust_alloc_error_handler` in the WASM build under
    /// fragmented 32-bit address space.
    pub(crate) scratch_per_source_total_outflow: FxHashMap<Coord, u32>,

    /// Per-tick scratch: per-target inflow lists for the fused
    /// outflow phase ([`crate::tick::outflow_phase_inplace`]). Sibling
    /// to [`Self::scratch_inflows_by_target`], but the entry type
    /// ([`crate::tick::InflowFast`]) carries pre-resolved
    /// `(head_start, head_len, wrap_start, wrap_len)` ranges into
    /// `arena_cur` instead of `(source_coord, source_dir)` — so the
    /// write phase reads source slots directly without re-traversing
    /// any `world.cells` lookup. Filled by the fused path only;
    /// `apply_outflow` (the public reference impl used by tests and
    /// `step_diffusion`) keeps using the legacy map.
    pub(crate) scratch_inflows_fast: FxHashMap<Coord, Vec<crate::tick::InflowFast>>,

    /// Lex-sorted snapshot of `cells.keys()`. Mirrors the canonical
    /// `(x, y, z)` order that [`Self::sorted_iter`] commits to.
    ///
    /// Maintained by [`Self::rebuild_indices_if_dirty`], which the
    /// tick loop runs after `gc_empty` and before the tick counter
    /// advances. Reading [`Self::sorted_iter`] outside that contract
    /// (after a manual mutation, before a tick) requires calling
    /// `rebuild_indices_if_dirty` first — `debug_assert` enforces it.
    pub(crate) sorted_cache: Vec<Coord>,

    /// `true` if at least one mutation since the last
    /// `rebuild_indices_if_dirty` added or removed a key, so
    /// `sorted_cache` is stale. Pure value replacement (e.g.
    /// [`Self::insert`] over an existing coord) leaves it `false`.
    pub(crate) sorted_dirty: bool,

    /// Cached `(x_min, x_max, y_min, y_max, z_min, z_max)` over all
    /// live cells, `None` for the empty world. Maintained eagerly on
    /// insert (incremental extend) and rebuilt lazily on
    /// remove/`gc_empty` (full fold over `sorted_cache`).
    pub(crate) bbox_cache: Option<(i32, i32, i32, i32, i32, i32)>,

    /// `true` if a removal since the last `rebuild_indices_if_dirty`
    /// may have shrunk the bbox; the next rebuild does a full fold to
    /// recompute. Inserts don't set this — they extend bbox in place.
    pub(crate) bbox_dirty: bool,
}

/// How the origin cell's whole memory is filled at big bang (see
/// `docs/genesis-plan.md`, A6). Orthogonal to the optional player
/// `overlay` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// Deterministic `xorshift32` noise (the legacy fill). Under the
    /// opcode fold every byte is still an executable instruction, so this
    /// is "active chaos", not inert — kept for baselines and tests.
    Noise,
    /// Procedural macro genesis: a seed-driven weighted stream of macros
    /// over the whole memory (`crate::genesis`). The information-bearing
    /// default for new worlds.
    Macros,
}

impl SparseWorld {
    /// Default value for [`SparseWorld::move_threshold`].
    pub const DEFAULT_MOVE_THRESHOLD: f32 = 2.0;

    /// Default polytropic index γ for [`SparseWorld::pressure_gamma`].
    /// Pressure itself defaults to `0.0`, so this only takes effect once
    /// a caller turns pressure on.
    pub const DEFAULT_PRESSURE_GAMMA: f64 = 2.0;

    /// Default reference energy `eref` for [`SparseWorld::pressure_eref`].
    pub const DEFAULT_PRESSURE_EREF: f64 = 1.0;

    /// Default half-saturation density `K` for
    /// [`SparseWorld::mutation_half_density`]. High so only dense cores
    /// mutate appreciably; `mutation_strength` defaults to `0` (off), so
    /// this only bites once a caller turns mutation on.
    pub const DEFAULT_MUTATION_HALF_DENSITY: f64 = 40_000.0;

    /// Build an empty world. No cells exist yet; the caller is responsible
    /// for inserting any initial state (typically via [`big_bang`](Self::big_bang)).
    /// `move_threshold` defaults to [`Self::DEFAULT_MOVE_THRESHOLD`].
    ///
    /// The arena starts at capacity zero — the first
    /// [`insert_with_memory`](Self::insert_with_memory) (or any
    /// other arena-allocating helper) will grow it on demand. For
    /// predictable behaviour (no implicit `Vec::resize` during
    /// `step`), prefer [`big_bang`](Self::big_bang) which sizes the
    /// arena to total energy up front.
    ///
    /// No longer `const fn` because `Arena::with_capacity` does a
    /// `Vec::with_capacity` and `Vec::resize`, neither of which are
    /// const yet.
    #[must_use]
    pub fn new(world_seed: u64) -> Self {
        Self::with_capacity(world_seed, 0)
    }

    /// Build an empty world whose arena is pre-allocated to
    /// `capacity` slots. The arena does not grow during `step` as
    /// long as total energy stays at or below `capacity` (which is
    /// guaranteed by energy conservation when the caller passes
    /// the simulation's total energy).
    #[must_use]
    pub fn with_capacity(world_seed: u64, capacity: u32) -> Self {
        // Only the two arenas get pre-reserved to `capacity` —
        // that's the structural fragmentation fix from Phase 2/3.
        // Reserving the other cell-keyed containers (cells.slots,
        // coord_to_slot, scratch maps, sorted_cache) to the same
        // bound seemed like a clean tidy-up in Phase 4, but in
        // practice it asks the WASM allocator for a single
        // ~140 MB `Vec<Option<(Coord, Cell)>>` block up front at
        // `big_bang(_, 1_000_000)`, which fails in the shared-
        // memory environment even though `--max-memory=4 GiB`
        // would in theory allow it. The growing-by-doubling cost
        // these containers pay during the first few hundred ticks
        // is sequential (only `apply_outflow`'s alloc-on-write
        // grows `cells`, only the first-tick fill grows scratch
        // maps), so it doesn't reintroduce the per-cell allocator
        // contention the arena refactor was here to fix.
        Self {
            cells: Cells::new(),
            arena: Arena::with_capacity(capacity),
            arena_next: Arena::with_capacity(capacity),
            world_seed,
            tick: 0,
            move_threshold: Self::DEFAULT_MOVE_THRESHOLD,
            gravity: 0.0,
            gravity_alpha: 0.0,
            gravity_radius: 1,
            pressure: 0.0,
            pressure_gamma: Self::DEFAULT_PRESSURE_GAMMA,
            pressure_eref: Self::DEFAULT_PRESSURE_EREF,
            mutation_strength: 0.0,
            mutation_half_density: Self::DEFAULT_MUTATION_HALF_DENSITY,
            scratch_neighbor_energies: FxHashMap::with_hasher(FxBuildHasher),
            scratch_mass: FxHashMap::with_hasher(FxBuildHasher),
            scratch_outflow: FxHashMap::with_hasher(FxBuildHasher),
            scratch_inflows_by_target: FxHashMap::with_hasher(FxBuildHasher),
            scratch_per_source_total_outflow: FxHashMap::with_hasher(FxBuildHasher),
            scratch_inflows_fast: FxHashMap::with_hasher(FxBuildHasher),
            sorted_cache: Vec::new(),
            sorted_dirty: false,
            bbox_cache: None,
            bbox_dirty: false,
        }
    }

    /// Build a world initialized as a big bang — one cell at [`Coord::ORIGIN`]
    /// holding the entire energy budget.
    ///
    /// `origin_tag = cell_seed(world_seed, ORIGIN)` (the seed value
    /// itself, not the first RNG draw); the memory slots are the
    /// `xorshift32(cell_seed)` stream. Same seed and same energy
    /// produce the same initial state on every run, bit-identical
    /// across host platforms.
    ///
    /// `energy == 0` produces an empty world (no cell at the origin), since
    /// a cell with zero energy does not exist by the world invariant.
    #[must_use]
    pub fn big_bang(world_seed: u64, energy: u32) -> Self {
        Self::big_bang_with(world_seed, energy, Base::Noise, &[])
    }

    /// Big bang whose origin cell is seeded by the procedural genesis
    /// generator ([`crate::genesis`]) instead of raw noise: the whole
    /// memory is a seed-driven weighted stream of macros. This is the
    /// information-bearing default for new worlds — see
    /// `docs/genesis-plan.md`.
    #[must_use]
    pub fn big_bang_macros(world_seed: u64, energy: u32) -> Self {
        Self::big_bang_with(world_seed, energy, Base::Macros, &[])
    }

    /// Big bang with a programmer-supplied prefix written into the origin
    /// cell's memory. The first `min(program.len(), energy)` slots are
    /// taken verbatim from `program`; the remaining slots are filled from
    /// the per-cell `xorshift32` noise stream.
    ///
    /// The RNG is **not** advanced for slots covered by the program, so
    /// for a fixed seed and program length the tail is identical
    /// regardless of program content — the prefix replaces, it doesn't
    /// consume entropy. `program.len() > energy` truncates. Equivalent to
    /// [`big_bang_with`](Self::big_bang_with) with [`Base::Noise`].
    #[must_use]
    pub fn big_bang_with_program(world_seed: u64, energy: u32, program: &[u32]) -> Self {
        Self::big_bang_with(world_seed, energy, Base::Noise, program)
    }

    /// Build a big-bang world from an explicit `(base, overlay)` genesis
    /// (see `docs/genesis-plan.md`, A6). The whole memory is filled by
    /// `base`; then `overlay` (the player's program, if any) is written
    /// verbatim over `[0, min(overlay.len(), energy))`, leaving the rest
    /// of the base fill untouched.
    ///
    /// With [`Base::Noise`] + `overlay` this reproduces
    /// [`big_bang_with_program`](Self::big_bang_with_program); with
    /// [`Base::Macros`] the tail beyond the overlay is generated program
    /// rather than noise, so passively emitted slices carry real code.
    ///
    /// The macro generator uses [`GenesisConfig::default`]; callers that
    /// need custom knobs build memory via
    /// [`crate::genesis::generate_into`] and
    /// [`insert_with_memory`](Self::insert_with_memory).
    #[must_use]
    pub fn big_bang_with(world_seed: u64, energy: u32, base: Base, overlay: &[u32]) -> Self {
        // Pre-allocate the arena to exactly the world's energy — by
        // conservation, total `mem_len` never exceeds `energy`, so the
        // arena never has to grow during `step`.
        let mut world = Self::with_capacity(world_seed, energy);
        if energy == 0 {
            return world;
        }

        // `origin_tag = cell_seed(seed, ORIGIN)` is the seed value
        // itself (not the first draw); it seeds both the noise/genesis
        // tape and the cell's identity tag.
        let origin_tag = cell_seed(world_seed, Coord::ORIGIN);
        let energy_usize = energy as usize;
        let n_overlay = overlay.len().min(energy_usize);
        let mem_start = world.arena.alloc(energy);
        {
            let slice = world.arena.slice_mut(mem_start, energy);
            match base {
                // Prefix verbatim, fresh RNG fills the tail. The RNG is
                // not advanced over the prefix — same length → same tail
                // regardless of prefix content (legacy contract).
                Base::Noise => {
                    slice[..n_overlay].copy_from_slice(&overlay[..n_overlay]);
                    let mut noise_rng = Rng::new(origin_tag);
                    for slot in slice.iter_mut().skip(n_overlay) {
                        *slot = noise_rng.next_u32();
                    }
                }
                // Generate the whole memory from the seed tape, then
                // overlay the prefix. The tail `[n_overlay, energy)` is
                // generated independently of the overlay, so it stays
                // identical regardless of prefix ("comparable background").
                Base::Macros => {
                    let mut tape = Rng::new(origin_tag);
                    generate_into(slice, &mut tape, &GenesisConfig::default());
                    slice[..n_overlay].copy_from_slice(&overlay[..n_overlay]);
                }
            }
        }

        let mut cell = Cell::new();
        cell.mem_start = mem_start;
        cell.mem_len = energy;
        cell.origin_tag = origin_tag;
        world.cells.insert(Coord::ORIGIN, cell);
        // Eager init — by-passing the public `insert` here means we
        // own the index seeding too, otherwise the first `sorted_iter`
        // before any tick would trip `debug_assert!(!sorted_dirty)`.
        world.sorted_cache.push(Coord::ORIGIN);
        world.bbox_cache = Some((0, 0, 0, 0, 0, 0));
        world
    }

    /// Build a cell with the given memory slots, allocating their
    /// storage in *this world's* arena. The returned cell is owned
    /// by the caller — usually mutated (e.g. setting `pc`,
    /// `origin_tag`) and then passed to
    /// [`SparseWorld::insert`].
    ///
    /// Convenience for tests and external callers that want to
    /// stage a cell with custom field overrides before inserting.
    /// The hot insert path is
    /// [`SparseWorld::insert_with_memory`].
    #[must_use]
    pub fn alloc_cell(&mut self, slots: &[u32]) -> Cell {
        Cell::with_memory(&mut self.arena, slots)
    }

    /// Build a cell with the given memory slots and insert it at
    /// `coord` in one shot. Equivalent to
    /// `self.insert(coord, self.alloc_cell(slots))` but avoids the
    /// intermediate move.
    pub fn insert_with_memory(&mut self, coord: Coord, slots: &[u32]) -> Option<Cell> {
        let cell = Cell::with_memory(&mut self.arena, slots);
        self.insert(coord, cell)
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
        let bbox_cache = self.bbox_cache;
        let (was_vacant, cell) = self
            .cells
            .get_or_insert_with(coord, || fresh_cell(world_seed, coord));
        if was_vacant {
            // Keyset grew → sorted_cache is now stale. Bbox is
            // extended in place; only removals need a lazy
            // rebuild, so we don't touch `bbox_dirty` here.
            self.sorted_dirty = true;
            self.bbox_cache = Some(extend_bbox(bbox_cache, coord));
        }
        cell
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

    /// Borrow the cell's memory slice at `coord`, if the cell
    /// exists. The slice borrows from the world's arena and is
    /// only valid for the duration of the `&self` borrow.
    ///
    /// Cross-crate callers (`aenternis-wasm`,
    /// `aenternis-server`) should use this instead of touching
    /// `Cell::memory` directly, since the arena module itself is
    /// `pub(crate)` and not part of the public surface.
    #[must_use]
    pub fn cell_memory(&self, coord: Coord) -> Option<&[u32]> {
        self.cells.get(&coord).map(|c| c.memory(&self.arena))
    }

    /// Read-only borrow of the world's memory arena. Pair with
    /// the `&Cell` returned by [`SparseWorld::get`] /
    /// [`SparseWorld::iter`] to call `cell.memory(arena)` —
    /// useful in tests and external readers that want both the
    /// metadata and the slot data in independent variables.
    #[must_use]
    pub const fn arena(&self) -> &Arena {
        &self.arena
    }

    /// Mutable borrow of the world's memory arena. Used by tests
    /// that build a cell via [`SparseWorld::alloc_cell`], mutate
    /// individual slots (e.g. `cell.set_memory_slot(world.arena_mut(),
    /// i, v)`), then call [`SparseWorld::insert`].
    ///
    /// Production code does not call this — the tick phases run
    /// inside the crate and already have split-borrow access to
    /// the arena. Exposed publicly because the only alternative
    /// (a per-slot helper on `SparseWorld`) is awkward for the
    /// orphan-cell-build-then-insert pattern that tests rely on.
    pub const fn arena_mut(&mut self) -> &mut Arena {
        &mut self.arena
    }

    /// Mutably borrow the cell at `coord`, if any.
    pub fn get_mut(&mut self, coord: Coord) -> Option<&mut Cell> {
        self.cells.get_mut(&coord)
    }

    /// Iterator over `(coord, energy)` pairs of all live cells, used
    /// by [`total_energy`](Self::total_energy) (in slot order).
    fn values(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.cells.iter().map(|(_, c)| c)
    }

    /// Insert (or replace) a cell at `coord`. Returns the previous
    /// cell if one existed there, mirroring `BTreeMap::insert`.
    ///
    /// **Arena cleanup on replacement.** If a cell is replaced, its
    /// memory range is freed back to the arena before the previous
    /// metadata is returned. The returned `Cell` therefore has
    /// `mem_len = 0` / `mem_start = 0` regardless of what it was
    /// before — its slot data is no longer addressable through this
    /// world's arena. Callers that need the content must copy it
    /// out (via [`Cell::memory`]) before calling `insert`.
    pub fn insert(&mut self, coord: Coord, cell: Cell) -> Option<Cell> {
        let prev = self.cells.insert(coord, cell);
        if let Some(mut prev_cell) = prev {
            prev_cell.free_memory(&mut self.arena);
            Some(prev_cell)
        } else {
            self.sorted_dirty = true;
            self.bbox_cache = Some(extend_bbox(self.bbox_cache, coord));
            None
        }
    }

    /// Remove the cell at `coord`, returning it if it was there.
    /// The removed cell's memory range is freed back to the arena
    /// before return — same contract as [`SparseWorld::insert`]
    /// when replacing.
    pub fn remove(&mut self, coord: Coord) -> Option<Cell> {
        let removed = self.cells.remove(&coord);
        if let Some(mut cell) = removed {
            cell.free_memory(&mut self.arena);
            // Keyset shrank — sorted_cache is stale, and the bbox
            // might need to contract on this axis (we don't know
            // without a full pass; the rebuild handles that lazily).
            self.sorted_dirty = true;
            self.bbox_dirty = true;
            Some(cell)
        } else {
            None
        }
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
        self.values().map(|c| u64::from(c.energy())).sum()
    }

    /// Bounding box across all live cells, as
    /// `(x_min, x_max, y_min, y_max, z_min, z_max)`. Returns `None` when
    /// the world is empty.
    ///
    /// `O(1)` — reads a side-table maintained by
    /// [`Self::rebuild_indices_if_dirty`] (which the tick loop runs
    /// after `gc_empty`). Callers that mutate outside the tick loop
    /// must call `rebuild_indices_if_dirty` before reading; the
    /// `debug_assert!` here flags forgotten rebuilds in test builds.
    #[must_use]
    pub fn bounding_box(&self) -> Option<(i32, i32, i32, i32, i32, i32)> {
        debug_assert!(
            !self.bbox_dirty,
            "bounding_box read while bbox cache is dirty — call rebuild_indices_if_dirty first"
        );
        self.bbox_cache
    }

    /// Drop every cell whose memory is empty (energy == 0). This is
    /// the garbage-collection step in the per-tick cycle described
    /// in `docs/mechanics.md` — the sparse-world counterpart of
    /// "the cell stops existing once it holds no energy."
    ///
    /// Empty cells have `mem_len == 0` by invariant (the apply
    /// phase frees memory back to the arena on shrink-to-zero), so
    /// no arena housekeeping is needed here — only the metadata
    /// `Cell` records get dropped.
    pub fn gc_empty(&mut self) {
        let len_before = self.cells.len();
        self.cells.retain(|_, cell| !cell.is_empty());
        if self.cells.len() != len_before {
            self.sorted_dirty = true;
            self.bbox_dirty = true;
        }
    }

    /// Bring the sorted index and bbox cache up to date if any mutation
    /// since the last rebuild marked them stale. Called by
    /// [`crate::tick::step`] / [`crate::tick::step_diffusion`] after
    /// `gc_empty` so the snapshot path can read both fields without
    /// triggering work per call.
    ///
    /// Idempotent: a second call with no intervening mutation is a
    /// pair of cheap flag checks. Callers that mutate outside the
    /// tick loop (manual `insert` / `remove` / `get_or_alloc`) and
    /// then want to read [`Self::sorted_iter`] or [`Self::bounding_box`]
    /// must call this themselves first — the read paths
    /// `debug_assert!` on the flags.
    pub fn rebuild_indices_if_dirty(&mut self) {
        if self.sorted_dirty {
            self.sorted_cache.clear();
            self.sorted_cache.reserve(self.cells.len());
            self.sorted_cache.extend(self.cells.keys().copied());
            self.sorted_cache.sort_unstable();
            self.sorted_dirty = false;
        }
        if self.bbox_dirty {
            // Full fold — only fires when at least one removal
            // happened. Reads from `sorted_cache` (which we just
            // brought up to date if it was dirty) for cache-friendly
            // sequential access.
            self.bbox_cache = self
                .sorted_cache
                .iter()
                .fold(None, |acc, c| Some(extend_bbox(acc, *c)));
            self.bbox_dirty = false;
        }
    }

    /// Iterate over `(coord, cell)` pairs in hash order
    /// (deterministic per run, not lex). For canonical lex order,
    /// use [`Self::sorted_iter`].
    pub fn iter(&self) -> impl Iterator<Item = (&Coord, &Cell)> + '_ {
        self.cells.iter()
    }

    /// Mutably iterate over `(coord, cell)` pairs. Walks in slot
    /// order (different from immutable `iter`) — the order is
    /// stable per run but not the same as `iter`'s. Closures that
    /// read only their own cell + a read-only neighbor snapshot are
    /// order-independent, so this is safe for the per-tick walks
    /// in [`crate::tick`].
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Coord, &mut Cell)> + '_ {
        self.cells.iter_mut()
    }

    /// Iterate over cell coordinates in hash order.
    pub fn coords(&self) -> impl Iterator<Item = &Coord> + '_ {
        self.cells.keys()
    }

    /// Iterate over `(coord, cell)` pairs in `(x, y, z)` lex order.
    ///
    /// Reads a side-table maintained by
    /// [`Self::rebuild_indices_if_dirty`]; the per-call cost is one
    /// `Vec` walk plus per-element `HashMap::get`, no sort or
    /// allocation. The tick loop refreshes the side-table after
    /// `gc_empty`, so snapshot callers (`cellsSnapshot`,
    /// `build_snapshot_payload`) read straight through.
    ///
    /// Callers that mutate outside the tick loop must call
    /// `rebuild_indices_if_dirty` before iterating; the
    /// `debug_assert!` here flags forgotten rebuilds in test builds.
    ///
    /// # Panics
    ///
    /// `expect`s that every coord in `sorted_cache` also exists in
    /// `cells`. The cache is private and only updated through the
    /// `insert`/`remove`/`get_or_alloc`/`gc_empty` mutators (which
    /// keep the invariant) and `rebuild_indices_if_dirty` (which
    /// reseeds from `cells.keys()`), so this is unreachable unless
    /// the struct's internal invariants are broken — in which case
    /// the panic is a louder signal than silently returning a
    /// half-empty iterator.
    pub fn sorted_iter(&self) -> impl Iterator<Item = (&Coord, &Cell)> + '_ {
        debug_assert!(
            !self.sorted_dirty,
            "sorted_iter read while sorted cache is dirty — call rebuild_indices_if_dirty first"
        );
        debug_assert_eq!(
            self.sorted_cache.len(),
            self.cells.len(),
            "sorted_cache and cells must agree on size when cache is clean"
        );
        self.sorted_cache.iter().map(move |c| {
            let cell = self
                .cells
                .get(c)
                .expect("sorted_cache invariant: every cached coord exists in cells");
            (c, cell)
        })
    }

    /// Diagnostic snapshot of every container's allocated size on the
    /// world. Reports `len` / `capacity` for `Vec`s and `capacity()`
    /// for `FxHashMap`s; for the nested scratch maps (`Coord ->
    /// [Vec<u32>; 6]` and `Coord -> Vec<InflowEntry>`) also reports
    /// the sum of inner `Vec` capacities. Used by the WASM diagnostic
    /// path to track which container is growing ahead of cell count
    /// before an OOM trap.
    ///
    /// `O(n)` over the cells in the nested scratch maps because of the
    /// inner-`Vec` capacity sum. Cheap enough to call once every few
    /// dozen ticks for diagnostics; do not put on the hot per-tick path.
    #[must_use]
    pub fn memory_report(&self) -> MemoryReport {
        let cells = self.cells.memory_report();
        let scratch_outflow_inner_vec_cap_sum: usize = self
            .scratch_outflow
            .values()
            .map(|per_dir| per_dir.iter().map(Vec::capacity).sum::<usize>())
            .sum();
        let scratch_inflows_inner_vec_cap_sum: usize = self
            .scratch_inflows_by_target
            .values()
            .map(Vec::capacity)
            .sum::<usize>()
            + self
                .scratch_inflows_fast
                .values()
                .map(Vec::capacity)
                .sum::<usize>();
        MemoryReport {
            tick: self.tick,
            cell_count: self.cells.len(),
            cells_slots_len: cells.slots_len,
            cells_slots_cap: cells.slots_cap,
            cells_free_slots_len: cells.free_slots_len,
            cells_free_slots_cap: cells.free_slots_cap,
            cells_coord_to_slot_cap: cells.coord_to_slot_cap,
            scratch_neighbor_energies_cap: self.scratch_neighbor_energies.capacity(),
            scratch_outflow_cap: self.scratch_outflow.capacity(),
            scratch_outflow_inner_vec_cap_sum,
            scratch_inflows_by_target_cap: self.scratch_inflows_by_target.capacity()
                + self.scratch_inflows_fast.capacity(),
            scratch_inflows_inner_vec_cap_sum,
            scratch_per_source_total_outflow_cap: self.scratch_per_source_total_outflow.capacity(),
            sorted_cache_len: self.sorted_cache.len(),
            sorted_cache_cap: self.sorted_cache.capacity(),
            arena_capacity: self.arena.capacity() as usize,
            arena_slots_vec_cap: self.arena.slots_vec_capacity(),
            arena_next_capacity: self.arena_next.capacity() as usize,
            arena_next_slots_vec_cap: self.arena_next.slots_vec_capacity(),
        }
    }
}

/// Diagnostic snapshot of every container on a [`SparseWorld`].
/// Returned by [`SparseWorld::memory_report`]. All counts are raw
/// element/entry counts — multiply by element size to estimate bytes.
#[derive(Debug, Clone, Copy)]
pub struct MemoryReport {
    /// Current `world.tick` value.
    pub tick: u64,
    /// Number of live cells (= `cells.len()`).
    pub cell_count: usize,
    /// `cells.slots.len()` — including recyclable `None` slots.
    pub cells_slots_len: usize,
    /// `cells.slots.capacity()` — backing `Vec`'s allocation.
    pub cells_slots_cap: usize,
    /// `cells.free_slots.len()` — slots awaiting reuse on next insert.
    pub cells_free_slots_len: usize,
    /// `cells.free_slots.capacity()` — backing `Vec`'s allocation.
    pub cells_free_slots_cap: usize,
    /// `cells.coord_to_slot.capacity()` — entries the coord→slot map
    /// can hold without rehash.
    pub cells_coord_to_slot_cap: usize,
    /// `scratch_neighbor_energies.capacity()` — entry capacity, not
    /// bucket count.
    pub scratch_neighbor_energies_cap: usize,
    /// `scratch_outflow.capacity()` — entry capacity.
    pub scratch_outflow_cap: usize,
    /// Sum of `Vec<u32>::capacity()` across every inner per-direction
    /// outflow buffer in `scratch_outflow`. The hot per-tick storage
    /// the pooled map keeps alive between ticks.
    pub scratch_outflow_inner_vec_cap_sum: usize,
    /// `scratch_inflows_by_target.capacity()` — entry capacity.
    pub scratch_inflows_by_target_cap: usize,
    /// Sum of `Vec<InflowEntry>::capacity()` across every inner
    /// per-target inflow list.
    pub scratch_inflows_inner_vec_cap_sum: usize,
    /// `scratch_per_source_total_outflow.capacity()` — entry capacity.
    pub scratch_per_source_total_outflow_cap: usize,
    /// `sorted_cache.len()` — live cached coord count.
    pub sorted_cache_len: usize,
    /// `sorted_cache.capacity()` — backing `Vec<Coord>`'s allocation.
    pub sorted_cache_cap: usize,
    /// Current arena's conceptual slot capacity (= `Arena::capacity`).
    pub arena_capacity: usize,
    /// Current arena's backing `Vec<u32>::capacity()` in slots —
    /// equals `arena_capacity` in steady state, may be larger right
    /// after a grow due to `Vec`'s amortized doubling.
    pub arena_slots_vec_cap: usize,
    /// Staging arena's conceptual slot capacity.
    pub arena_next_capacity: usize,
    /// Staging arena's backing `Vec<u32>::capacity()` in slots.
    pub arena_next_slots_vec_cap: usize,
}

/// Build an empty cell whose `origin_tag` is deterministic in
/// `(world_seed, coord)`. Used by [`SparseWorld::get_or_alloc`] for the
/// alloc-on-write path during inflow.
///
/// `origin_tag` is `cell_seed(world_seed, coord)` — the seed value
/// itself, *not* the first draw from a freshly-seeded RNG.
const fn fresh_cell(world_seed: u64, coord: Coord) -> Cell {
    let origin_tag = cell_seed(world_seed, coord);
    let mut cell = Cell::new();
    cell.origin_tag = origin_tag;
    cell
}

/// Stretch a bbox to also include `coord`. The bbox tuple layout is
/// `(x_min, x_max, y_min, y_max, z_min, z_max)`; `None` becomes the
/// single-point bbox at `coord`.
///
/// Delegating to `i32::min` / `i32::max` keeps the per-axis ordering
/// logic out of mutable inline `<` / `>` operators — any mutation
/// inside this function lands inside the stdlib, which has its own
/// tests; the only thing left here for mutants to flip is which axis
/// each min/max applies to, and the axis-specific bbox tests cover
/// that.
#[must_use]
pub(crate) fn extend_bbox(
    bbox: Option<(i32, i32, i32, i32, i32, i32)>,
    coord: Coord,
) -> (i32, i32, i32, i32, i32, i32) {
    match bbox {
        None => (coord.x, coord.x, coord.y, coord.y, coord.z, coord.z),
        Some((x_min, x_max, y_min, y_max, z_min, z_max)) => (
            x_min.min(coord.x),
            x_max.max(coord.x),
            y_min.min(coord.y),
            y_max.max(coord.y),
            z_min.min(coord.z),
            z_max.max(coord.z),
        ),
    }
}

// `IntoIterator` impls so that `for (coord, cell) in &world` and
// `for (coord, cell) in &mut world` work — clippy::iter_without_into_iter
// complains otherwise.

impl<'a> IntoIterator for &'a SparseWorld {
    type Item = (&'a Coord, &'a Cell);
    type IntoIter = crate::world::cells::Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl<'a> IntoIterator for &'a mut SparseWorld {
    type Item = (&'a Coord, &'a mut Cell);
    type IntoIter = crate::world::cells::IterMut<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter_mut()
    }
}

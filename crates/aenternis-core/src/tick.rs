//! Tick orchestration — the per-cell update cycle that turns a world at
//! tick N into the same world at tick N+1.
//!
//! The cycle has six logical phases (see `docs/mechanics.md`):
//!
//! 1. CPU — every cell executes `floor(energy / K)` instructions
//! 2. Sub-tick reflow — pointer layout reacts to `combined_rate`
//! 3. Outflow — `combined_rate[d]` slots copied into each neighbor
//! 4. Inflow — slots from neighbors appended to the end of memory
//! 5. Reset transient state (active outflow, override flags)
//! 6. Layout for next tick (using fresh natural rates)
//!
//! Phases 1–5 run inside a single `step` once it lands. **Today, only the
//! "compute natural rates" half of phase 6 is implemented.** Outflow,
//! inflow, alloc-on-write, and the CPU phase are wired in subsequent
//! commits — see `docs/plan.md` for the roadmap.
//!
//! ## Borrow-checker pattern
//!
//! Updating cells while looking up neighbors is the central challenge.
//! The pattern used throughout this module:
//!
//! 1. **Snapshot** the read-only data the loop will need (here: every
//!    cell's energy keyed by coordinate).
//! 2. Pull any `Copy` fields off the world into locals so they don't
//!    hold a shared borrow during the mutable phase.
//! 3. **Mutate** in a single pass over `world.cells.iter_mut()`,
//!    consulting the snapshot for neighbor reads.
//!
//! That's a `O(N)` extra read pass per tick, but it keeps the borrow
//! checker happy without `RefCell` or unsafe.

use rustc_hash::FxHashMap;

use crate::apportion::{apportion_with_shuffle, COMBINED_CLAMPED_RNG_DOMAIN};
use crate::cell::proportional_clamp;
use crate::parallel::par_or_seq_iter_mut;
use crate::{Cell, Coord, Direction, Rng, SparseWorld};

// Rayon prelude is pulled in at module scope on every target where the
// `par_or_seq_iter_mut!` macro can route through `par_iter_mut` — native
// unconditionally, plus wasm32 with the `wasm-threads` feature (via
// `wasm-bindgen-rayon`). The default wasm32 build skips this import; the
// macro on that target lowers to a plain `iter_mut` loop.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
use rayon::prelude::*;

/// Compute natural per-direction rates for every cell in the world.
///
/// For each cell `C` and each direction `d`:
///
/// ```text
/// rate[d] = stochastic_floor((C.energy - neighbor[d].energy) * coeff)
///                when C.energy > neighbor[d].energy
///         = 0     otherwise
/// ```
///
/// `neighbor[d].energy` is `0` for void neighbors — the natural "empty
/// space" value, which makes a cell on the world boundary emit outward
/// just like a cell with a low-energy neighbor.
///
/// **Determinism:** rates depend only on `(world_seed, tick, coord, d)`,
/// never on iteration order or cell allocation history. Per-cell-per-tick
/// RNG is built fresh from those four values inside the loop.
///
/// **Conservation:** if the sum of rates would exceed the cell's energy,
/// rates are clamped proportionally so total outflow ≤ memory size. This
/// preserves the energy invariant — a cell never emits more than it has.
///
/// Empty cells (`energy == 0`) get all-zero rates and are otherwise left
/// alone. They normally would have been removed by [`SparseWorld::gc_empty`]
/// before this point, but the function is tolerant if they're still around.
pub fn compute_natural_rates(world: &mut SparseWorld, coeff: f64) {
    refresh_neighbor_energies(world);

    // Pull Copy fields off the world so the per-cell closure below
    // doesn't hold a shared borrow during the (parallel) mutable phase.
    let world_seed = world.world_seed;
    // Convention: the layout for step #N is computed at the end of
    // step #(N-1), *before* `world.tick` is incremented — so the
    // rng_tick used here is one behind `world.tick`. Frozen: bumping
    // the off-by-one requires regenerating every RNG-derived baseline
    // (see `tests/apply_outflow_bit_parity.rs`).
    let rng_tick = world.tick.saturating_sub(1);
    let snapshot = &world.scratch_neighbor_energies;

    // Per-cell work: reads `snapshot` and `world_seed` / `rng_tick` /
    // `coeff` only via shared captures, writes only into its own
    // `cell.rates`. Each cell's RNG is freshly seeded from
    // `(world_seed, rng_tick, coord)` — order of execution doesn't
    // affect the result, so this is safe to run in parallel without
    // breaking the per-cell determinism contract.
    let body = |coord: &Coord, cell: &mut Cell| {
        let my_energy = cell.energy();
        if my_energy == 0 {
            cell.rates = [0; Direction::COUNT];
            return;
        }
        let neighbor_energies = snapshot
            .get(coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);
        let mut rng = Rng::for_cell_at_tick(world_seed, rng_tick, *coord, 0);
        for &d in &Direction::ALL {
            let neighbor_energy = neighbor_energies[d.index()];
            let rate = if my_energy > neighbor_energy {
                let delta = my_energy - neighbor_energy;
                // `coeff` is `f64`, so JS `Number(0.15)` flows through
                // unchanged — no `f32(0.15)→f64` rounding artifact.
                rng.stochastic_floor(f64::from(delta) * coeff)
            } else {
                0
            };
            cell.rates[d.index()] = rate;
        }
        if cell.total_rate() > my_energy {
            proportional_clamp(&mut cell.rates, my_energy, world_seed, rng_tick, *coord);
        }
    };

    par_or_seq_iter_mut!(&mut world.cells, body);
}

/// Rebuild [`SparseWorld::scratch_neighbor_energies`] from the current
/// world state. The snapshot is a `Coord → [u32; 6]` map keyed by the
/// cell's own coordinate, where each slot holds the energy of that
/// neighbor (0 for void).
///
/// Reused across the `compute_natural_rates` and `cpu_phase` phases of
/// a single [`step`] call — both phases observe the same energies (cell
/// `memory.len()` doesn't change inside the CPU phase), so building
/// once saves one full snapshot pass per tick.
///
/// **Allocation:** the snapshot's keyset is sync'd to `world.cells` in
/// place — stale coords dropped, missing ones inserted with all-zero
/// arrays — so the backing storage carries over from previous ticks
/// without per-tick allocator churn.
///
/// **Parallelism:** the value-fill pass is dispatched through
/// [`crate::parallel::par_or_seq_iter_mut!`], which goes parallel via
/// rayon on native targets once the map size crosses the helper's
/// threshold and stays sequential otherwise. Each entry's six neighbor
/// lookups read the immutable `world.cells` map by coord, so the
/// parallel walk is bit-identical to a sequential one.
fn refresh_neighbor_energies(world: &mut SparseWorld) {
    let cells = &world.cells;
    let snapshot = &mut world.scratch_neighbor_energies;
    // Keyset sync: drop stale, insert empty slots for new coords.
    snapshot.retain(|coord, _| cells.contains_key(coord));
    snapshot.reserve(cells.len().saturating_sub(snapshot.len()));
    for coord in cells.keys() {
        snapshot.entry(*coord).or_default();
    }

    let body = |coord: &Coord, energies: &mut [u32; Direction::COUNT]| {
        for &d in &Direction::ALL {
            let neighbor_coord = coord.neighbor(d);
            energies[d.index()] = cells.get(&neighbor_coord).map_or(0, Cell::energy);
        }
    };

    par_or_seq_iter_mut!(snapshot, body);
}

/// Per-cell, per-direction slot copies collected from the outflow phase.
///
/// `outflow[&coord][d.index()]` is the `Vec<u32>` of slots that the cell
/// at `coord` will emit through face `d`. The vector length matches
/// `cell.rates[d.index()]`. Cells with all-zero rates still appear in the
/// map but with empty vectors.
///
/// The outflow map is **read-only**: it captures what *would* leave each
/// cell next, without modifying the source cells. Applying the outflow
/// (shrinking source memory + appending into neighbors) is the inflow
/// phase, scheduled for the next iteration.
pub type Outflow = FxHashMap<Coord, [Vec<u32>; Direction::COUNT]>;

/// Collect the outflow snapshot for every cell in `world`.
///
/// For each cell `C` and each direction `d`:
///
/// - `combined = C.rates[d] + C.active_outflow[d]`
/// - the per-cell six-element `combined` vector is then **proportionally
///   clamped** so its sum does not exceed `C.energy()` (= memory size)
/// - `ptr = C.pointers[d]`
/// - the slice `C.memory[ptr .. ptr + combined[d]]` (modular wrap on
///   memory length) is copied into `outflow[&C.coord][d.index()]`.
///
/// Why combined-and-clamped, not just `rates`: a cell that ran `port`
/// during the CPU phase has accumulated `active_outflow` in some
/// direction(s); per the mechanics spec the actual per-tick emission
/// size in that direction is `rates + active_outflow`,
/// scaled down proportionally if their sum exceeds memory. Without this
/// clamp here, `port` would only shift *which* slots get emitted (via
/// pointer layout) and never grow the *amount*, which silently breaks
/// the `port` instruction's whole purpose.
///
/// **Determinism:** iterates the world in `FxHashMap` hash order. Output
/// is keyed by `Coord` and independent of iteration order — same per-cell
/// inputs always produce the same per-direction slots.
///
/// **Allocation:** allocates the `Outflow` map plus one `Vec<u32>` per
/// emitting direction per cell. For long-running simulations (where
/// the world has hundreds of thousands of cells), prefer
/// [`collect_outflow_into`] — it reuses an externally owned `Outflow`
/// across ticks so the `Vec` capacities stay allocated. [`step`] uses
/// that path internally via [`SparseWorld::scratch_outflow`].
#[must_use]
pub fn collect_outflow(world: &SparseWorld) -> Outflow {
    let mut outflow = Outflow::default();
    collect_outflow_into(world, &mut outflow);
    outflow
}

/// Fill an externally owned [`Outflow`] from the current world, reusing
/// its existing per-direction `Vec` capacities across ticks.
///
/// Semantics match [`collect_outflow`]: same per-cell, per-direction
/// slot extraction, same combined-and-clamped emission size. The only
/// difference is allocation behaviour — entries are `clear()`ed and
/// refilled in place so a steady-state world doesn't hit the allocator
/// for the hot per-tick `Vec<u32>` storage.
///
/// **Keyset sync:** stale coords (no longer in `world.cells`) are
/// dropped, missing coords are inserted with default-constructed
/// (non-allocating) per-direction arrays before the parallel fill.
///
/// **Determinism:** per-entry work is independent and depends only on
/// the cell's own state, so parallel and sequential paths produce
/// identical results.
pub fn collect_outflow_into(world: &SparseWorld, outflow: &mut Outflow) {
    // Sync keyset: drop entries for coords that no longer exist, then
    // insert empty slots for new ones. Default-constructing
    // `[Vec<u32>; 6]` is non-allocating (`Vec::new()` per slot).
    outflow.retain(|coord, _| world.cells.contains_key(coord));
    outflow.reserve(world.cells.len().saturating_sub(outflow.len()));
    for coord in world.cells.keys() {
        outflow.entry(*coord).or_default();
    }

    // Per-entry work: clear the per-direction buffers (keeps capacity),
    // then refill from the cell's memory. Each entry's cell is read by
    // coord lookup against the immutable `world.cells`, so per-entry
    // tasks are fully independent.
    let cells = &world.cells;
    let arena = &world.arena;
    let world_seed = world.world_seed;
    let rng_tick = world.tick;
    let body = |coord: &Coord, per_dir: &mut [Vec<u32>; Direction::COUNT]| {
        for v in per_dir.iter_mut() {
            v.clear();
        }
        let Some(cell) = cells.get(coord) else {
            return;
        };
        let mem_size = cell.memory_len();
        if mem_size == 0 {
            return;
        }
        let mem_size_u32 = u32::try_from(mem_size).unwrap_or(u32::MAX);
        let combined = combined_clamped(
            &cell.rates,
            &cell.active_outflow,
            mem_size_u32,
            world_seed,
            rng_tick,
            *coord,
        );
        let cell_memory = cell.memory(arena);
        for &d in &Direction::ALL {
            let rate = combined[d.index()] as usize;
            let ptr = cell.pointers[d.index()] as usize;
            let buf = &mut per_dir[d.index()];
            buf.reserve(rate);
            debug_assert!(
                rate <= mem_size,
                "rate must be clamped to mem_size by combined_clamped"
            );
            let end = ptr.saturating_add(rate);
            if end <= mem_size {
                buf.extend_from_slice(&cell_memory[ptr..end]);
            } else {
                let tail = mem_size - ptr;
                buf.extend_from_slice(&cell_memory[ptr..mem_size]);
                let wrap = rate - tail;
                buf.extend_from_slice(&cell_memory[..wrap]);
            }
        }
    };

    par_or_seq_iter_mut!(outflow, body);
}

/// Lay out per-direction pointers for every cell in the world.
///
/// Uses each cell's combined rate (`rates + active_outflow`) as the
/// per-direction consumption budget, **clamped** to the cell's memory
/// size so that total emission never exceeds what the cell actually
/// holds. Honors any `pointer_override` flags set by a CPU-phase
/// `setp` / `setpv` instruction this tick.
///
/// This is the sub-tick reflow step from `docs/mechanics.md`. See
/// [`combined_clamped`] for the per-direction `u64` accumulation that
/// safely sums `rates + active_outflow` without overflowing `u32` when
/// the two are near the type's max.
pub fn lay_out_pointers(world: &mut SparseWorld) {
    // Per-cell pointer layout has no inter-cell dependencies — each
    // cell only reads its own rates / active_outflow / memory size.
    // Parallelizing is bit-identical to the sequential walk.
    let world_seed = world.world_seed;
    let rng_tick = world.tick;
    let body = |coord: &Coord, cell: &mut Cell| {
        let mem_size = cell.memory_len();
        if mem_size == 0 {
            return;
        }
        let mem_size_u32 = u32::try_from(mem_size).unwrap_or(u32::MAX);
        let combined = combined_clamped(
            &cell.rates,
            &cell.active_outflow,
            mem_size_u32,
            world_seed,
            rng_tick,
            *coord,
        );
        cell.lay_out_pointers(&combined);
    };

    par_or_seq_iter_mut!(&mut world.cells, body);
}

/// Compute clamped combined per-direction rate for one cell.
///
/// `combined = rates[d] + active_outflow[d]`, summed in `u64` so the
/// addition never saturates on `u32` overflow the way JS `Number`
/// addition wouldn't, then proportionally clamped to `cap` and
/// returned as `u32` per direction.
///
/// Centralized here because both [`lay_out_pointers`] (sub-tick reflow)
/// and [`collect_outflow`] need exactly the same clamped combined for
/// pointer layout and outflow amounts respectively, and both calls
/// must agree to the bit. Splitting the logic into two open-coded
/// blocks left a saturating-`u32`-add bug in earlier revisions where
/// the two paths could disagree on the clamp output by ~1500 slots
/// per direction.
///
/// The algorithmic core (proportional `f64` scale + Largest-Remainder
/// leftover distribution with a Fisher-Yates tie-break) lives in
/// [`crate::apportion::apportion_with_shuffle`]; see that module for the
/// JS bit-parity argument, the statistical-isotropy contract, and the
/// `f64`-precision bounds. This wrapper only builds the `[u64; 6]`
/// input from `rates + active_outflow`.
#[must_use]
pub fn combined_clamped(
    rates: &[u32; Direction::COUNT],
    active_outflow: &[u32; Direction::COUNT],
    cap: u32,
    world_seed: u64,
    rng_tick: u64,
    coord: Coord,
) -> [u32; Direction::COUNT] {
    let combined: [u64; Direction::COUNT] =
        std::array::from_fn(|i| u64::from(rates[i]) + u64::from(active_outflow[i]));
    apportion_with_shuffle(
        &combined,
        cap,
        world_seed,
        rng_tick,
        coord,
        COMBINED_CLAMPED_RNG_DOMAIN,
    )
}

/// Apply an [`Outflow`] snapshot to the world with collision-as-soft-mixing
/// (dominance / intrusion) semantics, per `docs/mechanics.md`.
///
/// Three phases:
///
/// 1. **Shrink sources.** Every cell loses `total_outflow` slots from
///    the end of memory. After this step, `cell.energy()` equals the
///    post-burn energy used by the dominance math — so we don't need
///    to snapshot pre-step energies into a side table; the cell itself
///    *is* the snapshot.
/// 2. **Build per-target inflow lists** with dominance computed from
///    the post-shrink energies (attacker post-burn for source,
///    post-outflow for target).
/// 3. **Per-target intrusion insert.** Inflows are sorted by dominance
///    descending (tie-break by source-direction canonical order) and
///    applied one by one: each inflow is `splice`d in at `write_start
///    = memSize - intrusion_depth`, displacing the target's existing
///    memory upward. Strong attackers drive deep into the core; weak
///    ones stack on the membrane.
///
/// **Dominance:** `dominance = clamp(1 - r / move_threshold, 0, 1)`
/// where `r = target_E_post_outflow / max(attacker_E_post_burn, 1)`,
/// `attacker_E_post_burn = source.energy_pre - source.total_outflow`,
/// and `target_E_post_outflow = target.energy_pre - target.total_outflow`.
/// Void targets have both energies and total outflow at zero.
///
/// **Origin-tag inheritance:** if the highest-dominance inflow has
/// `dominance >= 0.5`, the target adopts the attacker's `origin_tag`.
/// Sub-`0.5` collisions leave the tag alone.
///
/// **PC under metempsychosis:** the target's program counter stays
/// numerically the same. If `pc_old < write_start`, the program runs
/// on. If `pc_old >= write_start`, PC now points into the attacker's
/// freshly-inserted code — body snatch. The PC is finally taken modulo
/// the new memory length so it stays in range; only happens when
/// memory shrank (impossible here — inflow only grows or holds).
///
/// **Conservation:** total slots before == total slots after.
///
/// **Allocation:** the per-target intrusion insert uses
/// [`Vec::splice`] over a pre-reserved `target.memory` buffer, so each
/// applied inflow shifts memory in place rather than allocating a
/// fresh `Vec` per inflow (the old code path's hot allocation source —
/// at ~200 k cells with ~6 inflows each, that was ~1.3 M `Vec`
/// allocations per tick).
///
/// **Parallelism:** the per-target apply phase iterates `world.cells`
/// through [`crate::parallel::par_or_seq_iter_mut!`] — rayon-parallel
/// on native targets above the helper's threshold, sequential below
/// (and always on WASM). Each cell's work depends only on its own
/// state plus the read-only `inflows_by_target` map, so the parallel
/// walk is bit-identical to a sequential one.
#[allow(clippy::too_many_lines)]
pub fn apply_outflow(world: &mut SparseWorld, outflow: &Outflow) {
    // Reset the staging arena. After this `arena_next` is one big
    // free range covering its full capacity; per-cell bump allocs
    // below carve from it sequentially, and the swap at the bottom
    // promotes it to the new `arena` (the previous `arena` becomes
    // next tick's staging buffer and is cleared then).
    world.arena_next.clear();

    // -------------------------------------------------------------------
    // Phase 1: cache per-source total outflow.
    // -------------------------------------------------------------------
    // No in-place shrink anymore — the post-outflow length is just
    // `old_mem_len - total_outflow`, computed numerically and used
    // below as the size of the cell's "Original" rope segment that
    // the merge will copy out of `arena_cur`.
    let mut per_source_total_outflow: FxHashMap<Coord, u32> = FxHashMap::default();
    per_source_total_outflow.reserve(outflow.len());
    for (coord, per_dir) in outflow {
        let total: u32 = per_dir
            .iter()
            .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX))
            .fold(0u32, u32::saturating_add);
        per_source_total_outflow.insert(*coord, total);
    }

    // -------------------------------------------------------------------
    // Phase 2: build per-target inflow lists with dominance.
    // -------------------------------------------------------------------
    //
    // The inflow map is pulled out of `world` via `mem::take` so its
    // per-target `Vec` capacities — `clear()`-reused across ticks —
    // skip the ~200 k `Vec::with_capacity(0)→reserve(N)` cycle the
    // freshly-built `FxHashMap::default()` version was paying.
    //
    // Dominance math uses *post-outflow* energies (the values the
    // pre-double-buffer code arrived at by mutating arena in phase 1);
    // we just compute them inline.
    let move_threshold = world.move_threshold.max(f32::EPSILON);
    let mut inflows_by_target = std::mem::take(&mut world.scratch_inflows_by_target);
    for v in inflows_by_target.values_mut() {
        v.clear();
    }
    inflows_by_target.reserve(outflow.len().saturating_sub(inflows_by_target.len()));

    for (source_coord, per_dir) in outflow {
        let (src_old_mem_len, src_origin_tag) = world
            .cells
            .get(source_coord)
            .map_or((0u32, 0u32), |c| (c.mem_len, c.origin_tag));
        let src_total_outflow = per_source_total_outflow
            .get(source_coord)
            .copied()
            .unwrap_or(0);
        let attacker_post = src_old_mem_len.saturating_sub(src_total_outflow);
        let attacker_post_burn = u32::max(1, attacker_post);

        for &d in &Direction::ALL {
            let slots = &per_dir[d.index()];
            if slots.is_empty() {
                continue;
            }
            let target_coord = source_coord.neighbor(d);
            let tgt_old_mem_len = world.cells.get(&target_coord).map_or(0, |c| c.mem_len);
            let tgt_total_outflow = per_source_total_outflow
                .get(&target_coord)
                .copied()
                .unwrap_or(0);
            let target_e_post = tgt_old_mem_len.saturating_sub(tgt_total_outflow);

            let r = u32_to_f32(target_e_post) / u32_to_f32(attacker_post_burn);
            let dom = (1.0 - r / move_threshold).clamp(0.0, 1.0);

            let dir_from_target = d.opposite().index() as u8;
            inflows_by_target
                .entry(target_coord)
                .or_default()
                .push(InflowEntry {
                    source_coord: *source_coord,
                    source_dir: d.index() as u8,
                    dominance: dom,
                    src_origin_tag,
                    dir_from_target,
                });
        }
    }

    // Sort each target's inflows (dominance desc, dir asc). Empty
    // entries (stale from previous tick) are skipped; their `Vec`
    // capacity is reused without churn.
    for entries in inflows_by_target.values_mut() {
        if entries.len() <= 1 {
            continue;
        }
        entries.sort_by(|a, b| {
            b.dominance
                .partial_cmp(&a.dominance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.dir_from_target.cmp(&b.dir_from_target))
        });
    }

    // -------------------------------------------------------------------
    // Phase 3: alloc-on-write metadata for new targets.
    // -------------------------------------------------------------------
    // The actual arena ranges are bump-allocated in `arena_next`
    // during the write phase; here we only insert the `Cell`
    // metadata (with the deterministic `origin_tag`) so the write
    // loop below sees every receiver as a regular `cells.iter_mut`
    // bucket.
    let world_seed = world.world_seed;
    for (target_coord, entries) in &inflows_by_target {
        if entries.is_empty() {
            continue;
        }
        if !world.cells.contains_key(target_coord) {
            let mut cell = Cell::new();
            cell.origin_tag = crate::rng::cell_seed(world_seed, *target_coord);
            world.cells.insert(*target_coord, cell);
            world.sorted_dirty = true;
            world.bbox_cache = Some(crate::world::sparse::extend_bbox(
                world.bbox_cache,
                *target_coord,
            ));
        }
    }

    // -------------------------------------------------------------------
    // Phase 4: write each cell's new memory into `arena_next`.
    // -------------------------------------------------------------------
    // Split borrow: `cells.iter_mut` mutates cell metadata in place
    // (mem_start, mem_len, inflow, pc, origin_tag) while we read
    // arena_cur (`&world.arena`) and bump-allocate into arena_next
    // (`&mut world.arena_next`). Sequential — Phase 3 *could* go
    // parallel via disjoint prefix-sum slices in `arena_next`, but
    // the sequential path is simpler and the per-cell work is a
    // memcpy of `new_len` slots either way; parallel parallelism is
    // a follow-up if measurement shows it matters.
    {
        let cells = &mut world.cells;
        let arena_cur = &world.arena;
        let arena_next = &mut world.arena_next;
        let inflows = &inflows_by_target;

        for (coord, cell) in cells.iter_mut() {
            // Cache pre-write values; we'll overwrite mem_start /
            // mem_len below and still need the old ones to read from
            // arena_cur.
            let old_mem_start = cell.mem_start;
            let old_mem_len = cell.mem_len;
            let total_outflow = per_source_total_outflow.get(coord).copied().unwrap_or(0);
            let post_outflow_len = old_mem_len.saturating_sub(total_outflow);
            let entries = inflows.get(coord).map_or::<&[InflowEntry], _>(&[], |e| e);

            // Clear last tick's inflow tracking; the inflows below
            // will accumulate into a fresh array.
            cell.inflow = [0; Direction::COUNT];

            // Sum inflow slot counts to learn new_len up front (we
            // need to bump-alloc the destination range before
            // starting the per-segment copy).
            let mut total_inflow: u32 = 0;
            for entry in entries {
                if let Some(per_dir) = outflow.get(&entry.source_coord) {
                    let slots = &per_dir[entry.source_dir as usize];
                    total_inflow =
                        total_inflow.saturating_add(u32::try_from(slots.len()).unwrap_or(u32::MAX));
                }
            }
            let new_len = post_outflow_len.saturating_add(total_inflow);

            if new_len == 0 {
                // Cell will be gc'd. Drop its arena reference so the
                // pre-swap `cell.memory(arena_cur)` in any debug
                // tooling between here and the swap doesn't index
                // a stale range.
                cell.mem_start = 0;
                cell.mem_len = 0;
                continue;
            }

            let new_start = arena_next.alloc(new_len);

            write_cell_into_next_arena(
                cell,
                arena_cur,
                arena_next,
                new_start,
                new_len,
                old_mem_start,
                post_outflow_len,
                entries,
                outflow,
            );
        }
    }

    // -------------------------------------------------------------------
    // Phase 5: swap arenas. After this, `arena` holds the new state;
    // `arena_next` holds stale data that the next tick's `clear()`
    // will discard.
    // -------------------------------------------------------------------
    std::mem::swap(&mut world.arena, &mut world.arena_next);

    // Return the pooled inflow map to the world so its per-target
    // `Vec` capacities carry over to the next tick's apply.
    world.scratch_inflows_by_target = inflows_by_target;
}

/// Inflow entry built during phase 2 of [`apply_outflow`].
///
/// Stores the minimum needed to look up the slice of slots in the
/// source's [`Outflow`] entry on demand (no borrow), the attacker's
/// dominance against the target, and the attacker's origin tag for
/// metempsychosis.
///
/// The struct holds `(source_coord, source_dir)` rather than a `&[u32]`
/// so the containing `Vec` can live on [`SparseWorld`] across ticks
/// (no lifetime parameter, so capacity reuse via `Vec::clear` is
/// possible). The slots are recovered in phase 3 with one extra
/// `outflow.get(source_coord)` lookup per applied inflow — at ~6
/// inflows per target × ~200 k targets that's well under a millisecond
/// of cache-warm hash lookups, far less than the ~200 k `Vec`
/// allocations the pooled storage replaces.
#[derive(Debug, Clone, Copy)]
pub struct InflowEntry {
    /// Source cell that emitted this inflow.
    pub source_coord: crate::Coord,
    /// Direction in which the source emitted (`d` such that
    /// `target_coord = source_coord.neighbor(d)`).
    pub source_dir: u8,
    /// Dominance score of this inflow against the target.
    pub dominance: f32,
    /// Attacker's `origin_tag`, used for metempsychosis if dominance
    /// crosses the threshold.
    pub src_origin_tag: u32,
    /// Direction-from-target: the face the inflow appears to enter
    /// through. Equal to `Direction::from(source_dir).opposite().index()`,
    /// cached here so the sort tie-breaker and the
    /// `target.inflow[dir]` update don't need to recompute it.
    pub dir_from_target: u8,
}

/// One segment of the inflow-merge rope: either a sub-range of the
/// target's pre-existing memory, or a sub-slice of one of the entries'
/// outflow slots. See [`merge_inflows`].
#[derive(Debug, Clone, Copy)]
enum MergeSegment {
    Original {
        start: u32,
        end: u32,
    },
    Insert {
        entry_idx: u32,
        start: u32,
        end: u32,
    },
}

impl MergeSegment {
    #[inline]
    const fn len(self) -> u32 {
        match self {
            Self::Original { start, end } | Self::Insert { start, end, .. } => end - start,
        }
    }

    /// Split the segment at `offset` (0 < offset < `self.len()`).
    #[inline]
    const fn split_at(self, offset: u32) -> (Self, Self) {
        match self {
            Self::Original { start, end } => {
                let mid = start + offset;
                (
                    Self::Original { start, end: mid },
                    Self::Original { start: mid, end },
                )
            }
            Self::Insert {
                entry_idx,
                start,
                end,
            } => {
                let mid = start + offset;
                (
                    Self::Insert {
                        entry_idx,
                        start,
                        end: mid,
                    },
                    Self::Insert {
                        entry_idx,
                        start: mid,
                        end,
                    },
                )
            }
        }
    }
}

/// Worst-case rope size when merging up to 6 inflows into a target's
/// memory: 1 initial Original + 6 Insert + up to 6 Original splits = 13.
/// `16` rounds up for headroom and a power-of-two stack array.
const MERGE_ROPE_CAP: usize = 16;

/// Max inflow entries per target — one per face. Mirrors `Direction::COUNT`
/// but kept as a local constant so [`merge_inflows`]'s stack arrays can
/// be sized at compile time.
const MERGE_MAX_ENTRIES: usize = Direction::COUNT;

/// Insert `new_seg` at logical position `write_start` within the rope
/// `rope[..rope_len]`. Returns the new `rope_len`. The rope must have
/// at least 2 slots of free capacity (`rope_len + 2 <= rope.len()`),
/// since one insert can produce one split + one new segment.
///
/// Logical positions count segment lengths from index 0: a `write_start`
/// of 0 prepends; a `write_start` equal to the rope's total length
/// appends. Anything in between either lands on a segment boundary (no
/// split) or inside a segment (splits it).
///
/// **Accepted-as-unkillable mutants** (`cargo mutants 27.0.0`, verified
/// 2026-05-12):
///
/// - `<` → `<=` at the split-condition compare: boundary splits emit a
///   trailing empty `Original{end, end}` segment that flatten ignores.
/// - `+` → `*` (i.e. `+1` → `*1`) in the shift-right range: copies one
///   extra cell that the subsequent `rope[i+2] = second` immediately
///   overwrites.
/// - `> 0` → `>= 0` on the `original_len` guard at the caller (line ~753):
///   for `original_len = 0` the mutant pre-seeds an `Original{0, 0}` that
///   contributes nothing to the flatten — same output as the empty-rope
///   start.
///
/// All three change the work the function does, not the data it
/// produces — there is no observable difference for any test to detect.
fn rope_insert(
    rope: &mut [MergeSegment; MERGE_ROPE_CAP],
    rope_len: usize,
    write_start: u32,
    new_seg: MergeSegment,
) -> usize {
    let mut cum: u32 = 0;
    let mut insert_at: usize = rope_len;
    let mut split: Option<(usize, u32)> = None;

    for (i, seg) in rope[..rope_len].iter().enumerate() {
        if write_start <= cum {
            insert_at = i;
            split = None;
            break;
        }
        let seg_len = seg.len();
        if write_start < cum + seg_len {
            split = Some((i, write_start - cum));
            break;
        }
        cum += seg_len;
    }

    if let Some((i, offset)) = split {
        // Shift right by 2 to make room for [first_half, new_seg, second_half].
        for j in (i + 1..rope_len).rev() {
            rope[j + 2] = rope[j];
        }
        let (first, second) = rope[i].split_at(offset);
        rope[i] = first;
        rope[i + 1] = new_seg;
        rope[i + 2] = second;
        rope_len + 2
    } else {
        // Plain insert at `insert_at` — shift right by 1.
        for j in (insert_at..rope_len).rev() {
            rope[j + 1] = rope[j];
        }
        rope[insert_at] = new_seg;
        rope_len + 1
    }
}

/// Build the cell's new memory in `arena_next` from the post-outflow
/// remainder of its old data plus the inflow slots, preserving the
/// sequential splice semantics of the pre-double-buffer rope merge.
///
/// `entries` must be sorted by `(dominance desc, dir_from_target asc)`
/// — same contract as the splice-based predecessor. `outflow` is the
/// read-only per-source slot map. Caller has bump-allocated
/// `(new_start, new_len)` in `arena_next`; this function copies the
/// new contents in.
///
/// **Semantics, identical to the splice predecessor:**
///
/// - Rope starts with one Original segment covering
///   `[old_mem_start .. old_mem_start + post_outflow_len)` of
///   `arena_cur` (i.e. the cell's memory after the conceptual
///   end-shrink).
/// - For each entry in order: `intrusion_depth = floor(dominance *
///   current_len)`; `write_start = current_len - intrusion_depth`.
///   `current_len` grows by `slots.len()` after each insert.
/// - Origin-tag inheritance: `entries[0]` wins iff its dominance ≥ 0.5.
/// - `cell.inflow[dir_from_target]` accumulates the slot count per
///   applied entry (saturating).
/// - PC stays numerically the same; modulo'd back into range at the end.
///
/// **How it works:** small stack-allocated rope of segments tracks
/// where each insert lands. Once built, we flatten by walking the
/// rope and copying each segment directly into the destination slice
/// in `arena_next` — no thread-local scratch needed, since the
/// destination is a single contiguous `&mut [u32]` carved out of
/// `arena_next` by the caller.
///
/// **Accepted-as-unkillable mutants** (`cargo mutants 27.0.0`):
///
/// - `> 0` → `>= 0` on the `if post_outflow_len > 0` rope-seed
///   guard: with `>= 0` the seed becomes `Original { start: 0,
///   end: 0 }` for an empty source. The flatten loop's own
///   `if seg_len > 0` check (also documented below) drops it
///   without writing, so the output is identical.
/// - `> 0` → `>= 0` on the two `if seg_len > 0` guards inside
///   the flatten loop (one per segment kind): the inner copy
///   degenerates to `dest[pos..pos].copy_from_slice(&src[..0])`
///   when `seg_len == 0`, an observable no-op. The `> 0` guard
///   is a perf early-exit, not a correctness gate.
#[allow(clippy::too_many_arguments)]
fn write_cell_into_next_arena(
    cell: &mut Cell,
    arena_cur: &crate::world::arena::Arena,
    arena_next: &mut crate::world::arena::Arena,
    new_start: u32,
    new_len: u32,
    old_mem_start: u32,
    post_outflow_len: u32,
    entries: &[InflowEntry],
    outflow: &Outflow,
) {
    // Origin-tag inheritance fires on the highest-dominance entry only,
    // and only when dominance ≥ 0.5.
    if let Some(top) = entries.first() {
        if top.dominance >= 0.5 {
            cell.origin_tag = top.src_origin_tag;
        }
    }

    let post_outflow_len_usize = post_outflow_len as usize;

    let mut rope = [MergeSegment::Original { start: 0, end: 0 }; MERGE_ROPE_CAP];
    let mut rope_len: usize = if post_outflow_len > 0 {
        rope[0] = MergeSegment::Original {
            start: 0,
            end: post_outflow_len,
        };
        1
    } else {
        0
    };
    let mut current_len = post_outflow_len_usize;

    // Resolve each entry's slots once so flatten doesn't re-hash.
    let mut slot_slices: [&[u32]; MERGE_MAX_ENTRIES] = [&[]; MERGE_MAX_ENTRIES];
    let mut slot_count: usize = 0;

    for entry in entries {
        if slot_count >= MERGE_MAX_ENTRIES {
            break;
        }
        let Some(per_dir) = outflow.get(&entry.source_coord) else {
            continue;
        };
        let slots = &per_dir[entry.source_dir as usize];
        if slots.is_empty() {
            continue;
        }

        let intrusion_depth = (entry.dominance * usize_to_f32(current_len)) as usize;
        let write_start_usize = current_len.saturating_sub(intrusion_depth);

        let slots_len_u32 = u32::try_from(slots.len()).unwrap_or(u32::MAX);
        let write_start_u32 = u32::try_from(write_start_usize).unwrap_or(u32::MAX);

        let new_seg = MergeSegment::Insert {
            entry_idx: slot_count as u32,
            start: 0,
            end: slots_len_u32,
        };
        rope_len = rope_insert(&mut rope, rope_len, write_start_u32, new_seg);
        slot_slices[slot_count] = slots;
        slot_count += 1;
        current_len += slots.len();

        let dir_idx = entry.dir_from_target as usize;
        cell.inflow[dir_idx] = cell.inflow[dir_idx].saturating_add(slots_len_u32);
    }

    debug_assert_eq!(
        current_len, new_len as usize,
        "rope flatten size must match the bump-allocated new_len"
    );

    // Flatten the rope directly into the destination slice. One
    // memcpy per segment, no intermediate buffer — `arena_next` is
    // already sized to `new_len` for this cell by the caller.
    let dest = arena_next.slice_mut(new_start, new_len);
    let mut pos: usize = 0;
    for seg in &rope[..rope_len] {
        match *seg {
            MergeSegment::Original { start, end } => {
                let seg_len = (end - start) as usize;
                if seg_len > 0 {
                    let src = arena_cur.slice(old_mem_start + start, end - start);
                    dest[pos..pos + seg_len].copy_from_slice(src);
                    pos += seg_len;
                }
            }
            MergeSegment::Insert {
                entry_idx,
                start,
                end,
            } => {
                let seg_len = (end - start) as usize;
                if seg_len > 0 {
                    let slots = slot_slices[entry_idx as usize];
                    dest[pos..pos + seg_len].copy_from_slice(&slots[start as usize..end as usize]);
                    pos += seg_len;
                }
            }
        }
    }
    debug_assert_eq!(pos, new_len as usize);

    cell.mem_start = new_start;
    cell.mem_len = new_len;

    // PC wrap mirrors the pre-double-buffer rope-merge predecessor:
    // it fired *only* when at least one inflow with non-empty slots
    // was applied (`slot_count > 0`). Cells with outflow but no
    // inflow keep their pre-apply pc verbatim — `cpu_phase`'s
    // `pc_u % mem_size` lazily wraps it on the next tick. Wrapping
    // eagerly here would change the per-cell `cell.pc` snapshot the
    // bit-parity baseline hashes after every tick.
    if slot_count > 0 {
        if new_len == 0 {
            cell.pc = 0;
        } else {
            cell.pc %= new_len;
        }
    }
}

/// `u32 → f32` cast for dominance arithmetic. Cell energies stay well
/// below `2^24` in any realistic world (where `f32` is exact), so the
/// cast is lossless in practice. `clippy::cast_precision_loss` can't
/// see that constraint, so we localize the suppression. `const fn`
/// can't hold float `as` casts on MSRV 1.78 either, hence the second
/// allow.
#[allow(clippy::cast_precision_loss, clippy::missing_const_for_fn)]
#[inline]
fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

/// `usize → f32` cast for `intrusion_depth`. Same story as
/// [`u32_to_f32`]: memory sizes are bounded by the world's energy,
/// which fits comfortably under the `f32` mantissa precision.
#[allow(clippy::cast_precision_loss, clippy::missing_const_for_fn)]
#[inline]
fn usize_to_f32(v: usize) -> f32 {
    v as f32
}

/// Reset transient per-tick state on every cell: pointer overrides and
/// active outflow buffers. Called after the outflow phase to clear the
/// programmer's per-tick instructions before the next tick starts.
pub fn end_of_tick(world: &mut SparseWorld) {
    for cell in world.cells.values_mut() {
        cell.end_of_tick();
    }
}

/// Run the CPU phase: every cell executes `floor(energy / k)` instructions.
///
/// `k` is the world-wide compute constant from `docs/mechanics.md`. The
/// canonical value is 1 (compute is a conserved quantity = total energy).
/// `k = 0` is silently treated as `k = 1` to avoid a division-by-zero
/// while keeping the API monomorphic in `u32`.
///
/// **Neighbor snapshot:** before the per-cell loop runs, this function
/// captures every cell's six-direction neighbor energies into a
/// snapshot map. The per-cell budget loop then passes that snapshot to
/// [`vm::execute_instruction`] so `senergy` reads see a static field —
/// emissions and absorptions a cell makes mid-instruction don't show up
/// in another cell's sensor reads in the same tick. This matches the
/// introspection invariant from `docs/aenternis.md`.
///
/// **Determinism:** each cell's instruction budget reads only the
/// shared snapshot and writes only into its own state, so the parallel
/// walk below produces the same result as a sequential one. The
/// per-cell-per-tick RNG (if any opcode draws one) is keyed by
/// `(world_seed, tick, coord)`, not by iteration order.
pub fn cpu_phase(world: &mut SparseWorld, k: u32) {
    // The neighbor-energy snapshot is shared with `compute_natural_rates`
    // when `cpu_phase` runs inside [`step`]. If a caller invokes
    // `cpu_phase` standalone (e.g. tests), refresh it here so the read
    // below sees fresh data.
    if world.scratch_neighbor_energies.len() != world.cells.len() {
        refresh_neighbor_energies(world);
    }

    let k_safe = k.max(1);
    // Sequential walk in Phase 2 because each `execute_instruction`
    // call takes `&mut Arena`, and we can't share a mutable arena
    // borrow across rayon worker threads. Phase 3 of the arena
    // refactor re-introduces parallelism via prefix-sum disjoint
    // ranges into `arena_next`; for now we keep the cell ranges in
    // a single arena and walk them one at a time.
    let snapshot = &world.scratch_neighbor_energies;
    let cells = &mut world.cells;
    let arena = &mut world.arena;
    for (coord, cell) in cells.iter_mut() {
        let neighbors = snapshot
            .get(coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);
        let budget = cell.energy() / k_safe;
        for _ in 0..budget {
            crate::vm::execute_instruction(cell, arena, &neighbors);
        }
    }
}

/// Run one full simulation tick on the world.
///
/// Phases (see `docs/mechanics.md`):
///
/// 1. [`initialize`] — fresh natural rates and pointer layout.
/// 2. [`cpu_phase`] — per-cell `floor(energy/k)` instructions; programs
///    may override pointers via `setp`/`setpv` and accumulate active
///    outflow via `port`.
/// 3. [`lay_out_pointers`] — sub-tick reflow with combined rate
///    (`rates + active_outflow`), honoring `pointer_override` flags.
/// 4. [`collect_outflow`] / [`apply_outflow`] — emission across faces,
///    alloc-on-write into void neighbors.
/// 5. [`end_of_tick`] — reset overrides and active outflow.
/// 6. [`SparseWorld::gc_empty`] — drop cells whose memory shrank to zero.
/// 7. Increment `world.tick`.
///
/// Energy is conserved across the cycle.
pub fn step(world: &mut SparseWorld, coeff: f64, k: u32) {
    initialize(world, coeff);
    cpu_phase(world, k);
    lay_out_pointers(world);
    // `mem::take` pulls the scratch buffer out so we can pass `&mut
    // world.cells` into `apply_outflow` while still owning a populated
    // `Outflow`. Reattach at the end so capacities persist across ticks.
    let mut outflow = std::mem::take(&mut world.scratch_outflow);
    collect_outflow_into(world, &mut outflow);
    apply_outflow(world, &outflow);
    world.scratch_outflow = outflow;
    end_of_tick(world);
    world.gc_empty();
    world.rebuild_indices_if_dirty();
    world.tick = world.tick.saturating_add(1);
}

/// Bring the world into a "ready to step" state by computing natural
/// rates from current gradients and laying out pointers from those rates.
///
/// Combines [`compute_natural_rates`] and [`lay_out_pointers`] in the
/// canonical order. Useful as a one-shot setup after manual world
/// construction (insert / remove cells outside the step cycle), and as
/// the leading phase of [`step_diffusion`] itself.
pub fn initialize(world: &mut SparseWorld, coeff: f64) {
    compute_natural_rates(world, coeff);
    lay_out_pointers(world);
}

/// Run one diffusion-only tick on the world.
///
/// Order of phases (CPU and dominance are not yet wired in):
///
/// 1. [`initialize`] — fresh natural rates from current gradients,
///    pointer layout from those rates. (Once the VM lands, the CPU
///    phase runs between this step and the next, and lay-out is
///    re-run as the sub-tick reflow with combined rates.)
/// 2. [`collect_outflow`] — snapshot of slots emitted per direction.
/// 3. [`apply_outflow`] — shrink sources, append into (possibly newly
///    alloc-on-written) neighbors.
/// 4. [`end_of_tick`] — reset overrides and active outflow.
/// 5. [`SparseWorld::gc_empty`] — drop cells whose memory shrank to zero.
/// 6. Increment `world.tick`.
///
/// Energy is conserved across the cycle: every slot that leaves a
/// source ends up appended into exactly one neighbor.
pub fn step_diffusion(world: &mut SparseWorld, coeff: f64) {
    initialize(world, coeff);
    let mut outflow = std::mem::take(&mut world.scratch_outflow);
    collect_outflow_into(world, &mut outflow);
    apply_outflow(world, &outflow);
    world.scratch_outflow = outflow;
    end_of_tick(world);
    world.gc_empty();
    world.rebuild_indices_if_dirty();
    world.tick = world.tick.saturating_add(1);
}

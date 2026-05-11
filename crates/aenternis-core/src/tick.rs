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

use crate::cell::proportional_clamp;
use crate::{Cell, Coord, Direction, Rng, SparseWorld};

// Rayon's parallel iterator traits are only pulled in on native
// targets. WASM builds fall back to sequential `iter_mut()` (rayon
// needs `SharedArrayBuffer` + COOP/COEP, separate infrastructure).
#[cfg(not(target_arch = "wasm32"))]
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
    // JS prototype 9-B computes the layout for step #N at the end of
    // step #(N-1), *before* incrementing `this.tick` — so the rng_tick
    // used is N-2 (saturated at 0). We always run in 9-B parity, so
    // this offset is hardcoded.
    let rng_tick = world.tick.saturating_sub(1);
    let snapshot = &world.scratch_neighbor_energies;

    // Per-cell work: reads `snapshot` and `world_seed` / `rng_tick` /
    // `coeff` only via shared captures, writes only into its own
    // `cell.rates`. Each cell's RNG is freshly seeded from
    // `(world_seed, rng_tick, coord)` — order of execution doesn't
    // affect the result, so this is safe to run in parallel without
    // breaking the bit-identity contract against JS prototype 9-B.
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

    #[cfg(not(target_arch = "wasm32"))]
    world
        .cells
        .par_iter_mut()
        .for_each(|(c, cell)| body(c, cell));
    #[cfg(target_arch = "wasm32")]
    for (c, cell) in &mut world.cells {
        body(c, cell);
    }
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
/// **Parallelism:** the value-fill pass runs in parallel via rayon
/// (native targets). Each entry's six neighbor lookups read the
/// immutable `world.cells` map by coord, so the parallel walk is
/// bit-identical to a sequential one.
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

    #[cfg(not(target_arch = "wasm32"))]
    snapshot.par_iter_mut().for_each(|(c, e)| body(c, e));
    #[cfg(target_arch = "wasm32")]
    for (c, e) in snapshot.iter_mut() {
        body(c, e);
    }
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
/// direction(s); the spec (and JS prototype 9-B) say the actual
/// per-tick emission size in that direction is `rates + active_outflow`,
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
    let world_seed = world.world_seed;
    let rng_tick = world.tick;
    let body = |coord: &Coord, per_dir: &mut [Vec<u32>; Direction::COUNT]| {
        for v in per_dir.iter_mut() {
            v.clear();
        }
        let Some(cell) = cells.get(coord) else {
            return;
        };
        let mem_size = cell.memory.len();
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
                buf.extend_from_slice(&cell.memory[ptr..end]);
            } else {
                let tail = mem_size - ptr;
                buf.extend_from_slice(&cell.memory[ptr..mem_size]);
                let wrap = rate - tail;
                buf.extend_from_slice(&cell.memory[..wrap]);
            }
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    outflow
        .par_iter_mut()
        .for_each(|(c, per_dir)| body(c, per_dir));
    #[cfg(target_arch = "wasm32")]
    for (c, per_dir) in outflow.iter_mut() {
        body(c, per_dir);
    }
}

/// Lay out per-direction pointers for every cell in the world.
///
/// Uses each cell's combined rate (`rates + active_outflow`) as the
/// per-direction consumption budget, **clamped** to the cell's memory
/// size so that total emission never exceeds what the cell actually
/// holds. Honors any `pointer_override` flags set by a CPU-phase
/// `setp` / `setpv` instruction this tick.
///
/// This is the sub-tick reflow step from `docs/mechanics.md`, matching
/// JS prototype 9-B's `applyCombinedLayout`. See [`combined_clamped`]
/// for the per-direction `u64` accumulation that keeps Rust bit-aligned
/// with JS `Number` arithmetic when `rates + active_outflow` exceeds
/// `u32::MAX`.
pub fn lay_out_pointers(world: &mut SparseWorld) {
    // Per-cell pointer layout has no inter-cell dependencies — each
    // cell only reads its own rates / active_outflow / memory size.
    // Parallelizing is bit-identical to the sequential walk.
    let world_seed = world.world_seed;
    let rng_tick = world.tick;
    let body = |coord: &Coord, cell: &mut Cell| {
        let mem_size = cell.memory.len();
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

    #[cfg(not(target_arch = "wasm32"))]
    world
        .cells
        .par_iter_mut()
        .for_each(|(c, cell)| body(c, cell));
    #[cfg(target_arch = "wasm32")]
    for (c, cell) in &mut world.cells {
        body(c, cell);
    }
}

/// RNG domain salt for [`combined_clamped`]'s leftover-distribution
/// tie-break. Distinct from the default domain (`0`) used by
/// [`compute_natural_rates`] so the two streams cannot correlate even
/// when they share `(world_seed, tick, coord)`.
pub(crate) const COMBINED_CLAMPED_RNG_DOMAIN: u32 = 1;

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
/// The clamp uses `f64` for `cap / total` and the per-direction
/// `combined * scale` step, matching JS prototype 9-B's
/// `Math.floor(combined * cap / total)` to the bit. `u64` integer
/// division would also be correct mathematically but disagrees with
/// JS at boundary values (truncation vs round-to-nearest-then-floor).
///
/// ## Leftover distribution — Largest-Remainder with shuffled tie-break
///
/// After `floor(combined * scale)` the per-direction sum may be up to
/// `Direction::COUNT - 1 = 5` short of `cap`. Hamilton/Hare
/// apportionment closes that gap: indices are sorted by their
/// fractional remainder descending and the top `leftover` get `+1`
/// each. Ties between equal remainders are broken by a Fisher-Yates
/// shuffle of `[0..6]` seeded from `(world_seed, rng_tick, coord)`
/// under a dedicated [`COMBINED_CLAMPED_RNG_DOMAIN`].
///
/// **Statistical isotropy.** Across many `(world_seed, rng_tick,
/// coord)` triples each direction wins/loses the tie-break with equal
/// probability, so the leftover distribution does not introduce a
/// systematic preference for any face — the macro emission balance
/// over a populated world is uniform across `Direction::ALL`. (Strict
/// per-call equivariance under direction permutation is provably
/// incompatible with exact conservation + integer outputs + per-
/// direction non-exceedance, so it is not part of the contract; see
/// `tick_combined_clamped_contracts.rs` for the operational test.)
///
/// The fast path (`total <= cap`) skips RNG and sort entirely; only
/// the actually-clamping path pays the (constant, six-element) cost.
#[must_use]
pub fn combined_clamped(
    rates: &[u32; Direction::COUNT],
    active_outflow: &[u32; Direction::COUNT],
    cap: u32,
    world_seed: u64,
    rng_tick: u64,
    coord: Coord,
) -> [u32; Direction::COUNT] {
    let combined_u64: [u64; Direction::COUNT] =
        std::array::from_fn(|i| u64::from(rates[i]) + u64::from(active_outflow[i]));
    let total: u64 = combined_u64.iter().sum();
    let cap64 = u64::from(cap);
    if total <= cap64 {
        // Each `combined_u64[i]` is bounded above by `total <= cap` here,
        // so `as u32` is lossless.
        #[allow(clippy::cast_possible_truncation)]
        return std::array::from_fn(|i| combined_u64[i] as u32);
    }
    // Clamp via `f64` to bit-match JS. `total` reaches at most
    // `6 * (u32::MAX + small_natural_rate)` ≈ `2^34.6`, well under
    // `f64`'s `2^53` exact-integer ceiling.
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
        let combined_f = combined_u64[i] as f64;
        let scaled = combined_f * scale;
        let floored = scaled.floor();
        frac[i] = scaled - floored;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let val = floored as u32;
        clamped[i] = val;
        new_total = new_total.saturating_add(val);
    }
    // Distribute leftover by Largest-Remainder with shuffled tie-break.
    // `cap >= new_total` always holds: each `floored ≤ combined * scale`
    // and `sum(combined * scale) = cap`, so `new_total ≤ cap`. The
    // shuffle + sort runs unconditionally even when `leftover == 0`
    // (the rare case where `f64` rounding leaves `new_total == cap`
    // exactly): a `take(0)` below makes that path a no-op without an
    // observable-equivalent `if leftover > 0` early-skip that mutation
    // tests would correctly flag as redundant.
    let leftover = cap.saturating_sub(new_total) as usize;
    let mut order: [usize; Direction::COUNT] = [0, 1, 2, 3, 4, 5];
    let mut rng = Rng::for_cell_at_tick(world_seed, rng_tick, coord, COMBINED_CLAMPED_RNG_DOMAIN);
    // Fisher-Yates shuffle of `order`. Indices i in (1..6).rev() pick a
    // uniformly-distributed swap target in `0..=i` from the RNG. After
    // this loop `order` is a uniformly-random permutation of
    // `[0, 1, 2, 3, 4, 5]` deterministic in `(world_seed, rng_tick,
    // coord)`.
    for i in (1..Direction::COUNT).rev() {
        // `next_u32() as usize % (i + 1)` — unbiased enough at this
        // tiny range; the modulo bias for a 32-bit draw over 2..=6 is
        // below 2^-29, and we are already shuffling six elements.
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
/// in parallel via rayon (native targets); each cell's work depends
/// only on its own state plus the read-only `inflows_by_target` map,
/// so the parallel walk is bit-identical to a sequential one.
#[allow(clippy::too_many_lines)]
pub fn apply_outflow(world: &mut SparseWorld, outflow: &Outflow) {
    // -------------------------------------------------------------------
    // Phase 1: shrink each source by its total outgoing slot count.
    // -------------------------------------------------------------------
    // After this loop, `cell.energy()` for every source equals
    // `pre - total_outflow` — exactly the post-burn / post-outflow
    // value the dominance math needs. No separate snapshot map needed.
    for (coord, per_dir) in outflow {
        let total: u32 = per_dir
            .iter()
            .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX))
            .fold(0u32, u32::saturating_add);
        if total == 0 {
            continue;
        }
        if let Some(cell) = world.cells.get_mut(coord) {
            cell.shrink_from_end(total);
        }
    }

    // -------------------------------------------------------------------
    // Phase 2: build per-target inflow lists with dominance.
    // -------------------------------------------------------------------
    //
    // The inflow map is pulled out of `world` via `mem::take` so its
    // per-target `Vec` capacities — `clear()`-reused across ticks —
    // skip the ~200 k `Vec::with_capacity(0)→reserve(N)` cycle the
    // freshly-built `FxHashMap::default()` version was paying.
    let move_threshold = world.move_threshold.max(f32::EPSILON);
    let mut inflows_by_target = std::mem::take(&mut world.scratch_inflows_by_target);
    // Clear value lengths but keep capacities; the keyset stays as
    // whatever it was last tick (most coords overlap), and stale
    // entries are dropped at the end of the apply phase.
    for v in inflows_by_target.values_mut() {
        v.clear();
    }
    inflows_by_target.reserve(outflow.len().saturating_sub(inflows_by_target.len()));

    for (source_coord, per_dir) in outflow {
        // One cell lookup covers both fields we need from the source
        // (energy and origin tag), instead of two separate `get` calls.
        let (attacker_post, src_origin_tag) = world
            .cells
            .get(source_coord)
            .map_or((0u32, 0u32), |c| (c.energy(), c.origin_tag));
        let attacker_post_burn = u32::max(1, attacker_post);

        for &d in &Direction::ALL {
            let slots = &per_dir[d.index()];
            if slots.is_empty() {
                continue;
            }
            let target_coord = source_coord.neighbor(d);
            let target_e_post = world.cells.get(&target_coord).map_or(0, Cell::energy);

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

    // -------------------------------------------------------------------
    // Phase 3: sort each target's inflows, alloc-on-write any void
    // targets, then apply intrusion inserts in parallel.
    // -------------------------------------------------------------------
    // Sort sequentially so the per-target apply can run from a
    // read-only `inflows_by_target` reference under `par_iter_mut`.
    // Empty `Vec`s (stale entries from previous ticks) are skipped —
    // their capacity stays reserved for next tick.
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
    // Alloc-on-write for any target that didn't exist yet, so the
    // parallel walk over `world.cells` finds every receiver as a
    // single mutable bucket. Skip stale entries with empty `Vec`s.
    for (target_coord, entries) in &inflows_by_target {
        if !entries.is_empty() {
            world.get_or_alloc(*target_coord);
        }
    }

    let inflows = &inflows_by_target;
    let body = |coord: &Coord, target: &mut Cell| {
        // Phase 0 (clear last tick's inflow tracking) fuses into the
        // same parallel iter so we don't need a separate full-cells
        // pass. `sinflow` reads what arrived in the previous tick;
        // anything left over from before has to go before we
        // accumulate this tick's arrivals.
        target.inflow = [0; Direction::COUNT];

        let Some(entries) = inflows.get(coord) else {
            return;
        };
        if entries.is_empty() {
            return;
        }

        // Origin-tag inheritance: highest-dominance source wins, but
        // only if its dominance crosses the metempsychosis threshold.
        if let Some(top) = entries.first() {
            if top.dominance >= 0.5 {
                target.origin_tag = top.src_origin_tag;
            }
        }

        // One reserve covers every following splice — keeps the
        // allocator out of the inner loop. Looking up slots on demand
        // (one `outflow.get` per applied entry) trades a cheap
        // cache-warm hash lookup for the ability to pool
        // `inflows_by_target` across ticks.
        let total_added: usize = entries
            .iter()
            .map(|e| {
                outflow
                    .get(&e.source_coord)
                    .map_or(0, |per_dir| per_dir[e.source_dir as usize].len())
            })
            .sum();
        target.memory.reserve(total_added);

        for entry in entries {
            let Some(per_dir) = outflow.get(&entry.source_coord) else {
                continue;
            };
            let slots = &per_dir[entry.source_dir as usize];
            let current = target.memory.len();
            let intrusion_depth = (entry.dominance * usize_to_f32(current)) as usize;
            let write_start = current.saturating_sub(intrusion_depth);

            target
                .memory
                .splice(write_start..write_start, slots.iter().copied());

            let dir_idx = entry.dir_from_target as usize;
            let slots_len = u32::try_from(slots.len()).unwrap_or(u32::MAX);
            target.inflow[dir_idx] = target.inflow[dir_idx].saturating_add(slots_len);
        }

        // PC stays numerically the same; bring it back into range if
        // memory ever shrank. (Inflow phase only grows, so this is
        // defensive — relevant only if a future change adds shrink.)
        if target.memory.is_empty() {
            target.pc = 0;
        } else {
            target.pc %= target.memory.len() as u32;
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    world
        .cells
        .par_iter_mut()
        .for_each(|(c, cell)| body(c, cell));
    #[cfg(target_arch = "wasm32")]
    for (c, cell) in &mut world.cells {
        body(c, cell);
    }

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
    let snapshot = &world.scratch_neighbor_energies;

    let body = |coord: &Coord, cell: &mut Cell| {
        let neighbors = snapshot
            .get(coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);
        let budget = cell.energy() / k_safe;
        for _ in 0..budget {
            crate::vm::execute_instruction(cell, &neighbors);
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    world
        .cells
        .par_iter_mut()
        .for_each(|(c, cell)| body(c, cell));
    #[cfg(target_arch = "wasm32")]
    for (c, cell) in &mut world.cells {
        body(c, cell);
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
    world.tick = world.tick.saturating_add(1);
}

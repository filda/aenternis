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

use crate::apportion::{
    apportion_with_shuffle, COMBINED_CLAMPED_RNG_DOMAIN, DENSITY_MUTATION_RNG_DOMAIN,
};
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
//
// `float_cmp`: the `gravity == 0.0 && pressure == 0.0` fast-path test is a
// deliberate exact comparison of config values (never computed), so no
// epsilon is wanted — `-0.0 == 0.0` holds and any non-zero turns the
// active path on. `suboptimal_flops`: the `drive` sum must stay as separate
// `*`/`+` ops. Fusing into `mul_add` (FMA) is a *single*-rounding op that is
// not guaranteed portable native↔wasm, which would break the bit-for-bit
// cross-host reproducibility contract this rate path depends on.
#[allow(clippy::float_cmp, clippy::suboptimal_flops)]
pub fn compute_natural_rates(world: &mut SparseWorld, coeff: f64) {
    // Pull Copy fields off the world so the per-cell closure below
    // doesn't hold a shared borrow during the (parallel) mutable phase.
    let gravity = world.gravity;
    let pressure = world.pressure;
    let gamma = world.pressure_gamma;
    let eref = world.pressure_eref;
    let alpha = world.gravity_alpha;
    let radius = world.gravity_radius;

    // Build the per-tick blocked energy grid up front and read all
    // neighbor energies through it — the 6-face snapshot always, and the
    // gravitational mass stencil when gravity is on. Both skip the
    // per-neighbor `cells.get` hash; the grid build (one hash per cell)
    // costs less than the 6-per-cell hashing the snapshot used to do, so
    // the gravity-off path (tests / frozen baseline) is faster too, with
    // byte-identical neighbor energies.
    refresh_energy_blocks(world);
    refresh_neighbor_energies(world);
    // Gravitational mass `M = α·Σ E(c+d)/|d|` is only needed when gravity
    // is on; on the gravity-off path `scratch_mass` stays empty and the
    // rate loop's frozen fast path never reads it.
    if gravity != 0.0 {
        refresh_mass(world, alpha, radius);
    }

    let world_seed = world.world_seed;
    // Convention: the layout for step #N is computed at the end of
    // step #(N-1), *before* `world.tick` is incremented — so the
    // rng_tick used here is one behind `world.tick`. Frozen: bumping
    // the off-by-one requires regenerating every RNG-derived baseline
    // (see `tests/apply_outflow_bit_parity.rs`).
    let rng_tick = world.tick.saturating_sub(1);
    let snapshot = &world.scratch_neighbor_energies;
    let mass_snapshot = &world.scratch_mass;

    // Per-cell work: reads `snapshot` / `mass_snapshot` and the `Copy`
    // physics params only via shared captures, writes only into its own
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

        // Exact `== 0.0` comparison is deliberate: these are config
        // values set literally to zero, not computed, so no epsilon is
        // wanted (and `-0.0 == 0.0` holds). The check is hoisted out of
        // the hot loop by the optimizer.
        if gravity == 0.0 && pressure == 0.0 {
            // FROZEN FAST PATH — byte-for-byte the pre-gravity code.
            // Any drift here re-blesses `apply_outflow_bit_parity.rs`,
            // so this branch must stay textually identical.
            for &d in &Direction::ALL {
                let neighbor_energy = neighbor_energies[d.index()];
                let rate = if my_energy > neighbor_energy {
                    let delta = my_energy - neighbor_energy;
                    // `coeff` is `f64`, so JS `Number(0.15)` flows
                    // through unchanged — no `f32(0.15)→f64` artifact.
                    rng.stochastic_floor(f64::from(delta) * coeff)
                } else {
                    0
                };
                cell.rates[d.index()] = rate;
            }
        } else {
            // Active path: radiation (down the energy gradient) + pressure
            // (outward, ∝ E^γ) + gravity (toward mass, can flow uphill).
            let m_c = mass_snapshot.get(coord).copied().unwrap_or(0.0);
            let pi_self = pressure_pi(my_energy, pressure, eref, gamma);
            for &d in &Direction::ALL {
                let neighbor_energy = neighbor_energies[d.index()];
                let m_nbr = mass_snapshot
                    .get(&coord.neighbor(d))
                    .copied()
                    .unwrap_or(0.0);
                let pi_nbr = pressure_pi(neighbor_energy, pressure, eref, gamma);
                let drive = coeff * (f64::from(my_energy) - f64::from(neighbor_energy))
                    + (pi_self - pi_nbr)
                    + gravity * (m_nbr - m_c);
                // Explicit `drive > 0.0` guard — never `stochastic_floor(
                // max(0.0, drive))`, which would consume an RNG draw on
                // the `drive == 0` edge and desync the per-cell stream.
                // This guard consumes exactly one draw iff `drive > 0.0`.
                let rate = if drive > 0.0 {
                    rng.stochastic_floor(drive)
                } else {
                    0
                };
                cell.rates[d.index()] = rate;
            }
        }
        if cell.total_rate() > my_energy {
            proportional_clamp(&mut cell.rates, my_energy, world_seed, rng_tick, *coord);
        }
    };

    par_or_seq_iter_mut!(&mut world.cells, body);
}

/// Pressure potential `Π(E) = pressure · eref · (E/eref)^γ` — the outward
/// counter-force that grows with local density. Returns `0.0` when
/// pressure is off, which also short-circuits the `(E/eref)` division
/// (so `eref == 0` can't produce a stray `NaN` on the gravity-only path).
///
/// γ is evaluated via [`gamma_pow`], i.e. only through `*` and `sqrt`,
/// both IEEE-754 correctly-rounded on every target — so `Π` is bit-for-bit
/// reproducible across native and wasm. See `docs/mechanics.md`.
//
// `float_cmp`: `pressure == 0.0` is a deliberate exact off-switch test.
// `suboptimal_flops`: keep `pressure * eref * pow` as plain multiplies —
// `mul_add` would introduce a non-portable FMA rounding (see
// `compute_natural_rates`).
#[allow(clippy::float_cmp, clippy::suboptimal_flops)]
fn pressure_pi(energy: u32, pressure: f64, eref: f64, gamma: f64) -> f64 {
    if pressure == 0.0 {
        return 0.0;
    }
    let x = f64::from(energy) / eref;
    pressure * eref * gamma_pow(x, gamma)
}

/// `x^γ` for the portable set of polytropic indices
/// `{1.0, 1.5, 2.0, 2.5, 3.0}`, built from multiply and `sqrt` only —
/// both IEEE correctly-rounded, so the result is bit-identical on every
/// target. Arbitrary γ would require `powf`, which is *not* correctly
/// rounded (last-ULP differences native↔wasm could flip a
/// `stochastic_floor` decision and silently diverge two worlds), so it is
/// out of scope; the config layer snaps γ to a supported value.
///
/// An unsupported γ falls back to γ=2 (`x·x`): deterministic, portable,
/// and a `debug_assert` flags it in test builds so a bad config is caught
/// loudly rather than silently mis-evaluated.
//
// `float_cmp`: γ is matched against the exact portable values the config
// layer snaps it to (see `snap_gamma` in `aenternis-wasm`), so exact `==`
// is correct here — an epsilon match would blur adjacent indices.
#[allow(clippy::float_cmp)]
fn gamma_pow(x: f64, gamma: f64) -> f64 {
    if gamma == 1.0 {
        x
    } else if gamma == 1.5 {
        x * x.sqrt()
    } else if gamma == 2.5 {
        x * x * x.sqrt()
    } else if gamma == 3.0 {
        x * x * x
    } else {
        // γ == 2.0 (the default) lands here, as does any value the config
        // layer failed to snap. Both evaluate `x²`; the `debug_assert`
        // catches the latter loudly in test builds. Folding γ=2 into this
        // arm (rather than a separate `else` after `debug_assert!(false)`)
        // keeps the arithmetic reachable, so its mutants are caught.
        debug_assert!(
            gamma == 2.0,
            "unsupported pressure_gamma {gamma} — config layer must snap to {{1, 1.5, 2, 2.5, 3}}"
        );
        x * x
    }
}

/// Rebuild the per-tick blocked energy grid: scatter each live cell's
/// energy into its dense `GRAV_BLOCK_SIDE³` block (see
/// [`SparseWorld::scratch_energy_block_idx`]). `clear` keeps both the
/// map's buckets and the pool's `[u32; VOL]` backing across ticks; a
/// pooled block is overwritten with a fresh zeroed array on reuse, so a
/// cell only ever sees its own slot set (the rest stay 0 = void).
///
/// Built once per tick, then shared by [`refresh_neighbor_energies`]
/// (6-face) and [`refresh_mass`] (stencil) so both skip the per-coord
/// `cells.get` hash. A caller must run this before either of those.
fn refresh_energy_blocks(world: &mut SparseWorld) {
    let cells = &world.cells;
    let idx = &mut world.scratch_energy_block_idx;
    let pool = &mut world.scratch_energy_block_pool;
    idx.clear();
    pool.clear();
    for (coord, cell) in cells.iter() {
        let e = cell.energy();
        if e == 0 {
            continue;
        }
        let slot = *idx
            .entry(block_coord(coord.x, coord.y, coord.z))
            .or_insert_with(|| {
                let i = u32::try_from(pool.len()).unwrap_or(u32::MAX);
                pool.push([0u32; GRAV_BLOCK_VOL]);
                i
            });
        pool[slot as usize][block_local(coord.x, coord.y, coord.z)] = e;
    }
}

/// Rebuild [`SparseWorld::scratch_neighbor_energies`] — a `Coord → [u32; 6]`
/// snapshot of each cell's six face-neighbor energies (0 for void).
/// Reused across `compute_natural_rates` and `cpu_phase` in one [`step`]
/// (both observe the same energies, since `memory.len()` is fixed inside
/// the CPU phase), so building once serves both.
///
/// Reads energies from the pre-built [`refresh_energy_blocks`] grid
/// rather than hashing every neighbor coord into `world.cells`: an
/// interior cell's six faces share its own block, so the per-cell work
/// collapses to ~1 block resolve plus dense reads. **`refresh_energy_blocks`
/// must run first.**
///
/// **Allocation:** the snapshot keyset is sync'd to `world.cells` in
/// place (stale dropped, new inserted zeroed), so storage carries across
/// ticks. **Parallelism:** the fill is dispatched through
/// [`crate::parallel::par_or_seq_iter_mut!`]; each entry reads only the
/// immutable grid by coord, so the parallel walk is bit-identical to the
/// sequential one.
fn refresh_neighbor_energies(world: &mut SparseWorld) {
    {
        let cells = &world.cells;
        let snapshot = &mut world.scratch_neighbor_energies;
        snapshot.retain(|coord, _| cells.contains_key(coord));
        snapshot.reserve(cells.len().saturating_sub(snapshot.len()));
        for coord in cells.keys() {
            snapshot.entry(*coord).or_default();
        }
    }

    let block_idx = &world.scratch_energy_block_idx;
    let block_pool = &world.scratch_energy_block_pool;
    let body = |coord: &Coord, energies: &mut [u32; Direction::COUNT]| {
        let mut cur_bc: Option<Coord> = None;
        let mut cur_block: Option<&[u32; GRAV_BLOCK_VOL]> = None;
        for &d in &Direction::ALL {
            let nc = coord.neighbor(d);
            let bc = block_coord(nc.x, nc.y, nc.z);
            if cur_bc != Some(bc) {
                cur_bc = Some(bc);
                cur_block = block_idx.get(&bc).map(|&i| &block_pool[i as usize]);
            }
            energies[d.index()] = cur_block.map_or(0, |b| b[block_local(nc.x, nc.y, nc.z)]);
        }
    };

    par_or_seq_iter_mut!(&mut world.scratch_neighbor_energies, body);
}

/// Gravity mass-gather block grid. Space is tiled into dense
/// `GRAV_BLOCK_SIDE³` energy blocks; a block coord is `coord >>
/// GRAV_BLOCK_BITS` per axis (arithmetic shift floors for negatives) and
/// the in-block index packs `coord & MASK` per axis. `GRAV_BLOCK_VOL` is
/// the pool element length (see `SparseWorld::scratch_energy_block_pool`).
pub(crate) const GRAV_BLOCK_BITS: i32 = 3;
const GRAV_BLOCK_SIDE: i32 = 1 << GRAV_BLOCK_BITS;
const GRAV_BLOCK_MASK: i32 = GRAV_BLOCK_SIDE - 1;
pub(crate) const GRAV_BLOCK_VOL: usize =
    (GRAV_BLOCK_SIDE * GRAV_BLOCK_SIDE * GRAV_BLOCK_SIDE) as usize;

/// Block coord containing `(x, y, z)`.
#[inline]
const fn block_coord(x: i32, y: i32, z: i32) -> Coord {
    Coord::new(
        x >> GRAV_BLOCK_BITS,
        y >> GRAV_BLOCK_BITS,
        z >> GRAV_BLOCK_BITS,
    )
}

/// In-block index for `(x, y, z)` (`0..GRAV_BLOCK_VOL`).
///
/// **Accepted-as-unkillable mutants** (`cargo mutants`): the two
/// `|` → `^` swaps. The three masked fields occupy disjoint bit ranges
/// (`[0,3)`, `[3,6)`, `[6,9)`), so `^` and `|` produce byte-identical
/// output — a genuinely equivalent mutant no test can distinguish.
#[inline]
const fn block_local(x: i32, y: i32, z: i32) -> usize {
    (((z & GRAV_BLOCK_MASK) << (2 * GRAV_BLOCK_BITS))
        | ((y & GRAV_BLOCK_MASK) << GRAV_BLOCK_BITS)
        | (x & GRAV_BLOCK_MASK)) as usize
}

/// Build the gravitational stencil for cutoff radius `R`: every integer
/// offset `d` with `0 < |d| ≤ R`, paired with its `1/r` kernel weight
/// `1/|d|`. Offsets are emitted in a fixed `(dz, dy, dx)`-ascending order
/// so the downstream mass sum is reproducible.
///
/// Weights use only `sqrt` and `/`, both IEEE-754 correctly-rounded on
/// every target, so the stencil — and the masses built from it — are
/// bit-for-bit identical across native and wasm. `R ≤ 0` yields an empty
/// stencil (mass everywhere `0`, i.e. gravity inert).
fn gravity_stencil(radius: i32) -> Vec<(Coord, f64)> {
    let r = radius.max(0);
    let r2 = r * r;
    let mut stencil = Vec::new();
    for dz in -r..=r {
        for dy in -r..=r {
            for dx in -r..=r {
                let d2 = dx * dx + dy * dy + dz * dz;
                if d2 > 0 && d2 <= r2 {
                    // `f64::from(i32)` is lossless; `sqrt` + `/` are
                    // correctly-rounded → portable weight.
                    let weight = 1.0 / f64::from(d2).sqrt();
                    stencil.push((Coord::new(dx, dy, dz), weight));
                }
            }
        }
    }
    stencil
}

/// Rebuild [`SparseWorld::scratch_mass`] — the gravitational potential
/// `M(c) = gravity_alpha · Σ_{0<|d|≤R} E(c+d) / |d|`, evaluated with the
/// [`gravity_stencil`] for the world's `gravity_radius`.
///
/// **Keyset = occupied ∪ face-shell.** `M` is computed not only at every
/// live cell but also at each *face neighbor* of a live cell, including
/// void ones — because [`compute_natural_rates`] reads `M` at those
/// face neighbors to decide flow, and a void point near distant mass has
/// a real (non-zero) potential. Computing it here (rather than on the fly
/// in the rate loop) is what lets that loop borrow `scratch_mass` while
/// it mutably walks `world.cells`, with no aliasing.
///
/// **Determinism:** each cell's sum walks the stencil in its fixed order
/// over `f64` values produced only by correctly-rounded ops, so the
/// result is independent of cell iteration order — bit-identical
/// sequential or parallel, native or wasm.
///
/// **Cost:** `O((N + shell)·R³)`. Only called when
/// [`SparseWorld::gravity`] is non-zero.
//
// `suboptimal_flops`: the `acc += e*weight` accumulation must stay
// separate ops — `mul_add`'s single rounding is not portable native↔wasm.
#[allow(clippy::suboptimal_flops)]
fn refresh_mass(world: &mut SparseWorld, alpha: f64, radius: i32) {
    let stencil = gravity_stencil(radius);

    // Keyset: every live cell plus its six face neighbors (the coords the
    // rate loop will query `M` at). Cleared and rebuilt each tick; `clear`
    // retains the backing capacity so there's no per-tick allocator churn.
    {
        let cells = &world.cells;
        let mass = &mut world.scratch_mass;
        mass.clear();
        for coord in cells.keys() {
            mass.entry(*coord).or_insert(0.0);
            for &d in &Direction::ALL {
                mass.entry(coord.neighbor(d)).or_insert(0.0);
            }
        }
    }

    // The blocked energy grid (`refresh_energy_blocks`) is built by the
    // caller before this runs, so the gather below resolves neighbor
    // energies through it instead of a per-offset `cells.get` hash.
    let stencil = &stencil;
    let block_idx = &world.scratch_energy_block_idx;
    let block_pool = &world.scratch_energy_block_pool;
    let body = |coord: &Coord, m: &mut f64| {
        let mut acc = 0.0_f64;
        // Re-resolve the block only when the stencil walk crosses a block
        // boundary. The `(dz, dy, dx)`-ascending stencil order keeps long
        // same-block runs, so this collapses the per-offset `cells.get`
        // hash probe to a few dozen lookups plus a dense in-block read.
        // Energies are added in the exact stencil order — same values,
        // same order — so the `f64` mass sum is bit-identical to the
        // per-cell-hash version (the bit-parity baseline is untouched).
        let mut cur_bc: Option<Coord> = None;
        let mut cur_block: Option<&[u32; GRAV_BLOCK_VOL]> = None;
        for (off, weight) in stencil {
            let nx = coord.x + off.x;
            let ny = coord.y + off.y;
            let nz = coord.z + off.z;
            let bc = block_coord(nx, ny, nz);
            if cur_bc != Some(bc) {
                cur_bc = Some(bc);
                cur_block = block_idx.get(&bc).map(|&i| &block_pool[i as usize]);
            }
            let e = cur_block.map_or(0, |b| b[block_local(nx, ny, nz)]);
            // Plain `+ x*w`, never `mul_add`: FMA's single rounding is not
            // portable native↔wasm and would break reproducibility.
            acc += f64::from(e) * weight;
        }
        *m = alpha * acc;
    };

    par_or_seq_iter_mut!(&mut world.scratch_mass, body);
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
    // Take the pooled map out, clear it, and reuse the existing
    // hashbrown bucket array. After `clear()` len is 0, so `reserve`
    // is a no-op in steady state (capacity preserved) and only grows
    // on the first tick / after a catastrophic shrink.
    let mut per_source_total_outflow = std::mem::take(&mut world.scratch_per_source_total_outflow);
    per_source_total_outflow.clear();
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
    // Drop entries for targets whose cell has since been gc'd. Without
    // this retain the outer map (and every dead target's inner
    // `Vec<InflowEntry>` capacity) grows monotonically with the set of
    // ever-been-a-target coords — same leak the fused
    // [`outflow_phase_inplace`] would otherwise hit, kept in sync here
    // so `step_diffusion` and direct `apply_outflow` callers don't
    // bleed memory in long runs.
    inflows_by_target.retain(|coord, _| world.cells.contains_key(coord));
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
    // Same for the per-source total-outflow map — keeping its
    // hashbrown bucket array allocated across ticks turns the
    // 17 MB-per-tick churn at ~700 k cells into a single up-front
    // grow.
    world.scratch_per_source_total_outflow = per_source_total_outflow;
}

/// Inflow descriptor built during phase 2 of [`outflow_phase_inplace`].
///
/// Pre-resolves the source's slot range in `arena_cur` so the write
/// phase can `memcpy` source slots straight into `arena_next` without
/// any further world lookup. The wrap-around case (rare: only when
/// `source.pointers[d] + rate > source.mem_len`) is encoded as a
/// non-zero `wrap_len` carrying the prefix that wraps to the start of
/// the source's memory range.
///
/// Sibling to [`InflowEntry`], which is used by the public-API
/// [`apply_outflow`] path (tests + `step_diffusion`). The reason we
/// don't share: `InflowEntry` keys back into the `Outflow` map by
/// `(source_coord, source_dir)`, while `InflowFast` carries the
/// resolved arena ranges directly — fewer hashmap lookups in the hot
/// per-tick path.
#[derive(Debug, Clone, Copy)]
pub struct InflowFast {
    /// First slot of the inflow in `arena_cur` (= `source.mem_start +
    /// source.pointers[source_dir]`). Valid for `head_len` slots.
    pub head_start: u32,
    /// Number of slots starting at `head_start`. Equals the inflow's
    /// total rate when there is no wrap; otherwise the part before the
    /// wrap point.
    pub head_len: u32,
    /// Start of the wrap-around portion in `arena_cur` (= `source
    /// .mem_start`). Only meaningful when `wrap_len > 0`.
    pub wrap_start: u32,
    /// Slots in the wrap-around portion. Zero in the common case (no
    /// wrap).
    pub wrap_len: u32,
    /// Dominance against the target (`0.0..=1.0`).
    pub dominance: f32,
    /// Attacker's `origin_tag`, used for metempsychosis when this
    /// inflow's dominance crosses the `>= 0.5` threshold and it's the
    /// top-ranked entry for the target.
    pub src_origin_tag: u32,
    /// Direction-from-target index (`0..6`). Used for the secondary
    /// sort key and to bump `target.inflow[dir_from_target]`.
    pub dir_from_target: u8,
}

impl InflowFast {
    /// Total slot count of this inflow (= `head_len + wrap_len`).
    #[must_use]
    #[inline]
    pub const fn slot_count(self) -> u32 {
        self.head_len.saturating_add(self.wrap_len)
    }
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

/// Fused replacement for [`lay_out_pointers`] + [`collect_outflow`] +
/// [`apply_outflow`] in [`step`]'s per-tick path.
///
/// Eliminates the per-cell-per-direction `Vec<u32>` intermediate that
/// [`Outflow`] kept — slot data flows from `arena_cur` straight into
/// `arena_next` in a single memcpy, instead of the old two-stage
/// `arena_cur → Outflow Vec → arena_next` pipeline. At ~500 k cells
/// the dominant phase (`apply_outflow` ≈ 65 % of tick per
/// `examples/phase_breakdown`) loses both the ~12 hashmap lookups per
/// cell that the old structure needed and the redundant memcpy of
/// every slot leaving the world.
///
/// Semantics match the chained `lay_out_pointers` then
/// `collect_outflow` then `apply_outflow` flow exactly — the
/// `tests/apply_outflow_bit_parity.rs` baseline pins every observable
/// per-cell field tick-by-tick, so any divergence surfaces as a hash
/// mismatch under `./check`.
///
/// **Phases:**
///
/// 1. Per source: compute combined rate (`rates + active_outflow`,
///    proportionally clamped to `mem_len`), lay out `cell.pointers`
///    from the end of memory honoring `pointer_override`, cache total
///    outflow in `scratch_per_source_total_outflow`. Sequential so
///    the pointer layout and the scratch insert share one pass.
/// 2. Build per-target [`InflowFast`] lists. Each entry pre-resolves
///    the source's slot range in `arena_cur`
///    (`head_start, head_len, wrap_start, wrap_len`) plus dominance
///    from post-outflow energies, so phase 4 needs no further
///    `world.cells` lookup to read source slots.
/// 3. Sort each target's inflows (`dominance desc, dir_from_target
///    asc`) and alloc-on-write metadata for new targets.
/// 4. For each cell: bump-allocate `new_len` slots in `arena_next`,
///    rope-merge the post-outflow remainder of the cell's old memory
///    with the sorted inflows, copy segments directly from
///    `arena_cur`. Honors origin-tag inheritance and PC wrap.
/// 5. `mem::swap(&mut world.arena, &mut world.arena_next)` and return
///    the pooled scratch maps to the world for next-tick reuse.
///
/// **Determinism:** Phase 1's `combined_clamped` reads only the cell's
/// own state plus `(world_seed, rng_tick, coord)`, so its iteration
/// order doesn't matter; phase 2's `combined_clamped` recompute hits
/// the same inputs and produces bit-identical rates. The dominance
/// formula and tie-break sort are identical to the legacy path.
#[allow(clippy::too_many_lines)]
pub fn outflow_phase_inplace(world: &mut SparseWorld) {
    let world_seed = world.world_seed;
    let rng_tick = world.tick;
    let move_threshold = world.move_threshold.max(f32::EPSILON);

    // -------------------------------------------------------------------
    // Phase 1: combined rates + pointer layout + total outflow per source.
    // -------------------------------------------------------------------
    // Sequential to keep the three operations in one pass — splitting
    // the pointer layout into a parallel branch would require a side
    // channel to ferry the total back out, and the per-cell work here
    // is light enough that the sequential pass costs ~2 % of tick time
    // even at 500 k cells.
    let mut per_source_total = std::mem::take(&mut world.scratch_per_source_total_outflow);
    per_source_total.clear();
    per_source_total.reserve(world.cells.len().saturating_sub(per_source_total.len()));

    for (coord, cell) in world.cells.iter_mut() {
        let mem_size = cell.memory_len();
        if mem_size == 0 {
            per_source_total.insert(*coord, 0);
            continue;
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
        let total: u32 = combined.iter().copied().fold(0u32, u32::saturating_add);
        per_source_total.insert(*coord, total);
    }

    // -------------------------------------------------------------------
    // Phase 2: build per-target inflow lists with dominance + pre-
    // resolved source slot ranges in `arena_cur`.
    // -------------------------------------------------------------------
    // Pulled out via `mem::take` so per-target `Vec<InflowFast>`
    // capacities persist across ticks. The `retain` *must* run before
    // the inner `clear()` — without it, stale entries for cells that
    // have died via `gc_empty` since they were last a target stay
    // forever, and their inner `Vec<InflowFast>` capacities pile up
    // monotonically. In a long-running sim with diffusion across a
    // growing bbox this leak reached ~29 M outer entries plus ~92 M
    // pooled `InflowFast` slots (~3 GB) by tick ~2000 at 686 k live
    // cells — the actual OOM trigger observed in production, not the
    // arena (`apply_outflow` was already alloc-on-write disciplined).
    // Matches the existing pattern in [`collect_outflow_into`] /
    // [`refresh_neighbor_energies`]: drop dead keys, then clear+refill.
    let mut inflows = std::mem::take(&mut world.scratch_inflows_fast);
    inflows.retain(|coord, _| world.cells.contains_key(coord));
    for v in inflows.values_mut() {
        v.clear();
    }
    inflows.reserve(world.cells.len().saturating_sub(inflows.len()));

    // Target energies come from the neighbor snapshot instead of a
    // `cells.get` hash probe per direction (6 per source). The snapshot
    // is built by `compute_natural_rates` at the top of this tick and the
    // CPU phase never changes `mem_len` (instructions rewrite content,
    // not length), so `snapshot[source][d] == cells[target].mem_len`
    // exactly — bit-identical, one lookup per source instead of six.
    debug_assert_eq!(
        world.scratch_neighbor_energies.len(),
        world.cells.len(),
        "outflow_phase_inplace requires the neighbor snapshot from this \
         tick's compute_natural_rates (it runs inside step after initialize)"
    );
    let neighbor_snapshot = &world.scratch_neighbor_energies;

    for (source_coord, source_cell) in world.cells.iter() {
        let mem_size = source_cell.memory_len();
        if mem_size == 0 {
            continue;
        }
        let mem_size_u32 = u32::try_from(mem_size).unwrap_or(u32::MAX);
        // Recompute combined — the same `(world_seed, rng_tick, coord)`
        // RNG keying as phase 1 produces identical rates. Caching
        // combined in a side map was considered, but at 6 u32 × cells
        // it's ~12 MB peak at 500 k cells — same magnitude as the OOM
        // pressure we're trying to relieve, for a 1.7 % CPU saving.
        let combined = combined_clamped(
            &source_cell.rates,
            &source_cell.active_outflow,
            mem_size_u32,
            world_seed,
            rng_tick,
            *source_coord,
        );
        let src_total = per_source_total.get(source_coord).copied().unwrap_or(0);
        let attacker_post = source_cell.mem_len.saturating_sub(src_total);
        let attacker_post_burn = attacker_post.max(1);
        let source_mem_start = source_cell.mem_start;
        let source_mem_len = source_cell.mem_len;
        let src_origin_tag = source_cell.origin_tag;
        let target_energies = neighbor_snapshot
            .get(source_coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);

        for &d in &Direction::ALL {
            let rate = combined[d.index()];
            if rate == 0 {
                continue;
            }

            let target_coord = source_coord.neighbor(d);
            let tgt_mem_len = target_energies[d.index()];
            let tgt_total = per_source_total.get(&target_coord).copied().unwrap_or(0);
            let target_e_post = tgt_mem_len.saturating_sub(tgt_total);

            let r = u32_to_f32(target_e_post) / u32_to_f32(attacker_post_burn);
            let dom = (1.0 - r / move_threshold).clamp(0.0, 1.0);

            // Resolve the source's slot range in `arena_cur` once,
            // including the rare wraparound (when `ptr + rate >
            // mem_len`). Phase 4's flatten consumes this directly.
            let ptr = source_cell.pointers[d.index()];
            let tail = source_mem_len.saturating_sub(ptr);
            let (head_len, wrap_len) = if rate <= tail {
                (rate, 0)
            } else {
                (tail, rate - tail)
            };

            inflows.entry(target_coord).or_default().push(InflowFast {
                head_start: source_mem_start.saturating_add(ptr),
                head_len,
                wrap_start: source_mem_start,
                wrap_len,
                dominance: dom,
                src_origin_tag,
                dir_from_target: d.opposite().index() as u8,
            });
        }
    }

    // -------------------------------------------------------------------
    // Phase 3: sort each target's inflows + alloc-on-write metadata.
    // -------------------------------------------------------------------
    for entries in inflows.values_mut() {
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

    for (target_coord, entries) in &inflows {
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
    world.arena_next.clear();
    {
        let cells = &mut world.cells;
        let arena_cur = &world.arena;
        let arena_next = &mut world.arena_next;
        let inflows_ref = &inflows;
        let per_source_ref = &per_source_total;

        for (coord, cell) in cells.iter_mut() {
            let old_mem_start = cell.mem_start;
            let old_mem_len = cell.mem_len;
            let total_outflow = per_source_ref.get(coord).copied().unwrap_or(0);
            let post_outflow_len = old_mem_len.saturating_sub(total_outflow);
            let entries: &[InflowFast] = inflows_ref.get(coord).map_or(&[], |e| e.as_slice());

            cell.inflow = [0; Direction::COUNT];

            let total_inflow: u32 = entries
                .iter()
                .map(|e| e.slot_count())
                .fold(0u32, u32::saturating_add);
            let new_len = post_outflow_len.saturating_add(total_inflow);

            if new_len == 0 {
                cell.mem_start = 0;
                cell.mem_len = 0;
                continue;
            }

            let new_start = arena_next.alloc(new_len);

            write_cell_into_next_arena_fast(
                cell,
                arena_cur,
                arena_next,
                new_start,
                new_len,
                old_mem_start,
                post_outflow_len,
                entries,
            );
        }
    }

    // -------------------------------------------------------------------
    // Phase 5: swap arenas + return scratch.
    // -------------------------------------------------------------------
    std::mem::swap(&mut world.arena, &mut world.arena_next);
    world.scratch_inflows_fast = inflows;
    world.scratch_per_source_total_outflow = per_source_total;
}

/// Sibling of [`write_cell_into_next_arena`] for the
/// [`outflow_phase_inplace`] fast path. Same rope-merge semantics, but
/// reads inflow slot data straight from `arena_cur` via the pre-
/// resolved [`InflowFast`] range (with optional wrap) instead of
/// indirecting through a copied-out `&[u32]`.
///
/// **Accepted-as-unkillable mutants** (`cargo mutants 27.0.0`, verified
/// 2026-05-19) — all mirror the same-shaped unkillables already
/// documented on [`write_cell_into_next_arena`]:
///
/// - `> 0` → `>= 0` on `if post_outflow_len > 0` rope-seed guard:
///   with `>= 0` the seed becomes `Original { start: 0, end: 0 }` for
///   an empty source; the flatten loop's own `if seg_len > 0` guard
///   drops it without writing.
/// - `> 0` → `>= 0` on the two `if seg_len > 0` guards inside the
///   flatten loop (one per `MergeSegment` arm): the inner copy
///   degenerates to `dest[pos..pos].copy_from_slice(...&[..0])` when
///   `seg_len == 0` — observably a no-op. The guards are perf
///   early-exits, not correctness gates.
/// - `-` → `+` on `let seg_len = (end - start)` in the `Insert` arm:
///   only diverges from the original when `start == end > 0` (a
///   zero-length Insert with non-zero start), which the rope merge
///   never produces. The `if seg_len > 0` guard means the mutated
///   value gates the same set of cases as the original for any state
///   reachable from a real `outflow_phase_inplace` call; in the
///   unreachable edge case, `flatten_inflow_segment`'s downstream
///   `copy_from_slice` of a `start..end` range with `start == end`
///   is still a no-op.
#[allow(clippy::too_many_arguments)]
fn write_cell_into_next_arena_fast(
    cell: &mut Cell,
    arena_cur: &crate::world::arena::Arena,
    arena_next: &mut crate::world::arena::Arena,
    new_start: u32,
    new_len: u32,
    old_mem_start: u32,
    post_outflow_len: u32,
    entries: &[InflowFast],
) {
    // Origin-tag inheritance fires on the top-ranked entry only, and
    // only when its dominance ≥ 0.5. Matches `write_cell_into_next_arena`.
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

    // Pre-resolve each entry's slot range once so the flatten loop
    // doesn't re-hash any map. Empty `InflowFast` slots are unused.
    let zero_entry = InflowFast {
        head_start: 0,
        head_len: 0,
        wrap_start: 0,
        wrap_len: 0,
        dominance: 0.0,
        src_origin_tag: 0,
        dir_from_target: 0,
    };
    let mut entry_descriptors: [InflowFast; MERGE_MAX_ENTRIES] = [zero_entry; MERGE_MAX_ENTRIES];
    let mut slot_count: usize = 0;

    for entry in entries {
        if slot_count >= MERGE_MAX_ENTRIES {
            break;
        }
        let entry_total = entry.slot_count();
        if entry_total == 0 {
            continue;
        }

        let intrusion_depth = (entry.dominance * usize_to_f32(current_len)) as usize;
        let write_start_usize = current_len.saturating_sub(intrusion_depth);
        let write_start_u32 = u32::try_from(write_start_usize).unwrap_or(u32::MAX);

        let new_seg = MergeSegment::Insert {
            entry_idx: slot_count as u32,
            start: 0,
            end: entry_total,
        };
        rope_len = rope_insert(&mut rope, rope_len, write_start_u32, new_seg);
        entry_descriptors[slot_count] = *entry;
        slot_count += 1;
        current_len += entry_total as usize;

        let dir_idx = entry.dir_from_target as usize;
        cell.inflow[dir_idx] = cell.inflow[dir_idx].saturating_add(entry_total);
    }

    debug_assert_eq!(
        current_len, new_len as usize,
        "rope flatten size must match the bump-allocated new_len"
    );

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
                    let entry = entry_descriptors[entry_idx as usize];
                    flatten_inflow_segment(arena_cur, dest, &mut pos, &entry, start, end);
                }
            }
        }
    }
    debug_assert_eq!(pos, new_len as usize);

    cell.mem_start = new_start;
    cell.mem_len = new_len;

    // PC wrap mirrors `write_cell_into_next_arena`: fires only when at
    // least one inflow with non-empty slots was applied. Cells with
    // outflow but no inflow keep their pre-apply pc verbatim.
    if slot_count > 0 {
        if new_len == 0 {
            cell.pc = 0;
        } else {
            cell.pc %= new_len;
        }
    }
}

/// Copy a sub-range `[start, end)` of an [`InflowFast`]'s slots from
/// `arena_cur` into `dest[*pos..]`, advancing `pos`. Handles the
/// wraparound boundary at `head_len`: the segment lands wholly in the
/// head, wholly in the wrap, or straddles the boundary and emits two
/// memcpys. Wraparound only ever fires when the source's
/// `pointers[d] + rate` overflowed `mem_len`, so the straddle branch
/// is rare in practice.
fn flatten_inflow_segment(
    arena_cur: &crate::world::arena::Arena,
    dest: &mut [u32],
    pos: &mut usize,
    entry: &InflowFast,
    start: u32,
    end: u32,
) {
    debug_assert!(start < end);
    let seg_len = (end - start) as usize;

    if end <= entry.head_len {
        let src = arena_cur.slice(entry.head_start + start, end - start);
        dest[*pos..*pos + seg_len].copy_from_slice(src);
        *pos += seg_len;
    } else if start >= entry.head_len {
        let wrap_offset = start - entry.head_len;
        let src = arena_cur.slice(entry.wrap_start + wrap_offset, end - start);
        dest[*pos..*pos + seg_len].copy_from_slice(src);
        *pos += seg_len;
    } else {
        let head_part_len = (entry.head_len - start) as usize;
        let wrap_part_len = (end - entry.head_len) as usize;
        let src_head = arena_cur.slice(entry.head_start + start, entry.head_len - start);
        dest[*pos..*pos + head_part_len].copy_from_slice(src_head);
        *pos += head_part_len;
        let src_wrap = arena_cur.slice(entry.wrap_start, end - entry.head_len);
        dest[*pos..*pos + wrap_part_len].copy_from_slice(src_wrap);
        *pos += wrap_part_len;
    }
}

/// Reset transient per-tick state on every cell: pointer overrides and
/// active outflow buffers. Called after the outflow phase to clear the
/// programmer's per-tick instructions before the next tick starts.
pub fn end_of_tick(world: &mut SparseWorld) {
    for cell in world.cells.values_mut() {
        cell.end_of_tick();
    }
}

/// Density-coupled point mutation: flip random bits in cell memory.
///
/// The per-slot flip probability rises with local energy density (see
/// `docs/mechanics.md`). A flip changes a slot's *value*, never the
/// slot count, so the `energy == mem_len` invariant — and total energy —
/// is preserved exactly; only the *content* (program) drifts.
///
/// **Density coupling (saturating).** The per-slot flip probability for a
/// cell of energy `E` is
/// `p = mutation_strength · E / (E + mutation_half_density)` — a saturating
/// curve, not linear: ~0 for a tiny cell (a 1-slot program does nothing),
/// rising toward `mutation_strength` as density grows, reaching half at
/// `E = mutation_half_density`. With that half-density set high, only the
/// dense cores gravity builds become "mutagenic cauldrons" while dispersed
/// / player-scale cells stay gentle — gravity thus decides *where*
/// evolution happens. Energy is density (a cell is a unit volume), so this
/// couples on the cell's own energy: self-contained (no dependency on the
/// gravity-only `scratch_mass`). All ops are `+`/`*`/`/` (correctly
/// rounded) → reproducible native↔wasm. See `docs/mechanics.md`.
///
/// **Determinism.** One RNG stream per cell, keyed
/// `(world_seed, tick, coord, DENSITY_MUTATION_RNG_DOMAIN)` — a domain
/// disjoint from the rate / clamp streams. Slots are walked in fixed
/// index order, so each slot's draw is just its position in that single
/// stream; the slot index is never itself a domain (which would alias the
/// rate stream at index 0). Reproducible across runs and independent of
/// cell iteration order, since each cell mutates only its own slots.
///
/// **No-op when off.** `mutation_strength == 0.0` returns before seeding
/// any RNG, so the phase is byte-for-byte absent from the tick — existing
/// baselines stay valid with mutation off.
///
/// Runs sequentially: it needs `&mut world.arena` (a single shared
/// buffer), which can't be split across a parallel cell walk. Mutation is
/// cheap next to the outflow phase; a collect-then-apply parallel variant
/// is only worth it if profiling later says so.
//
// `float_cmp`: `mutation_strength == 0.0` is a deliberate exact off-switch.
#[allow(clippy::float_cmp)]
pub fn apply_density_coupled_mutation(world: &mut SparseWorld) {
    let strength = world.mutation_strength;
    if strength == 0.0 {
        return;
    }
    let half_density = world.mutation_half_density;
    let world_seed = world.world_seed;
    let tick = world.tick;
    // Disjoint field borrows: read each cell's range, mutate arena slots.
    let cells = &world.cells;
    let arena = &mut world.arena;
    for (coord, cell) in cells.iter() {
        let energy = cell.energy();
        if energy == 0 {
            continue;
        }
        // Saturating density coupling: p = strength · E / (E + K). `+`/`*`/
        // `/` are correctly rounded → portable; `E + K > 0` (E ≥ 1) so no
        // division by zero even at K = 0 (which gives p = strength).
        let e = f64::from(energy);
        let p_flip = strength * e / (e + half_density);
        let mut rng = Rng::for_cell_at_tick(world_seed, tick, *coord, DENSITY_MUTATION_RNG_DOMAIN);
        for slot in arena.slice_mut(cell.mem_start, cell.mem_len).iter_mut() {
            if rng.next_f64() < p_flip {
                let bit = rng.next_u32() % 32;
                *slot ^= 1u32 << bit;
            }
        }
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
    // `cpu_phase` standalone (e.g. tests), rebuild it here so the read
    // below sees fresh data — `refresh_neighbor_energies` reads the
    // blocked grid, so build that first.
    if world.scratch_neighbor_energies.len() != world.cells.len() {
        refresh_energy_blocks(world);
        refresh_neighbor_energies(world);
    }

    let k_safe = k.max(1);
    let snapshot = &world.scratch_neighbor_energies;

    // Pair every live cell with its own disjoint slice of the arena so
    // the interpreter can run cells in parallel. Cell ranges never
    // overlap (bump allocation), so sorting by `mem_start` and chaining
    // `split_at_mut` carves the backing storage safely without `unsafe`.
    // Cells are fully independent inside the CPU phase — instructions
    // read only the cell's own memory/registers plus the immutable
    // neighbor snapshot, and the VM draws no RNG — so any execution
    // order (parallel included) is bit-identical to the sequential walk.
    let mut jobs: Vec<(Coord, &mut Cell)> = world
        .cells
        .iter_mut()
        // Accepted-as-unkillable mutant (`cargo mutants`): `>` → `>=`.
        // Empty cells are a no-op in the interpreter anyway (`mem.len()
        // == 0` early-returns), so including them only wastes work —
        // behaviorally equivalent, no test can distinguish it.
        .filter(|(_, cell)| cell.mem_len > 0)
        .map(|(coord, cell)| (*coord, cell))
        .collect();
    // Slot order usually matches arena order (`apply_outflow` lays the
    // ranges out in iteration order), so this is typically a single
    // no-swap verification pass.
    jobs.sort_unstable_by_key(|(_, cell)| cell.mem_start);

    let mut work: Vec<(Coord, &mut Cell, &mut [u32])> = Vec::with_capacity(jobs.len());
    let mut rest: &mut [u32] = world.arena.backing_mut();
    let mut consumed: usize = 0;
    for (coord, cell) in jobs {
        let start = cell.mem_start as usize;
        let len = cell.mem_len as usize;
        // `start >= consumed` holds because ranges are disjoint and
        // sorted; a violation means overlapping cells and panics here
        // (subtraction underflow / split OOB) rather than corrupting.
        let (_, tail) = rest.split_at_mut(start - consumed);
        let (mine, tail) = tail.split_at_mut(len);
        rest = tail;
        consumed = start + len;
        work.push((coord, cell, mine));
    }

    let run = |coord: &Coord, cell: &mut Cell, mem: &mut [u32]| {
        let neighbors = snapshot
            .get(coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);
        let budget = cell.energy() / k_safe;
        for _ in 0..budget {
            crate::vm::execute_instruction_mem(cell, mem, &neighbors);
        }
    };

    // Per-item work here is `O(cell energy)` — orders heavier than the
    // per-entry map walks `PAR_THRESHOLD` calibrates for — so a much
    // smaller cutoff pays; below it (tiny test worlds) rayon dispatch
    // overhead isn't worth it.
    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
    {
        const CPU_PAR_THRESHOLD: usize = 32;
        // Accepted-as-unkillable mutants (`cargo mutants`): the `<`
        // comparison swaps. Both branches are bit-identical by design
        // (cells are independent inside the CPU phase), so which one runs
        // is unobservable to any functional test — the threshold only
        // trades rayon dispatch overhead against parallelism.
        if work.len() < CPU_PAR_THRESHOLD {
            for (coord, cell, mem) in &mut work {
                run(coord, cell, mem);
            }
        } else {
            work.into_par_iter()
                .for_each(|(coord, cell, mem)| run(&coord, cell, mem));
        }
    }
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
    for (coord, cell, mem) in &mut work {
        run(coord, cell, mem);
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
/// 3. [`outflow_phase_inplace`] — sub-tick pointer reflow with combined
///    rates, emission across faces, alloc-on-write into void neighbors.
///    Fuses the legacy `lay_out_pointers` + `collect_outflow` +
///    `apply_outflow` chain into one pass that skips the
///    `Outflow Vec<u32>` intermediate (see the function's own doc for
///    the rationale and the bit-parity contract).
/// 4. [`end_of_tick`] — reset overrides and active outflow.
/// 5. [`SparseWorld::gc_empty`] — drop cells whose memory shrank to zero.
/// 6. Increment `world.tick`.
///
/// Energy is conserved across the cycle. The legacy
/// `lay_out_pointers` / `collect_outflow` / `apply_outflow` helpers
/// are still public for tests and [`step_diffusion`]; they exercise
/// the reference path that the bit-parity baseline pins.
pub fn step(world: &mut SparseWorld, coeff: f64, k: u32) {
    initialize(world, coeff);
    cpu_phase(world, k);
    outflow_phase_inplace(world);
    apply_density_coupled_mutation(world);
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

#[cfg(test)]
#[allow(clippy::float_cmp)] // pins compare exactly-representable f64 results
mod gravity_unit_tests {
    //! Crate-private pins for the gravity/pressure helpers. These reach
    //! into `pressure_pi`/`gamma_pow` (private fns) and
    //! `world.scratch_mass` (`pub(crate)`), which the external
    //! integration tests in `tests/tick_gravity.rs` cannot touch.

    use super::{compute_natural_rates, gamma_pow, gravity_stencil, pressure_pi};
    use crate::{Coord, SparseWorld};

    // ----- gravity_stencil: the 1/r kernel offsets ---------------------

    #[test]
    fn stencil_radius_one_is_the_six_unit_faces() {
        let s = gravity_stencil(1);
        assert_eq!(s.len(), 6, "R=1 reaches only the six |d|=1 faces");
        assert!(
            s.iter().all(|&(_, w)| w == 1.0),
            "unit faces have weight 1/1"
        );
    }

    #[test]
    fn stencil_radius_two_has_the_full_shell_with_inverse_distance_weights() {
        let s = gravity_stencil(2);
        // |d|²∈{1,2,3,4}: 6 faces + 12 edges + 8 corners + 6 axis-2 = 32.
        assert_eq!(s.len(), 32);
        // A face (|d|=1 → w=1), a face-diagonal (|d|=√2 → w=1/√2), and an
        // axis-2 offset (|d|=2 → w=1/2) carry the inverse-distance weight.
        let weight_of = |dx, dy, dz| {
            s.iter()
                .find(|&&(o, _)| o == Coord::new(dx, dy, dz))
                .map(|&(_, w)| w)
                .unwrap()
        };
        assert_eq!(weight_of(1, 0, 0), 1.0);
        assert_eq!(weight_of(1, 1, 0), 1.0 / 2.0_f64.sqrt());
        assert_eq!(weight_of(2, 0, 0), 0.5);
    }

    #[test]
    fn stencil_is_empty_for_nonpositive_radius() {
        assert!(gravity_stencil(0).is_empty());
        assert!(gravity_stencil(-3).is_empty());
    }

    // ----- gamma_pow: the portable exponent chains --------------------

    #[test]
    fn gamma_pow_evaluates_each_supported_index_exactly() {
        let x = 4.0_f64; // sqrt(4) = 2, all results are exact integers
        assert_eq!(gamma_pow(x, 1.0), 4.0);
        assert_eq!(gamma_pow(x, 1.5), 8.0); // 4 * 2
        assert_eq!(gamma_pow(x, 2.0), 16.0); // 4 * 4
        assert_eq!(gamma_pow(x, 2.5), 32.0); // 4 * 4 * 2
        assert_eq!(gamma_pow(x, 3.0), 64.0); // 4 * 4 * 4
    }

    // ----- pressure_pi: Π(E) = pressure · eref · (E/eref)^γ -----------

    #[test]
    fn pressure_pi_is_zero_when_pressure_off() {
        // Off even with a degenerate eref = 0 (no stray div-by-zero NaN).
        assert_eq!(pressure_pi(123, 0.0, 0.0, 2.0), 0.0);
    }

    #[test]
    fn pressure_pi_matches_the_closed_form() {
        // eref = 1: Π = pressure * (E^2). E=3, p=2 → 2 * 9 = 18.
        assert_eq!(pressure_pi(3, 2.0, 1.0, 2.0), 18.0);
        // eref = 2: x = E/eref = 2, Π = p*eref*x^2 = 2*2*4 = 16.
        assert_eq!(pressure_pi(4, 2.0, 2.0, 2.0), 16.0);
        // γ = 1 is linear in x: x = 5, Π = p*eref*x = 3*1*5 = 15.
        assert_eq!(pressure_pi(5, 3.0, 1.0, 1.0), 15.0);
    }

    // ----- scratch_mass: M(c) = alpha · Σ neighbor energies -----------

    #[test]
    fn scratch_mass_is_alpha_times_the_neighbor_sum() {
        // C at origin with two live neighbors (200 + 100) and four void
        // faces → Σ = 300, so M_C = alpha * 300.
        let mut w = SparseWorld::new(7);
        w.gravity = 0.1; // non-zero so compute_natural_rates builds masses
        w.gravity_alpha = 0.05;
        w.insert_with_memory(Coord::new(-1, 0, 0), &[1; 200]);
        w.insert_with_memory(Coord::new(0, 0, 0), &[1; 100]);
        w.insert_with_memory(Coord::new(1, 0, 0), &[1; 100]);
        compute_natural_rates(&mut w, 0.15);

        let m_c = w.scratch_mass.get(&Coord::new(0, 0, 0)).copied().unwrap();
        assert_eq!(m_c, 0.05 * 300.0);
        // The far cell H has only C as a neighbor → M_H = alpha * 100.
        let m_h = w.scratch_mass.get(&Coord::new(-1, 0, 0)).copied().unwrap();
        assert_eq!(m_h, 0.05 * 100.0);
    }

    #[test]
    fn scratch_mass_is_not_built_on_the_gravity_off_path() {
        // gravity == 0 → fast path → scratch_mass stays empty (zero cost).
        let mut w = SparseWorld::big_bang(7, 256);
        compute_natural_rates(&mut w, 0.15);
        assert!(w.scratch_mass.is_empty());
    }
}

#[cfg(test)]
mod mutation_unit_tests {
    //! Crate-private pin for the mutation RNG domain reservation.

    use crate::apportion::{
        COMBINED_CLAMPED_RNG_DOMAIN, DENSITY_MUTATION_RNG_DOMAIN, PROPORTIONAL_CLAMP_RNG_DOMAIN,
    };
    use crate::{Coord, Rng};

    #[test]
    fn mutation_domain_is_disjoint_from_the_rate_and_clamp_domains() {
        // The mutation stream (domain 3) must not alias the rate (0),
        // combined-clamp (1) or proportional-clamp (2) streams for the
        // same (seed, tick, coord) — otherwise a slot-0 flip decision
        // would correlate with that cell's diffusion draws.
        const RATE_DOMAIN: u32 = 0;
        assert_ne!(DENSITY_MUTATION_RNG_DOMAIN, RATE_DOMAIN);
        assert_ne!(DENSITY_MUTATION_RNG_DOMAIN, COMBINED_CLAMPED_RNG_DOMAIN);
        assert_ne!(DENSITY_MUTATION_RNG_DOMAIN, PROPORTIONAL_CLAMP_RNG_DOMAIN);

        let (seed, tick, coord) = (0x1234_5678_u64, 9_u64, Coord::new(3, -2, 5));
        let mut mutation = Rng::for_cell_at_tick(seed, tick, coord, DENSITY_MUTATION_RNG_DOMAIN);
        let mut rate = Rng::for_cell_at_tick(seed, tick, coord, RATE_DOMAIN);
        // First draws of the two streams must differ — the domain salt
        // genuinely decorrelates them, not just the constant value.
        assert_ne!(mutation.next_u32(), rate.next_u32());
    }
}

#[cfg(test)]
mod mass_gather_parity_tests {
    //! Pins `refresh_mass` against a naive reference gather: the
    //! blocked-grid walk (and any future optimization of it) must be
    //! **bit-identical** to summing `E(c+d) · w(d)` per-offset in stencil
    //! order via direct `cells.get` — exact `f64 ==` on every key, across
    //! block boundaries and negative coords. This is the regression net
    //! that lets the mass path be restructured safely.
    use super::{gravity_stencil, refresh_energy_blocks, refresh_mass};
    use crate::{Coord, Rng, SparseWorld};

    /// A deterministic multi-blob world spanning several blocks,
    /// including negative coords and block-boundary straddles.
    fn scattered_world() -> SparseWorld {
        let mut world = SparseWorld::with_capacity(99, 100_000);
        let mut rng = Rng::new(0xC0FF_EE00);
        let mut draw = |modulo: u32| i32::try_from(rng.next_u32() % modulo).expect("small");
        for fill in 0..200u32 {
            // Coords in [-20, 20)³ — spans ~5 blocks per axis.
            let coord = Coord::new(draw(40) - 20, draw(40) - 20, draw(40) - 20);
            let len = (draw(300) + 1).unsigned_abs() as usize;
            let slots = vec![fill; len];
            world.insert_with_memory(coord, &slots);
        }
        world.rebuild_indices_if_dirty();
        world
    }

    /// Reference: the per-offset gather straight off the cells map, in
    /// stencil order.
    fn naive_mass(world: &SparseWorld, coord: Coord, alpha: f64, radius: i32) -> f64 {
        let mut acc = 0.0_f64;
        for (off, weight) in gravity_stencil(radius) {
            let nc = Coord::new(coord.x + off.x, coord.y + off.y, coord.z + off.z);
            let energy = world.get(nc).map_or(0, crate::Cell::energy);
            acc += f64::from(energy) * weight;
        }
        alpha * acc
    }

    // `float_cmp`: exact `==` is the whole point — the walk must be
    // bit-identical to the reference, not merely close.
    #[allow(clippy::float_cmp)]
    #[test]
    fn blocked_walk_is_bit_identical_to_naive_gather() {
        let mut world = scattered_world();
        let alpha = 0.05;
        for radius in [1, 2, 4, 8] {
            refresh_energy_blocks(&mut world);
            refresh_mass(&mut world, alpha, radius);
            assert!(!world.scratch_mass.is_empty());
            for (coord, &mass) in &world.scratch_mass {
                let expected = naive_mass(&world, *coord, alpha, radius);
                assert!(
                    mass == expected,
                    "mass mismatch at {coord:?} (radius {radius}): got {mass} vs naive {expected}"
                );
            }
        }
    }
}

#[cfg(test)]
mod block_addr_tests {
    //! Pins the gravity block-grid addressing. The mapping is internal
    //! (any bijection coord→(block, local) yields identical mass, since
    //! build and gather share it), so a system-level test can't catch a
    //! re-addressing change — these unit pins do. They also guard the
    //! one re-addressing that *isn't* harmless: a packing that overlaps
    //! the z/y/x bit fields (e.g. `2*BITS` → `2+BITS`) stops being a
    //! bijection and would alias distinct cells onto one slot.
    use super::{block_coord, block_local, GRAV_BLOCK_VOL};
    use crate::Coord;

    #[test]
    fn block_coord_floors_toward_negative() {
        assert_eq!(block_coord(0, 0, 0), Coord::new(0, 0, 0));
        assert_eq!(block_coord(7, 7, 7), Coord::new(0, 0, 0));
        assert_eq!(block_coord(8, 16, 0), Coord::new(1, 2, 0));
        // Arithmetic shift floors negatives: -1 >> 3 == -1, -8 >> 3 == -1.
        assert_eq!(block_coord(-1, -8, 0), Coord::new(-1, -1, 0));
    }

    #[test]
    fn block_local_packs_disjoint_fields() {
        assert_eq!(block_local(0, 0, 0), 0);
        assert_eq!(block_local(1, 0, 0), 1); // x → bits 0..3
        assert_eq!(block_local(0, 1, 0), 8); // y → bits 3..6
        assert_eq!(block_local(0, 0, 1), 64); // z → bits 6..9 (kills `*`→`+`)
        assert_eq!(block_local(7, 7, 7), GRAV_BLOCK_VOL - 1);
        // Negatives wrap into the block via `& MASK`.
        assert_eq!(block_local(-1, -1, -1), GRAV_BLOCK_VOL - 1);
    }

    #[test]
    fn local_index_is_a_bijection_over_a_block() {
        // Every (x, y, z) in one block maps to a distinct slot in
        // `0..VOL`. Catches any field-overlap re-packing.
        let mut seen = [false; GRAV_BLOCK_VOL];
        for z in 0..8 {
            for y in 0..8 {
                for x in 0..8 {
                    let i = block_local(x, y, z);
                    assert!(!seen[i], "collision at ({x},{y},{z}) → {i}");
                    seen[i] = true;
                }
            }
        }
        assert!(seen.iter().all(|&b| b), "indices must cover 0..VOL");
    }
}

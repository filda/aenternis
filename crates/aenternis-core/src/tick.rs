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

use std::collections::BTreeMap;

use crate::cell::proportional_clamp;
use crate::{Coord, Direction, Rng, SparseWorld};

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
pub fn compute_natural_rates(world: &mut SparseWorld, coeff: f32) {
    // Phase 1: snapshot energies. Immutable borrow of `world.cells`.
    let snapshot: BTreeMap<Coord, u32> = world
        .cells
        .iter()
        .map(|(coord, cell)| (*coord, cell.energy()))
        .collect();

    // Pull Copy fields off the world so the mutable iteration below
    // doesn't conflict with shared borrows.
    let world_seed = world.world_seed;
    let tick = world.tick;

    // Phase 2: compute rates per cell. Mutable borrow of `world.cells`.
    for (coord, cell) in &mut world.cells {
        let my_energy = cell.energy();
        if my_energy == 0 {
            cell.rates = [0; Direction::COUNT];
            continue;
        }

        let mut rng = Rng::for_cell_at_tick(world_seed, tick, *coord);

        for &d in &Direction::ALL {
            let neighbor_coord = coord.neighbor(d);
            let neighbor_energy = snapshot.get(&neighbor_coord).copied().unwrap_or(0);
            let rate = if my_energy > neighbor_energy {
                let delta = my_energy - neighbor_energy;
                rng.stochastic_floor(delta_to_f32(delta) * coeff)
            } else {
                0
            };
            cell.rates[d.index()] = rate;
        }

        if cell.total_rate() > my_energy {
            proportional_clamp(&mut cell.rates, my_energy);
        }
    }
}

/// `u32 → f32` cast used to compute the rate scaling factor.
///
/// In practice cell energies stay well below `2^24`, where `f32` is exact;
/// the cast is therefore lossless for any realistic world. The
/// `clippy::cast_precision_loss` lint can't see that constraint, so we
/// localize the suppression to one tiny helper rather than scattering it.
///
/// Float `as` casts in `const fn` are only stable from Rust 1.79; the
/// workspace MSRV is 1.78, so we suppress `missing_const_for_fn` here
/// rather than bumping the toolchain just for one helper.
#[allow(clippy::cast_precision_loss, clippy::missing_const_for_fn)]
#[inline]
fn delta_to_f32(delta: u32) -> f32 {
    delta as f32
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
pub type Outflow = BTreeMap<Coord, [Vec<u32>; Direction::COUNT]>;

/// Collect the outflow snapshot for every cell in `world`.
///
/// For each cell `C` and each direction `d`:
///
/// - `rate = C.rates[d]`
/// - `ptr = C.pointers[d]`
/// - the slice `C.memory[ptr .. ptr+rate]` (modular wrap on memory length)
///   is copied into `outflow[&C.coord][d.index()]`.
///
/// **Pre-condition:** `total_rate(C) ≤ C.energy()` for every cell — i.e.
/// [`compute_natural_rates`] has been called and proportionally clamped
/// rates as needed. The function is tolerant if the invariant is violated
/// (modular wrap will read slots more than once), but the result is then
/// not physically meaningful.
///
/// **Determinism:** iterates the world in `BTreeMap` order; the result is
/// reproducible for a given `(rates, pointers, memory)` triple per cell.
///
/// **Allocation:** allocates one `Vec<u32>` per direction per cell, even
/// when the rate is zero. That's six small allocations per cell per tick,
/// which is fine for the prototype phase. A pooled-buffer variant can come
/// later if profiling shows it's worth the complexity.
#[must_use]
pub fn collect_outflow(world: &SparseWorld) -> Outflow {
    let mut outflow = Outflow::new();
    for (coord, cell) in world {
        let mem_size = cell.memory.len();
        let mut per_direction: [Vec<u32>; Direction::COUNT] = Default::default();
        if mem_size > 0 {
            for &d in &Direction::ALL {
                let rate = cell.rates[d.index()] as usize;
                let ptr = cell.pointers[d.index()] as usize;
                let mut buf = Vec::with_capacity(rate);
                for k in 0..rate {
                    buf.push(cell.memory[(ptr + k) % mem_size]);
                }
                per_direction[d.index()] = buf;
            }
        }
        outflow.insert(*coord, per_direction);
    }
    outflow
}

/// Lay out per-direction pointers for every cell in the world.
///
/// Uses each cell's combined rate (`rates + active_outflow`) as the
/// per-direction consumption budget. Honors any `pointer_override`
/// flags set by a CPU-phase `setp` / `setpv` instruction this tick.
///
/// This is the sub-tick reflow step from `docs/mechanics.md`. In the
/// diffusion-only phase 1 there is no CPU phase, so `active_outflow`
/// is always all-zero and the combined rate equals the natural rate —
/// but the function is written for the general case so it doesn't need
/// rewriting once the VM lands.
pub fn lay_out_pointers(world: &mut SparseWorld) {
    for cell in world.cells.values_mut() {
        let combined: [u32; Direction::COUNT] =
            std::array::from_fn(|i| cell.rates[i].saturating_add(cell.active_outflow[i]));
        cell.lay_out_pointers(&combined);
    }
}

/// Apply an [`Outflow`] snapshot to the world.
///
/// Shrinks each source by its total outgoing slot count, then appends
/// per-direction inflows into neighbors. Void neighbors are alloc-on-
/// written via [`SparseWorld::get_or_alloc`].
///
/// The two phases run sequentially (all shrinks, then all appends), not
/// per-cell interleaved. Doing it the other way would let a freshly
/// shrunk cell receive inflows on top of its already-reduced memory,
/// which is what we want; but interleaving per-cell would make the
/// behavior depend on iteration order, which `BTreeMap` keeps stable
/// but that's not a contract we want to lean on for physics.
///
/// Per-direction order on the inflow side is the canonical
/// `[xp, xn, yp, yn, zp, zn]` — same load-bearing invariant as
/// elsewhere. A target cell receiving from multiple neighbors gets all
/// inflows appended in this fixed direction order.
///
/// **Conservation:** total slots before == total slots after, modulo
/// the cap behavior in [`Cell::append_slots`] (no cap is passed here,
/// so memory grows freely). Energy is therefore conserved.
pub fn apply_outflow(world: &mut SparseWorld, outflow: &Outflow) {
    // Phase 1: shrink each source by its total outgoing slot count.
    for (coord, per_dir) in outflow {
        let total: u32 = per_dir
            .iter()
            .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX))
            .fold(0u32, u32::saturating_add);
        if let Some(cell) = world.cells.get_mut(coord) {
            cell.shrink_from_end(total);
        }
    }

    // Phase 2: append per-direction inflows into neighbors. Allocate
    // void neighbors as empty cells before appending.
    for (source_coord, per_dir) in outflow {
        for &d in &Direction::ALL {
            let inflows = &per_dir[d.index()];
            if inflows.is_empty() {
                continue;
            }
            let target_coord = source_coord.neighbor(d);
            let target = world.get_or_alloc(target_coord);
            target.append_slots(inflows, None);
        }
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
/// Cells are visited in `BTreeMap` (canonical) order. Each cell's
/// budget runs to completion before the next cell starts.
pub fn cpu_phase(world: &mut SparseWorld, k: u32) {
    // Phase 1: snapshot neighbor energies for every existing cell.
    let coords: Vec<Coord> = world.cells.keys().copied().collect();
    let mut neighbor_lookup: BTreeMap<Coord, [u32; Direction::COUNT]> = BTreeMap::new();
    for coord in &coords {
        let mut energies = [0u32; Direction::COUNT];
        for &d in &Direction::ALL {
            energies[d.index()] = world.neighbor_energy(*coord, d);
        }
        neighbor_lookup.insert(*coord, energies);
    }

    let k_safe = k.max(1);

    // Phase 2: run each cell's instruction budget against the snapshot.
    for (coord, cell) in &mut world.cells {
        let neighbors = neighbor_lookup
            .get(coord)
            .copied()
            .unwrap_or([0; Direction::COUNT]);
        let budget = cell.energy() / k_safe;
        for _ in 0..budget {
            crate::vm::execute_instruction(cell, &neighbors);
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
pub fn step(world: &mut SparseWorld, coeff: f32, k: u32) {
    initialize(world, coeff);
    cpu_phase(world, k);
    lay_out_pointers(world);
    let outflow = collect_outflow(world);
    apply_outflow(world, &outflow);
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
pub fn initialize(world: &mut SparseWorld, coeff: f32) {
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
pub fn step_diffusion(world: &mut SparseWorld, coeff: f32) {
    initialize(world, coeff);
    let outflow = collect_outflow(world);
    apply_outflow(world, &outflow);
    end_of_tick(world);
    world.gc_empty();
    world.tick = world.tick.saturating_add(1);
}

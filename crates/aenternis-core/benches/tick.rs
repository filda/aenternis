//! Criterion benchmarks for [`tick::step`].
//!
//! Two scenarios per energy budget:
//!
//! - **cold** — fresh `big_bang`, single tick. Captures the all-void
//!   diffusion case where every cell's neighbors are empty space.
//! - **warm** — `big_bang` followed by [`WARMUP_TICKS`] tick
//!   evolutions, then one measured step on top of the resulting
//!   sparse-cluster state. More representative of long-running
//!   simulation behaviour where dominance / intrusion fires.
//!
//! The warm world is built once per benchmark and cloned for each
//! measurement to keep iteration cost dominated by the work under
//! test rather than by setup. Setup time is reported separately by
//! criterion's `iter_batched` machinery.
//!
//! Run with `cargo bench -p aenternis-core` from the workspace root.
//! HTML reports land under `target/criterion/`.

// `criterion_group!` and `criterion_main!` expand to module-level items
// the workspace's `missing_docs = "warn"` lint can't see through.
#![allow(missing_docs)]

use aenternis_core::{tick, Cell, Coord, Rng, SparseWorld};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

/// Diffusion coefficient — typical mid-range value used in the prototypes.
const COEFF: f64 = 0.20;

/// CPU compute constant — `instructions_per_cell = floor(energy / K)`.
const K: u32 = 1;

/// Fixed seed so successive runs are comparable across machines.
const SEED: u64 = 42;

/// Energy budgets for the big-bang. Each scales the resulting cell
/// count after a few ticks roughly linearly.
const ENERGY_BUDGETS: &[u32] = &[100, 1_000, 10_000];

/// Energy budget for the large-scale group. Kept separate so the
/// fast-iteration loop above stays under a minute, and so we can tune
/// criterion's sample count for the slower runs without affecting the
/// small ones.
const LARGE_ENERGY: u32 = 100_000;

/// Warmup ticks for the "warm" scenario — enough to spread out into a
/// realistic sparse cluster without taking minutes to build.
const WARMUP_TICKS: u32 = 10;

fn bench_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_step/cold");
    for &energy in ENERGY_BUDGETS {
        group.bench_with_input(
            BenchmarkId::from_parameter(energy),
            &energy,
            |b, &energy| {
                b.iter_batched(
                    || SparseWorld::big_bang(SEED, energy),
                    |mut w| {
                        tick::step(&mut w, COEFF, K);
                        black_box(&w);
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_warm(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_step/warm");
    for &energy in ENERGY_BUDGETS {
        // Build the warmed-up state once, then clone per measurement.
        let mut warmed = SparseWorld::big_bang(SEED, energy);
        for _ in 0..WARMUP_TICKS {
            tick::step(&mut warmed, COEFF, K);
        }
        group.bench_with_input(BenchmarkId::from_parameter(energy), &warmed, |b, warmed| {
            b.iter_batched(
                || warmed.clone(),
                |mut w| {
                    tick::step(&mut w, COEFF, K);
                    black_box(&w);
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// Single large-scale `warm` measurement at 100k energy. Built behind a
/// dedicated benchmark group so we can lower `sample_size` (each step
/// is an order of magnitude slower than the small-world cases). The
/// world after warmup is order-of-magnitude tens of thousands of cells
/// — the regime where parallelism / alloc-pool optimizations have
/// room to pay back their fixed costs.
fn bench_warm_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_step/warm_large");
    group.sample_size(20);

    let mut warmed = SparseWorld::big_bang(SEED, LARGE_ENERGY);
    for _ in 0..WARMUP_TICKS {
        tick::step(&mut warmed, COEFF, K);
    }
    let cell_count = warmed.len();

    group.bench_with_input(
        BenchmarkId::from_parameter(format!("{LARGE_ENERGY}_e_{cell_count}_cells")),
        &warmed,
        |b, warmed| {
            b.iter_batched(
                || warmed.clone(),
                |mut w| {
                    tick::step(&mut w, COEFF, K);
                    black_box(&w);
                },
                BatchSize::LargeInput,
            );
        },
    );
    group.finish();
}

/// Huge-scale measurements at energies where `collect_outflow_into` and
/// the parallel hot paths dominate cost. Sample size is small because
/// each step is hundreds of milliseconds; the goal is a stable median,
/// not a tight CI.
fn bench_warm_huge(c: &mut Criterion) {
    const HUGE_ENERGIES: &[u32] = &[500_000, 1_000_000];

    let mut group = c.benchmark_group("tick_step/warm_huge");
    group.sample_size(10);

    for &energy in HUGE_ENERGIES {
        let mut warmed = SparseWorld::big_bang(SEED, energy);
        for _ in 0..WARMUP_TICKS {
            tick::step(&mut warmed, COEFF, K);
        }
        let cell_count = warmed.len();

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{energy}_e_{cell_count}_cells")),
            &warmed,
            |b, warmed| {
                b.iter_batched(
                    || warmed.clone(),
                    |mut w| {
                        tick::step(&mut w, COEFF, K);
                        black_box(&w);
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

/// Per-cell energy budget for the dense-grid scenarios. Each cell
/// runs this many instructions in the CPU phase, so the value sets
/// per-cell work without blowing up wallclock at 30k+ cells.
const DENSE_CELL_ENERGY: u32 = 32;

/// Build a cubic dense grid of `side^3` cells, every coord in
/// `[-side/2, side/2)^3` allocated with `DENSE_CELL_ENERGY` slots of
/// PRNG-derived memory.
///
/// The big-bang+warmup scenarios above end up with at most ~800 cells
/// even at 1M energy, because diffusion at `COEFF = 0.20` collapses
/// the world long before the cell count can grow. That keeps every
/// existing bench below the [`par_or_seq_iter_mut`] threshold (8 192)
/// and never exercises the parallel path. This helper sidesteps that
/// dynamic by populating the world directly, so the resulting cell
/// count is a guaranteed `side^3` regardless of diffusion behaviour.
fn dense_grid_world(seed: u64, side: i32, cell_energy: u32) -> SparseWorld {
    let half = side / 2;
    let mut world = SparseWorld::new(seed);
    // Memory-fill PRNG: keyed off the low 32 bits of the world seed.
    // Distinct from the per-cell-at-tick RNG that `tick::step` builds
    // from `(world_seed, tick, coord)` — we only need any deterministic
    // stream of `u32`s to seed each cell's memory.
    let mut rng = Rng::new(seed as u32);
    for x in -half..(side - half) {
        for y in -half..(side - half) {
            for z in -half..(side - half) {
                let mut memory = Vec::with_capacity(cell_energy as usize);
                for _ in 0..cell_energy {
                    memory.push(rng.next_u32());
                }
                world.insert(Coord::new(x, y, z), Cell::with_memory(memory));
            }
        }
    }
    world
}

/// Dense cubic-grid benchmarks. Unlike the `big_bang`-based scenarios
/// above, the world is constructed cell-by-cell so the per-tick cell
/// count stays well above the [`par_or_seq_iter_mut`] threshold —
/// `side = 22` gives 10 648 cells (just over), `side = 32` gives
/// 32 768 cells (~4×). The parallel path is what's actually being
/// measured here.
fn bench_dense_grid(c: &mut Criterion) {
    const SIDES: &[i32] = &[22, 32];

    let mut group = c.benchmark_group("tick_step/dense_grid");
    group.sample_size(20);

    for &side in SIDES {
        let world = dense_grid_world(SEED, side, DENSE_CELL_ENERGY);
        let cell_count = world.len();

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("side_{side}_cells_{cell_count}")),
            &world,
            |b, w| {
                b.iter_batched(
                    || w.clone(),
                    |mut w| {
                        tick::step(&mut w, COEFF, K);
                        black_box(&w);
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_cold,
    bench_warm,
    bench_warm_large,
    bench_warm_huge,
    bench_dense_grid
);
criterion_main!(benches);

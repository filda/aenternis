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

use aenternis_core::{tick, SparseWorld};
use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion,
};

/// Diffusion coefficient — typical mid-range value used in the prototypes.
const COEFF: f64 = 0.20;

/// CPU compute constant — `instructions_per_cell = floor(energy / K)`.
const K: u32 = 1;

/// Fixed seed so successive runs are comparable across machines.
const SEED: u64 = 42;

/// Energy budgets for the big-bang. Each scales the resulting cell
/// count after a few ticks roughly linearly.
const ENERGY_BUDGETS: &[u32] = &[100, 1_000, 10_000];

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
        group.bench_with_input(
            BenchmarkId::from_parameter(energy),
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

criterion_group!(benches, bench_cold, bench_warm);
criterion_main!(benches);

//! Standalone profiling target for `tick::step` — meant to be run under
//! `samply` or another sampling profiler. Mirrors the criterion `warm/10000`
//! scenario but in a long-running loop so the profiler accumulates enough
//! samples for the small per-tick cost to surface above noise.
//!
//! Build and profile from the workspace root:
//!
//! ```sh
//! cargo build --release --example profile_tick
//! samply record target/release/examples/profile_tick
//! ```

use aenternis_core::{tick, SparseWorld};

const COEFF: f64 = 0.20;
const K: u32 = 1;
const SEED: u64 = 42;
const ENERGY: u32 = 10_000;
const WARMUP_TICKS: u32 = 10;
const MEASURED_TICKS: u32 = 5_000;

fn main() {
    let mut w = SparseWorld::big_bang(SEED, ENERGY);
    for _ in 0..WARMUP_TICKS {
        tick::step(&mut w, COEFF, K);
    }

    println!(
        "after warmup: {} cells, total_energy = {}",
        w.len(),
        w.total_energy(),
    );

    let started = std::time::Instant::now();
    for _ in 0..MEASURED_TICKS {
        tick::step(&mut w, COEFF, K);
    }
    let elapsed = started.elapsed();

    let final_cells = w.len();
    let total_energy = w.total_energy();
    println!(
        "{MEASURED_TICKS} ticks in {:.2?} ({:.0} ticks/sec, {} cells, total_energy = {})",
        elapsed,
        f64::from(MEASURED_TICKS) / elapsed.as_secs_f64(),
        final_cells,
        total_energy,
    );

    // Touch the world post-loop so the optimizer can't elide the work.
    std::hint::black_box(&w);
}

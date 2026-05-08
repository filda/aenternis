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

/// Reads `energy` and `ticks` from env (`ENERGY`, `TICKS`); falls back
/// to the small-world defaults the criterion `warm/10000` scenario uses.
/// Use larger numbers when profiling at scale, e.g.
/// `ENERGY=100000 TICKS=2000 cargo run --profile profiling --example profile_tick`.
fn main() {
    let energy: u32 = std::env::var("ENERGY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let total_ticks: u32 = std::env::var("TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_010);
    let warmup: u32 = (total_ticks / 1000).max(10).min(total_ticks);
    let measured = total_ticks - warmup;

    let mut w = SparseWorld::big_bang(SEED, energy);
    for _ in 0..warmup {
        tick::step(&mut w, COEFF, K);
    }
    println!(
        "after warmup ({warmup} ticks): {} cells, total_energy = {}",
        w.len(),
        w.total_energy(),
    );

    let started = std::time::Instant::now();
    for _ in 0..measured {
        tick::step(&mut w, COEFF, K);
    }
    let elapsed = started.elapsed();

    println!(
        "{measured} measured ticks in {:.2?} ({:.0} ticks/sec, {} cells, total_energy = {}, {:.2} µs/tick)",
        elapsed,
        f64::from(measured) / elapsed.as_secs_f64(),
        w.len(),
        w.total_energy(),
        (elapsed.as_secs_f64() / f64::from(measured)) * 1e6,
    );

    // Touch the world post-loop so the optimizer can't elide the work.
    std::hint::black_box(&w);
}

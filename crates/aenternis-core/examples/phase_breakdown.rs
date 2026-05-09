//! Per-phase timing breakdown for `tick::step`.
//!
//! Splits one tick into its individual phases and accumulates wallclock
//! per phase across many ticks, so we can see which phase dominates at a
//! given world size.

use std::time::{Duration, Instant};

use aenternis_core::{tick, SparseWorld};

const COEFF: f64 = 0.20;
const K: u32 = 1;
const SEED: u64 = 42;

fn main() {
    let energy: u32 = std::env::var("ENERGY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let total_ticks: u32 = std::env::var("TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    let warmup: u32 = (total_ticks / 10).max(10).min(total_ticks);
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

    let mut t_compute_rates = Duration::ZERO;
    let mut t_layout1 = Duration::ZERO;
    let mut t_cpu = Duration::ZERO;
    let mut t_layout2 = Duration::ZERO;
    let mut t_collect = Duration::ZERO;
    let mut t_apply = Duration::ZERO;
    let mut t_eot = Duration::ZERO;
    let mut t_gc = Duration::ZERO;

    let started = Instant::now();
    for _ in 0..measured {
        let s = Instant::now();
        tick::compute_natural_rates(&mut w, COEFF);
        t_compute_rates += s.elapsed();

        let s = Instant::now();
        tick::lay_out_pointers(&mut w);
        t_layout1 += s.elapsed();

        let s = Instant::now();
        tick::cpu_phase(&mut w, K);
        t_cpu += s.elapsed();

        let s = Instant::now();
        tick::lay_out_pointers(&mut w);
        t_layout2 += s.elapsed();

        let s = Instant::now();
        let outflow = tick::collect_outflow(&w);
        t_collect += s.elapsed();

        let s = Instant::now();
        tick::apply_outflow(&mut w, &outflow);
        t_apply += s.elapsed();

        let s = Instant::now();
        tick::end_of_tick(&mut w);
        t_eot += s.elapsed();

        let s = Instant::now();
        w.gc_empty();
        t_gc += s.elapsed();

        w.tick = w.tick.saturating_add(1);
    }
    let elapsed = started.elapsed();

    println!(
        "{measured} measured ticks in {:.2?} ({:.0} ticks/sec, {} cells, total_energy = {})",
        elapsed,
        f64::from(measured) / elapsed.as_secs_f64(),
        w.len(),
        w.total_energy(),
    );

    let phases: &[(&str, Duration)] = &[
        ("compute_natural_rates", t_compute_rates),
        ("lay_out_pointers (1st)", t_layout1),
        ("cpu_phase", t_cpu),
        ("lay_out_pointers (2nd)", t_layout2),
        ("collect_outflow", t_collect),
        ("apply_outflow", t_apply),
        ("end_of_tick", t_eot),
        ("gc_empty", t_gc),
    ];
    let phase_sum: Duration = phases.iter().map(|(_, d)| *d).sum();
    let denom_ms = phase_sum.as_secs_f64() * 1000.0;

    println!();
    println!("{:<26} {:>12}  {:>7}", "phase", "total (ms)", "% tick");
    for (name, d) in phases {
        let ms = d.as_secs_f64() * 1000.0;
        println!("{name:<26} {ms:>12.2}  {:>6.1}%", ms / denom_ms * 100.0);
    }
    println!();
    println!(
        "{:<26} {:>12.2}",
        "sum of phases (ms)",
        phase_sum.as_secs_f64() * 1000.0
    );
    println!(
        "{:<26} {:>12.2}",
        "wallclock (ms)",
        elapsed.as_secs_f64() * 1000.0
    );

    std::hint::black_box(&w);
}

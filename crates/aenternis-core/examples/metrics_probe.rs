//! Throwaway probe: does genesis (fertility) leave a lasting signature in the
//! code, and is mutation what erases it? Runs the Cauldron preset for
//! fertility {1, 20} × mutation {on, off}, sampling `compute_metrics` across
//! early and late ticks. Not part of the gate; delete after reading.
//!
//!   cargo run --release --example `metrics_probe`

use aenternis_core::{compute_metrics, tick, Base, GenesisConfig, SparseWorld};

const SEED: u64 = 1234;
const ENERGY: u32 = 1_000_000;
const COEFF: f64 = 0.15;
const K: u32 = 1;
const SAMPLE_TICKS: &[u32] = &[0, 5, 10, 20, 50, 100, 250, 650];

const fn cauldron(w: &mut SparseWorld, mutation: f64) {
    w.move_threshold = 1.0;
    w.gravity = 1.0;
    w.gravity_alpha = 0.05;
    w.gravity_radius = 4;
    w.pressure = 0.2;
    w.pressure_gamma = 2.0;
    w.pressure_eref = 50_000.0;
    w.mutation_strength = mutation;
    w.mutation_half_density = 40_000.0;
}

fn run(fertility: f64, mutation: f64) {
    let cfg = GenesisConfig {
        window: 256,
        fertility,
    };
    let mut w = SparseWorld::big_bang_with_config(SEED, ENERGY, Base::Macros, &[], &cfg);
    cauldron(&mut w, mutation);
    println!(
        "\n=== fertility={fertility}  mutation={mutation}  (max entropy = {:.3}) ===",
        (31f64).log2()
    );
    println!("  tick |   cells | entropy | diversity | uniqTypes");
    let mut next = 0usize;
    for t in 0..=*SAMPLE_TICKS.last().unwrap() {
        if next < SAMPLE_TICKS.len() && SAMPLE_TICKS[next] == t {
            let m = compute_metrics(&w);
            println!(
                "  {:>4} | {:>7} | {:>7.3} | {:>9.4} | {:>9}",
                t, m.cells, m.entropy_bits, m.cell_diversity, m.unique_types,
            );
            next += 1;
        }
        tick::step(&mut w, COEFF, K);
    }
}

fn main() {
    println!("Metrics probe — Cauldron preset, seed={SEED}, energy={ENERGY}");
    for &mutation in &[1.0, 0.0] {
        for &fertility in &[1.0, 20.0] {
            run(fertility, mutation);
        }
    }
}

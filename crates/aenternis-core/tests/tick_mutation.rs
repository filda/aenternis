//! Integration tests for density-coupled mutation (see
//! `docs/gravity-plan.md`).
//!
//! The flip probability is the saturating curve
//! `p = mutation_strength · E / (E + mutation_half_density)`. Invariants:
//!
//! 1. **No-op when off** (`mutation_strength == 0`) — memory byte-identical.
//! 2. **Conservation** — a bit flip changes a slot's value, never the
//!    slot count, so total energy and (isolated) every cell's energy hold.
//! 3. **Determinism** — same seed ⇒ identical mutated memory.
//! 4. **One bit per slot via XOR** (reversible) — distinguishes `^=` from
//!    `|=` / `&=`.
//! 5. **Saturating density coupling** — denser cells flip a larger
//!    fraction; a larger half-density `K` flips fewer; the strength scales
//!    the ceiling (value-pinned).

#![allow(clippy::cast_precision_loss)] // slot counts → f64 fractions, tiny

use aenternis_core::tick::{apply_density_coupled_mutation, step};
use aenternis_core::{Coord, SparseWorld};

fn memory_of(w: &SparseWorld, coord: Coord) -> Vec<u32> {
    w.cell_memory(coord).unwrap().to_vec()
}

/// `mutation_strength = 1`, `mutation_half_density = 0` ⇒ `p = E/(E+0) = 1`
/// for any non-empty cell: a clean "every slot flips once" configuration.
const fn full_mutation(w: &mut SparseWorld) {
    w.mutation_strength = 1.0;
    w.mutation_half_density = 0.0;
}

// ----- no-op when off --------------------------------------------------------

#[test]
fn mutation_is_a_noop_when_strength_is_zero() {
    let mut w = SparseWorld::big_bang(0x4242, 500);
    w.mutation_half_density = 100.0; // non-zero K, but strength stays 0
    let before = memory_of(&w, Coord::ORIGIN);
    apply_density_coupled_mutation(&mut w);
    assert_eq!(
        before,
        memory_of(&w, Coord::ORIGIN),
        "strength 0 must not mutate"
    );
}

// ----- conservation ----------------------------------------------------------

#[test]
fn mutation_alone_preserves_every_cell_energy() {
    // The isolated mutation phase never changes slot counts.
    let mut w = SparseWorld::new(1);
    full_mutation(&mut w);
    w.insert_with_memory(Coord::new(0, 0, 0), &[0xFFFF_FFFF; 40]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[0x1234_5678; 7]);
    let e_before: Vec<(Coord, u32)> = w.iter().map(|(c, cell)| (*c, cell.energy())).collect();
    apply_density_coupled_mutation(&mut w);
    let e_after: Vec<(Coord, u32)> = w.iter().map(|(c, cell)| (*c, cell.energy())).collect();
    assert_eq!(
        e_before, e_after,
        "mutation must not change any cell's energy"
    );
}

#[test]
fn total_energy_is_conserved_under_step_with_mutation() {
    let mut w = SparseWorld::big_bang(0xABCD, 40_000);
    w.mutation_strength = 0.5;
    w.mutation_half_density = 2_000.0;
    let e0 = w.total_energy();
    for _ in 0..40 {
        step(&mut w, 0.15, 1);
        assert_eq!(w.total_energy(), e0, "mutation must conserve total energy");
    }
}

// ----- determinism -----------------------------------------------------------

#[test]
fn mutation_run_is_deterministic() {
    let run = || {
        let mut w = SparseWorld::big_bang(0x7777, 20_000);
        w.mutation_strength = 0.5;
        w.mutation_half_density = 2_000.0;
        for _ in 0..25 {
            step(&mut w, 0.15, 1);
        }
        let mut fp: Vec<(Coord, Vec<u32>)> = w
            .iter()
            .map(|(c, _)| (*c, w.cell_memory(*c).unwrap().to_vec()))
            .collect();
        fp.sort_by_key(|(c, _)| *c);
        fp
    };
    assert_eq!(run(), run(), "mutation must be reproducible from the seed");
}

// ----- one bit per slot via XOR ----------------------------------------------

#[test]
fn mutation_changes_memory_at_full_rate() {
    let mut w = SparseWorld::new(0x9090);
    full_mutation(&mut w);
    w.insert_with_memory(Coord::ORIGIN, &[0x0F0F_0F0F; 50]);
    let before = memory_of(&w, Coord::ORIGIN);
    apply_density_coupled_mutation(&mut w);
    assert_ne!(
        before,
        memory_of(&w, Coord::ORIGIN),
        "p=1 must change memory"
    );
}

#[test]
fn mutation_flips_exactly_one_bit_per_slot_via_xor() {
    // p_flip = 1 (strength=1, K=0): every slot flips exactly once, and XOR
    // on an all-ones slot clears exactly one bit → popcount 31. `|=` would
    // leave it 32, `&=` would crash it to 1 — so popcount pins XOR.
    let mut w = SparseWorld::new(0x1357);
    full_mutation(&mut w);
    w.insert_with_memory(Coord::ORIGIN, &[0xFFFF_FFFF; 16]);
    apply_density_coupled_mutation(&mut w);
    for (i, slot) in memory_of(&w, Coord::ORIGIN).into_iter().enumerate() {
        assert_eq!(
            slot.count_ones(),
            31,
            "slot {i}: XOR of one bit into all-ones must clear exactly one bit"
        );
    }
}

// ----- saturating density coupling -------------------------------------------

/// Fraction of all-zero slots that became non-zero (= got flipped, since a
/// flip on a 0 slot sets exactly one bit).
fn flipped_fraction(w: &SparseWorld, coord: Coord) -> f64 {
    let mem = memory_of(w, coord);
    let flipped = mem.iter().filter(|&&s| s != 0).count();
    flipped as f64 / mem.len() as f64
}

#[test]
fn denser_cells_flip_a_larger_fraction() {
    // p = E/(E+K) rises with E, so the denser cell flips a larger fraction
    // of its slots. A sign flip in the E-dependence (e.g. K−E, or E·K in
    // the denominator) would break the ordering.
    let mut w = SparseWorld::new(0xD0E5);
    w.mutation_strength = 1.0;
    w.mutation_half_density = 50.0;
    w.insert_with_memory(Coord::new(0, 0, 0), &[0u32; 20]); // E=20 → p≈0.29
    w.insert_with_memory(Coord::new(1, 0, 0), &[0u32; 500]); // E=500 → p≈0.91
    apply_density_coupled_mutation(&mut w);
    let sparse = flipped_fraction(&w, Coord::new(0, 0, 0));
    let dense = flipped_fraction(&w, Coord::new(1, 0, 0));
    assert!(
        dense > sparse,
        "denser cell must flip a larger fraction (sparse={sparse:.2}, dense={dense:.2})"
    );
}

#[test]
fn larger_half_density_flips_fewer() {
    // Same cell, bigger K ⇒ smaller p ⇒ fewer flips. Pins the `+ K` in the
    // denominator (a `− K` or `* K` would invert or distort this).
    let build = |k: f64| {
        let mut w = SparseWorld::new(0x000B_EEF4);
        w.mutation_strength = 1.0;
        w.mutation_half_density = k;
        w.insert_with_memory(Coord::ORIGIN, &[0u32; 200]);
        apply_density_coupled_mutation(&mut w);
        flipped_fraction(&w, Coord::ORIGIN)
    };
    let small_k = build(10.0); // p = 200/210 ≈ 0.95
    let large_k = build(10_000.0); // p = 200/10200 ≈ 0.02
    assert!(
        large_k < small_k,
        "larger K must flip fewer (K=10 → {small_k:.2}, K=10000 → {large_k:.2})"
    );
}

#[test]
fn flip_fraction_matches_the_saturating_curve() {
    // Value pin on the whole formula. strength=0.5, E=K=400 ⇒
    // p = 0.5 · 400/800 = 0.25. With 400 all-zero slots ~100 flip. A band
    // [70, 130] accepts the correct p=0.25 (~100) and rejects the mutants:
    // `strength + E` (huge p→1), `strength / E`, `*`↔`/`, `E ± K` swaps —
    // all land far outside.
    let mut w = SparseWorld::new(0x0005_A117);
    w.mutation_strength = 0.5;
    w.mutation_half_density = 400.0;
    w.insert_with_memory(Coord::ORIGIN, &[0u32; 400]);
    apply_density_coupled_mutation(&mut w);
    let mem = memory_of(&w, Coord::ORIGIN);
    let flipped = mem.iter().filter(|&&s| s != 0).count();
    assert!(
        (70..=130).contains(&flipped),
        "p=0.25 over 400 slots should flip ~100 (got {flipped})"
    );
}

// ----- VM tolerance ----------------------------------------------------------

#[test]
fn vm_tolerates_heavily_mutated_program_memory() {
    // Mutated slots are executed as opcodes next tick. Post-fold every byte
    // decodes to a real opcode, so the VM must never panic on arbitrary
    // mutated memory — drive a small program cell hard.
    let program: Vec<u32> = (0..64).map(|i| i * 0x0101_0101 + 7).collect();
    let mut w = SparseWorld::big_bang_with_program(0xFEED, 2_000, &program);
    w.mutation_strength = 1.0;
    w.mutation_half_density = 500.0;
    for _ in 0..30 {
        step(&mut w, 0.2, 1); // must not panic
    }
    assert_eq!(w.total_energy(), 2_000);
}

//! Integration tests for density-coupled mutation (see
//! `docs/gravity-plan.md`).
//!
//! Invariants locked in:
//!
//! 1. **No-op at zero rate** — the default keeps memory byte-identical.
//! 2. **Conservation** — a bit flip changes a slot's value, never the
//!    slot count, so total energy and (for the isolated mutation phase)
//!    every cell's energy are preserved.
//! 3. **Determinism** — same seed ⇒ identical mutated memory.
//! 4. **It actually mutates**, flipping exactly one bit per affected slot
//!    via XOR (reversible), which distinguishes `^=` from `|=` / `&=`.

use aenternis_core::tick::{apply_density_coupled_mutation, step};
use aenternis_core::{Coord, SparseWorld};

fn memory_of(w: &SparseWorld, coord: Coord) -> Vec<u32> {
    w.cell_memory(coord).unwrap().to_vec()
}

// ----- no-op at zero rate ----------------------------------------------------

#[test]
fn mutation_is_a_noop_at_zero_rate() {
    let mut w = SparseWorld::big_bang(0x4242, 500);
    // base_mutation_rate defaults to 0.0.
    let before = memory_of(&w, Coord::ORIGIN);
    apply_density_coupled_mutation(&mut w);
    assert_eq!(
        before,
        memory_of(&w, Coord::ORIGIN),
        "rate 0 must not mutate"
    );
}

// ----- conservation ----------------------------------------------------------

#[test]
fn mutation_alone_preserves_every_cell_energy() {
    // The isolated mutation phase never changes slot counts.
    let mut w = SparseWorld::new(1);
    w.base_mutation_rate = 0.01;
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
    w.base_mutation_rate = 0.005;
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
        w.base_mutation_rate = 0.01;
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

// ----- it actually mutates ---------------------------------------------------

#[test]
fn mutation_changes_memory_at_a_high_rate() {
    let mut w = SparseWorld::new(0x9090);
    w.base_mutation_rate = 0.02; // p = min(0.02 * 50, 1) = 1.0 → every slot flips
    w.insert_with_memory(Coord::ORIGIN, &[0x0F0F_0F0F; 50]);
    let before = memory_of(&w, Coord::ORIGIN);
    apply_density_coupled_mutation(&mut w);
    let after = memory_of(&w, Coord::ORIGIN);
    assert_ne!(before, after, "a high rate must change memory");
}

#[test]
fn mutation_flips_exactly_one_bit_per_slot_via_xor() {
    // With p_flip = 1 every slot flips exactly once per call, and XOR on
    // an all-ones slot clears exactly one bit → popcount 31. A `|=` would
    // leave it at 32, a `&=` would crash it to 1 — so popcount pins XOR.
    let mut w = SparseWorld::new(0x1357);
    w.base_mutation_rate = 0.1; // p = min(0.1 * 16, 1) = 1.0
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

#[test]
fn flip_probability_multiplies_rate_by_density() {
    // p_flip = base_rate * E. With base_rate=0.02 and E=20 slots, p=0.4 —
    // a *fractional* probability, so on all-zero memory some slots flip
    // (become non-zero) and some stay 0. A `base_rate + E` mutant would
    // give p = 20.02 → clamped to 1.0 → *every* slot flips and none stay
    // 0, which this catches. (A flip on a 0 slot sets exactly one bit, so
    // flipped ⇔ non-zero here.)
    let mut w = SparseWorld::new(0x2468_ACE0);
    w.base_mutation_rate = 0.02;
    w.insert_with_memory(Coord::ORIGIN, &[0x0000_0000; 20]);
    apply_density_coupled_mutation(&mut w);
    let mem = memory_of(&w, Coord::ORIGIN);
    let zeros = mem.iter().filter(|&&s| s == 0).count();
    assert!(
        zeros > 0 && zeros < mem.len(),
        "fractional p must leave some slots unflipped and flip others \
         (zeros={zeros}/{})",
        mem.len()
    );
}

// ----- VM tolerance ----------------------------------------------------------

#[test]
fn vm_tolerates_heavily_mutated_program_memory() {
    // Mutated slots are executed as opcodes next tick. Post-fold every
    // byte decodes to a real opcode, so the VM must never panic on
    // arbitrary mutated memory — drive a small program cell hard.
    let program: Vec<u32> = (0..64).map(|i| i * 0x0101_0101 + 7).collect();
    let mut w = SparseWorld::big_bang_with_program(0xFEED, 2_000, &program);
    w.base_mutation_rate = 0.02;
    for _ in 0..30 {
        step(&mut w, 0.2, 1); // must not panic
    }
    assert_eq!(w.total_energy(), 2_000);
}

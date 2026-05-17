//! Integration tests for `cpu_phase` and the full `step`.
//!
//! `cpu_phase` is exercised in isolation against a single-cell world,
//! plus a two-cell world to confirm sensors see the right neighbor.
//! `step` is then tested for the cardinal invariants: tick advance,
//! energy conservation with VM running, determinism, and equivalence
//! to `step_diffusion` when the per-cell budget is zero.

use aenternis_core::tick::{cpu_phase, step, step_diffusion};
use aenternis_core::{Cell, Coord, Opcode, SparseWorld};

const fn op(o: Opcode) -> u32 {
    o as u32
}

// ----- cpu_phase -----

#[test]
fn cpu_phase_empty_world_is_noop() {
    let mut w = SparseWorld::new(0);
    cpu_phase(&mut w, 1);
    assert!(w.is_empty());
}

#[test]
fn cpu_phase_runs_floor_energy_div_k_instructions() {
    // 12 nop slots, k=1 → budget = 12. Each nop advances PC by 1, so
    // PC ends at 12 % 12 = 0 (full loop).
    let mut w = SparseWorld::new(0);
    let cell = w.alloc_cell(&[0u32; 12]);
    w.insert(Coord::ORIGIN, cell);
    cpu_phase(&mut w, 1);
    assert_eq!(w.get(Coord::ORIGIN).unwrap().pc, 0);
}

#[test]
fn cpu_phase_with_k_zero_treats_as_k_one() {
    // budget = 5 / max(0, 1) = 5 (treated as k=1).
    let mut w = SparseWorld::new(0);
    let cell = w.alloc_cell(&[0u32; 5]);
    w.insert(Coord::ORIGIN, cell);
    cpu_phase(&mut w, 0);
    // 5 nops, PC = 5 % 5 = 0.
    assert_eq!(w.get(Coord::ORIGIN).unwrap().pc, 0);
}

#[test]
fn cpu_phase_k_too_large_runs_zero_instructions() {
    // budget = 5 / 10 = 0 → no instruction runs, PC unchanged.
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[0u32; 5]);
    cell.pc = 3;
    w.insert(Coord::ORIGIN, cell);
    cpu_phase(&mut w, 10);
    assert_eq!(w.get(Coord::ORIGIN).unwrap().pc, 3);
}

#[test]
fn cpu_phase_each_cell_sees_its_own_neighbors() {
    // Cell A at origin reads its +x neighbor's energy via senergy.
    // Program: Senergy d=0 (Xp), a=4 → mem[4] = neighbors[Xp].energy.
    // Energy = 5, k = 5 → budget = 1 instruction.
    let mut w = SparseWorld::new(0);
    let a = w.alloc_cell(&[op(Opcode::Senergy), 0, 4, 0, 0]);
    w.insert(Coord::ORIGIN, a);
    let b = w.alloc_cell(&[1; 11]); // size 11
    w.insert(Coord::new(1, 0, 0), b);

    cpu_phase(&mut w, 5);

    let a = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(a.memory(w.arena())[4], 11, "expected B's energy to be observed");
}

#[test]
fn cpu_phase_two_cells_each_see_correct_neighbor_energy() {
    // Two adjacent cells, A at origin, B at +x. Both run senergy
    // toward each other and store the result in their own memory.
    //
    //   A program: Senergy d=0 (Xp) a=4 → mem[4] = B's energy
    //   B program: Senergy d=1 (Xn) a=4 → mem[4] = A's energy
    //
    // With k = 5 and energy = 5 each, both run exactly 1 instruction.
    let mut w = SparseWorld::new(0);
    let a = w.alloc_cell(&[op(Opcode::Senergy), 0, 4, 0, 0]); // size 5
    let b = w.alloc_cell(&[op(Opcode::Senergy), 1, 4, 0, 0]); // size 5
    w.insert(Coord::ORIGIN, a);
    w.insert(Coord::new(1, 0, 0), b);

    cpu_phase(&mut w, 5);

    assert_eq!(
        w.get(Coord::ORIGIN).unwrap().memory(w.arena())[4],
        5,
        "A should see B's energy"
    );
    assert_eq!(
        w.get(Coord::new(1, 0, 0)).unwrap().memory(w.arena())[4],
        5,
        "B should see A's energy"
    );
}

// ----- step -----

#[test]
fn step_advances_tick_by_one() {
    let mut w = SparseWorld::new(0);
    step(&mut w, 0.15, 1);
    assert_eq!(w.tick, 1);
}

#[test]
fn step_runs_cpu_phase() {
    // Program: Set mem[3] = 999. After 1 instruction: mem[3] = 999.
    // coeff = 0 → no diffusion outflow → memory survives the step.
    let mut w = SparseWorld::new(0);
    let cell = w.alloc_cell(&[op(Opcode::Set), 3, 999, 0, 0]); // size 5
    w.insert(Coord::ORIGIN, cell);

    step(&mut w, 0.0, 5); // k=5 → budget = 1 instruction

    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.memory(w.arena())[3], 999);
}

#[test]
fn step_conserves_energy_with_vm_running() {
    // Crown-jewel invariant: even with the CPU phase wired in and
    // arbitrary noise running as a program, total energy across the
    // world is preserved across many ticks. If anything (port,
    // setp/setpv override, sti indirect store) leaks energy, this
    // catches it.
    let mut w = SparseWorld::big_bang(0xDEAD_BEEF, 200);
    let total_before = w.total_energy();
    for _ in 0..30 {
        step(&mut w, 0.15, 1);
        assert_eq!(
            w.total_energy(),
            total_before,
            "energy not conserved at tick {}",
            w.tick
        );
    }
}

#[test]
fn step_is_deterministic_with_vm_running() {
    let mut a = SparseWorld::big_bang(2024, 100);
    let mut b = SparseWorld::big_bang(2024, 100);
    for _ in 0..20 {
        step(&mut a, 0.15, 1);
        step(&mut b, 0.15, 1);
    }
    let pa: Vec<_> = a
        .iter()
        .map(|(c, cell)| (*c, cell.memory(a.arena()).to_vec()))
        .collect();
    let pb: Vec<_> = b
        .iter()
        .map(|(c, cell)| (*c, cell.memory(b.arena()).to_vec()))
        .collect();
    assert_eq!(pa, pb);
}

#[test]
fn step_with_huge_k_matches_step_diffusion() {
    // Per-cell budget = floor(energy / u32::MAX) is 0 for any
    // realistic energy, so cpu_phase runs zero instructions. The
    // result must match `step_diffusion` byte-for-byte.
    let mut a = SparseWorld::big_bang(7, 50);
    let mut b = SparseWorld::big_bang(7, 50);
    for _ in 0..10 {
        step(&mut a, 0.15, u32::MAX);
        step_diffusion(&mut b, 0.15);
    }
    let pa: Vec<_> = a
        .iter()
        .map(|(c, cell)| (*c, cell.memory(a.arena()).to_vec()))
        .collect();
    let pb: Vec<_> = b
        .iter()
        .map(|(c, cell)| (*c, cell.memory(b.arena()).to_vec()))
        .collect();
    assert_eq!(pa, pb);
}

#[test]
fn step_world_size_bounded_by_total_energy_with_vm() {
    let mut w = SparseWorld::big_bang(99, 64);
    for _ in 0..20 {
        step(&mut w, 0.20, 1);
        let size = w.len() as u64;
        assert!(
            size <= w.total_energy(),
            "world.size() = {size} exceeds total_energy = {}",
            w.total_energy()
        );
    }
}

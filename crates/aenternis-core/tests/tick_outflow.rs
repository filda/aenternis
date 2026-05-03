//! Integration tests for `collect_outflow`.
//!
//! Properties checked:
//!
//! 1. **Per-direction length** matches `rate[d]` exactly.
//! 2. **Slot content** comes from `memory[ptr .. ptr+rate]` with modular
//!    wrap on memory length.
//! 3. **No void synthesis** — the outflow map only contains entries for
//!    cells that exist; void targets are not pre-allocated here.
//! 4. **Determinism** — same world produces identical outflow.
//! 5. Edge cases: empty world, empty cell, all-zero rates.

use aenternis_core::tick::collect_outflow;
use aenternis_core::{Cell, Coord, Direction, SparseWorld};

#[test]
fn empty_world_produces_empty_outflow() {
    let outflow = collect_outflow(&SparseWorld::new(0));
    assert!(outflow.is_empty());
}

#[test]
fn empty_cell_produces_all_empty_per_direction_vectors() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::ORIGIN, Cell::new());
    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).expect("entry should exist");
    for &d in &Direction::ALL {
        assert!(entry[d.index()].is_empty(), "expected empty for {d:?}");
    }
}

#[test]
fn cell_with_zero_rates_produces_empty_vectors() {
    let mut w = SparseWorld::new(0);
    // Memory is non-empty, but all rates default to 0 → no slots emitted.
    w.insert(Coord::ORIGIN, Cell::with_memory(vec![1, 2, 3]));
    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).unwrap();
    for &d in &Direction::ALL {
        assert!(entry[d.index()].is_empty(), "expected empty for {d:?}");
    }
}

#[test]
fn per_direction_length_matches_rate() {
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![10, 20, 30, 40, 50, 60, 70, 80]);
    cell.rates = [1, 2, 1, 0, 0, 0];
    cell.pointers = [0, 1, 3, 4, 5, 6];
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).unwrap();

    assert_eq!(entry[Direction::Xp.index()].len(), 1);
    assert_eq!(entry[Direction::Xn.index()].len(), 2);
    assert_eq!(entry[Direction::Yp.index()].len(), 1);
    assert_eq!(entry[Direction::Yn.index()].len(), 0);
    assert_eq!(entry[Direction::Zp.index()].len(), 0);
    assert_eq!(entry[Direction::Zn.index()].len(), 0);
}

#[test]
fn slots_come_from_memory_starting_at_pointer() {
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![10, 20, 30, 40, 50]);
    cell.rates[Direction::Xp.index()] = 3;
    cell.pointers[Direction::Xp.index()] = 1; // start at index 1
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).unwrap();
    assert_eq!(entry[Direction::Xp.index()], vec![20, 30, 40]);
}

#[test]
fn slots_wrap_modulo_memory_length() {
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![10, 20, 30]);
    cell.rates[Direction::Xp.index()] = 3; // exactly mem_size
    cell.pointers[Direction::Xp.index()] = 2; // wraps after 1 slot
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).unwrap();
    // Read order: m[2], m[(2+1) % 3 = 0], m[(2+2) % 3 = 1]
    assert_eq!(entry[Direction::Xp.index()], vec![30, 10, 20]);
}

#[test]
fn outflow_includes_all_existing_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(5, 5, 5), Cell::with_memory(vec![2]));
    w.insert(Coord::new(-3, 1, 4), Cell::with_memory(vec![3]));

    let outflow = collect_outflow(&w);
    assert_eq!(outflow.len(), 3);
    assert!(outflow.contains_key(&Coord::new(0, 0, 0)));
    assert!(outflow.contains_key(&Coord::new(5, 5, 5)));
    assert!(outflow.contains_key(&Coord::new(-3, 1, 4)));
}

#[test]
fn outflow_does_not_synthesize_void_targets() {
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![10]);
    cell.rates[Direction::Xp.index()] = 1;
    cell.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    // The source is in the map; the void target at (1, 0, 0) is not.
    // Allocation happens later in the inflow phase.
    assert_eq!(outflow.len(), 1);
    assert!(outflow.contains_key(&Coord::ORIGIN));
    assert!(!outflow.contains_key(&Coord::new(1, 0, 0)));
}

#[test]
fn outflow_is_deterministic() {
    let make_world = || {
        let mut w = SparseWorld::new(7);
        let mut cell = Cell::with_memory(vec![1, 2, 3, 4, 5]);
        cell.rates = [1, 0, 1, 0, 1, 0];
        cell.pointers = [0, 0, 1, 0, 2, 0];
        w.insert(Coord::ORIGIN, cell);
        w
    };

    let a = make_world();
    let b = make_world();
    assert_eq!(collect_outflow(&a), collect_outflow(&b));
}

#[test]
fn outflow_walks_all_six_directions() {
    // Each direction emits one slot from a distinct memory position,
    // so the resulting per-direction vectors are all distinct.
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![10, 20, 30, 40, 50, 60]);
    cell.rates = [1, 1, 1, 1, 1, 1];
    cell.pointers = [0, 1, 2, 3, 4, 5];
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    let entry = outflow.get(&Coord::ORIGIN).unwrap();
    assert_eq!(entry[Direction::Xp.index()], vec![10]);
    assert_eq!(entry[Direction::Xn.index()], vec![20]);
    assert_eq!(entry[Direction::Yp.index()], vec![30]);
    assert_eq!(entry[Direction::Yn.index()], vec![40]);
    assert_eq!(entry[Direction::Zp.index()], vec![50]);
    assert_eq!(entry[Direction::Zn.index()], vec![60]);
}

//! Integration tests for `apply_outflow`, `lay_out_pointers`,
//! `end_of_tick`, and the end-to-end `step_diffusion`.
//!
//! The single most important property checked here is **energy
//! conservation**: total slot count across the world is preserved
//! across every diffusion tick. If anything in the cycle leaks or
//! synthesizes slots, this is the test that catches it.

use aenternis_core::tick::{
    apply_outflow, collect_outflow, end_of_tick, lay_out_pointers, step_diffusion, Outflow,
};
use aenternis_core::{Cell, Coord, Direction, SparseWorld};

// ----- get_or_alloc (sparse world helper) -----

#[test]
fn get_or_alloc_returns_existing_cell_unchanged() {
    let mut w = SparseWorld::new(0);
    let c = Coord::new(2, 3, 5);
    w.insert_with_memory(c, &[7, 8, 9]);
    let original_tag = w.get(c).unwrap().origin_tag;

    let cell_tag = w.get_or_alloc(c).origin_tag;
    assert_eq!(w.cell_memory(c).unwrap(), &[7, 8, 9]);
    assert_eq!(cell_tag, original_tag);
}

#[test]
fn get_or_alloc_creates_empty_cell_with_origin_tag() {
    let mut w = SparseWorld::new(0xCAFE);
    let c = Coord::new(1, 2, 3);
    let cell = w.get_or_alloc(c);
    assert!(cell.is_empty());
    // origin_tag was drawn from the per-cell-at-tick stream — the
    // value is deterministic but we only assert it isn't zero.
    // (False-positive risk: 1 / 2^32 that the RNG actually emits 0.)
    assert_ne!(cell.origin_tag, 0);
}

#[test]
fn get_or_alloc_origin_tag_is_deterministic() {
    let coord = Coord::new(7, 11, 13);
    let mut a = SparseWorld::new(42);
    let mut b = SparseWorld::new(42);
    let tag_a = a.get_or_alloc(coord).origin_tag;
    let tag_b = b.get_or_alloc(coord).origin_tag;
    assert_eq!(tag_a, tag_b);
}

#[test]
fn get_or_alloc_origin_tag_depends_on_coord() {
    let mut w = SparseWorld::new(0);
    let tag_at_origin = w.get_or_alloc(Coord::ORIGIN).origin_tag;
    let tag_at_far = w.get_or_alloc(Coord::new(5, 5, 5)).origin_tag;
    assert_ne!(tag_at_origin, tag_at_far);
}

// ----- end_of_tick -----

#[test]
fn end_of_tick_resets_every_cell() {
    let mut w = SparseWorld::new(0);
    for c in [Coord::new(0, 0, 0), Coord::new(1, 0, 0)] {
        let mut cell = w.alloc_cell(&[1, 2, 3]);
        cell.pointer_override = [true; 6];
        cell.active_outflow = [9; 6];
        w.insert(c, cell);
    }
    end_of_tick(&mut w);
    for (_, cell) in &w {
        assert_eq!(cell.pointer_override, [false; 6]);
        assert_eq!(cell.active_outflow, [0; 6]);
    }
}

// ----- lay_out_pointers (world-level) -----

#[test]
fn lay_out_pointers_sets_pointers_from_rates() {
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[0; 21]);
    cell.rates = [1, 2, 3, 4, 5, 6];
    w.insert(Coord::ORIGIN, cell);

    lay_out_pointers(&mut w);

    let cell = w.get(Coord::ORIGIN).unwrap();
    // Same expected layout as Cell::lay_out_pointers' "balanced" test.
    assert_eq!(cell.pointers[Direction::Xp.index()], 0);
    assert_eq!(cell.pointers[Direction::Xn.index()], 1);
    assert_eq!(cell.pointers[Direction::Yp.index()], 3);
    assert_eq!(cell.pointers[Direction::Yn.index()], 6);
    assert_eq!(cell.pointers[Direction::Zp.index()], 10);
    assert_eq!(cell.pointers[Direction::Zn.index()], 15);
}

#[test]
fn lay_out_pointers_includes_active_outflow() {
    // combined = rates + active_outflow. With rates [0; 6] but
    // active_outflow [1, 2, 3, 4, 5, 6], the layout should match
    // the same balanced case as if rates were [1, 2, 3, 4, 5, 6].
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[0; 21]);
    cell.rates = [0; 6];
    cell.active_outflow = [1, 2, 3, 4, 5, 6];
    w.insert(Coord::ORIGIN, cell);

    lay_out_pointers(&mut w);

    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.pointers[Direction::Zn.index()], 15);
    assert_eq!(cell.pointers[Direction::Xp.index()], 0);
}

// ----- apply_outflow -----

#[test]
fn apply_outflow_with_empty_outflow_is_noop() {
    let mut w = SparseWorld::big_bang(7, 16);
    let snapshot_before = w.total_energy();
    apply_outflow(&mut w, &Outflow::default());
    assert_eq!(w.total_energy(), snapshot_before);
    assert_eq!(w.len(), 1);
}

#[test]
fn apply_outflow_shrinks_source_by_total_rate() {
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    cell.rates = [1, 1, 1, 0, 0, 0];
    cell.pointers = [0, 1, 2, 0, 0, 0];
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    // Source shrank by 3 (sum of rates).
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.energy(), 7);
    assert_eq!(cell.memory(w.arena()), vec![1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn apply_outflow_allocates_void_neighbor_with_inflow() {
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[10, 20, 30, 40, 50]);
    cell.rates[Direction::Xp.index()] = 2;
    cell.pointers[Direction::Xp.index()] = 0;
    cell.origin_tag = 0xCAFE; // explicit so we can verify inheritance
    w.insert(Coord::ORIGIN, cell);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    // The +x neighbor was void; it should now exist with the 2
    // emitted slots in its memory.
    let target = w
        .get(Coord::new(1, 0, 0))
        .expect("alloc-on-write should have created the cell");
    assert_eq!(target.memory(w.arena()), vec![10, 20]);
    // Strong attacker against an empty target → dominance ≈ 1.0,
    // so the target inherits the attacker's origin tag.
    assert_eq!(target.origin_tag, 0xCAFE);
}

#[test]
fn apply_outflow_appends_into_existing_neighbor() {
    let mut w = SparseWorld::new(0);
    let mut source = w.alloc_cell(&[10, 20, 30]);
    source.rates[Direction::Xp.index()] = 2;
    source.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::ORIGIN, source);
    w.insert_with_memory(Coord::new(1, 0, 0), &[99, 99]);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    // Existing memory + appended inflow.
    assert_eq!(target.memory(w.arena()), vec![99, 99, 10, 20]);
}

#[test]
fn apply_outflow_conserves_total_slots() {
    // Place a single cell with all six rates non-zero. After applying
    // outflow the total slot count across the entire world (source +
    // alloc'd void neighbors) should equal the original memory size.
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    cell.rates = [1, 1, 1, 1, 1, 1];
    cell.pointers = [0, 1, 2, 3, 4, 5];
    w.insert(Coord::ORIGIN, cell);

    let before = w.total_energy();

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    assert_eq!(
        w.total_energy(),
        before,
        "total slot count must be preserved"
    );
}

#[test]
fn apply_outflow_skips_outflow_for_missing_source() {
    // Outflow map can in principle reference a cell that no longer
    // exists in the world (e.g. if someone manually cleared it
    // between snapshot and apply). The function must tolerate that
    // and ignore the entry rather than panic.
    let mut w = SparseWorld::new(0);
    let mut outflow = Outflow::default();
    let mut per_dir: [Vec<u32>; Direction::COUNT] = Default::default();
    per_dir[Direction::Xp.index()] = vec![42];
    outflow.insert(Coord::ORIGIN, per_dir);

    apply_outflow(&mut w, &outflow);

    // No source existed, but the slot still flowed into the void
    // neighbor at (1, 0, 0).
    assert!(!w.contains(Coord::ORIGIN));
    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory(w.arena()), vec![42]);
}

// ----- step_diffusion (end-to-end) -----

#[test]
fn step_diffusion_empty_world_advances_tick() {
    let mut w = SparseWorld::new(7);
    step_diffusion(&mut w, 0.15);
    assert_eq!(w.tick, 1);
    assert!(w.is_empty());
}

#[test]
fn step_diffusion_conserves_energy_over_many_ticks() {
    // The crown-jewel invariant. If the diffusion cycle leaks or
    // synthesizes slots anywhere — outflow, inflow, alloc-on-write,
    // GC — this test fails. Run for 50 ticks with a non-trivial
    // initial energy and a reasonable coeff.
    let mut w = SparseWorld::big_bang(0xDEAD_BEEF, 200);
    let total_before = w.total_energy();
    for _ in 0..50 {
        step_diffusion(&mut w, 0.15);
        assert_eq!(
            w.total_energy(),
            total_before,
            "energy not conserved at tick {}",
            w.tick
        );
    }
}

#[test]
fn step_diffusion_world_size_bounded_by_total_energy() {
    let mut w = SparseWorld::big_bang(99, 64);
    for _ in 0..20 {
        step_diffusion(&mut w, 0.20);
        let size = w.len() as u64;
        assert!(
            size <= w.total_energy(),
            "world.size() = {size} exceeds total_energy = {}",
            w.total_energy()
        );
    }
}

#[test]
fn step_diffusion_expands_outward_from_origin() {
    let mut w = SparseWorld::big_bang(0, 100);
    assert_eq!(w.len(), 1);
    step_diffusion(&mut w, 0.30);
    // After one tick of strong diffusion, neighbors should exist.
    assert!(w.len() > 1, "expected expansion, got {} cells", w.len());
    // Specifically, at least one orthogonal neighbor of origin
    // should have been alloc-on-written.
    let neighbor_alloced = Direction::ALL
        .iter()
        .any(|&d| w.contains(Coord::ORIGIN.neighbor(d)));
    assert!(neighbor_alloced, "no orthogonal neighbor was allocated");
}

#[test]
fn step_diffusion_produces_no_empty_cells() {
    let mut w = SparseWorld::big_bang(7, 50);
    for _ in 0..10 {
        step_diffusion(&mut w, 0.20);
        for (coord, cell) in &w {
            assert!(
                !cell.is_empty(),
                "cell at {coord:?} has zero energy after gc",
            );
        }
    }
}

#[test]
fn step_diffusion_is_deterministic() {
    let mut a = SparseWorld::big_bang(2024, 100);
    let mut b = SparseWorld::big_bang(2024, 100);
    for _ in 0..30 {
        step_diffusion(&mut a, 0.15);
        step_diffusion(&mut b, 0.15);
    }
    // Compare cell-by-cell. Both worlds iterate in the same
    // BTreeMap order, so a direct iter zip gives byte-identity.
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

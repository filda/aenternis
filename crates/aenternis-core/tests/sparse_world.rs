//! Integration tests for the sparse world container.

use aenternis_core::rng::cell_seed_xs32;
use aenternis_core::{Cell, Coord, Direction, RngKind, SparseWorld};

// ----- new / big_bang -----

#[test]
fn new_creates_empty_world_with_seed() {
    let w = SparseWorld::new(42);
    assert_eq!(w.world_seed, 42);
    assert_eq!(w.tick, 0);
    assert!(w.is_empty());
    assert_eq!(w.len(), 0);
}

#[test]
fn new_with_zero_seed_is_valid() {
    let w = SparseWorld::new(0);
    assert_eq!(w.world_seed, 0);
    assert!(w.is_empty());
}

#[test]
fn big_bang_zero_energy_yields_empty_world() {
    let w = SparseWorld::big_bang(123, 0);
    assert!(w.is_empty());
    assert_eq!(w.len(), 0);
    assert!(!w.contains(Coord::ORIGIN));
}

#[test]
fn big_bang_places_single_cell_at_origin() {
    let w = SparseWorld::big_bang(7, 16);
    assert_eq!(w.len(), 1);
    assert!(w.contains(Coord::ORIGIN));
    let cell = w.get(Coord::ORIGIN).expect("origin cell missing");
    assert_eq!(cell.energy(), 16);
    assert_eq!(cell.memory.len(), 16);
}

#[test]
fn big_bang_is_deterministic() {
    let a = SparseWorld::big_bang(0xDEAD_BEEF, 32);
    let b = SparseWorld::big_bang(0xDEAD_BEEF, 32);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_eq!(ca.memory, cb.memory);
    assert_eq!(ca.origin_tag, cb.origin_tag);
}

#[test]
fn big_bang_different_seeds_produce_different_memory() {
    let a = SparseWorld::big_bang(1, 32);
    let b = SparseWorld::big_bang(2, 32);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_ne!(ca.memory, cb.memory);
}

#[test]
fn big_bang_with_program_writes_prefix() {
    let program = [0xCAFE, 0xBABE, 0xDEAD];
    let w = SparseWorld::big_bang_with_program(7, 16, &program);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.memory[0], 0xCAFE);
    assert_eq!(cell.memory[1], 0xBABE);
    assert_eq!(cell.memory[2], 0xDEAD);
}

#[test]
fn big_bang_xs32_uses_cell_seed_as_origin_tag() {
    // JS prototype 9-B sets `originTag = cellSeed(seed, x, y, z)` directly
    // (not the first RNG draw). The Xorshift32 path must reproduce that.
    let w = SparseWorld::big_bang_with_program_and_kind(7, 16, &[], RngKind::Xorshift32);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.origin_tag, cell_seed_xs32(7, Coord::ORIGIN));
}

#[test]
fn big_bang_xs32_diverges_from_pcg() {
    // Same seed/energy, different RNG backends → different memory streams.
    let pcg = SparseWorld::big_bang(7, 16);
    let xs = SparseWorld::big_bang_with_program_and_kind(7, 16, &[], RngKind::Xorshift32);
    let cp = pcg.get(Coord::ORIGIN).unwrap();
    let cx = xs.get(Coord::ORIGIN).unwrap();
    assert_ne!(cp.memory, cx.memory);
    // Tags differ too — PCG draws from splitmix-PCG chain, xs32 hashes
    // coords directly.
    assert_ne!(cp.origin_tag, cx.origin_tag);
}

#[test]
fn bounding_box_is_none_for_empty_world() {
    let w = SparseWorld::new(0);
    assert_eq!(w.bounding_box(), None);
}

#[test]
fn bounding_box_for_single_cell_at_origin() {
    let w = SparseWorld::big_bang(1, 4);
    assert_eq!(w.bounding_box(), Some((0, 0, 0, 0, 0, 0)));
}

#[test]
fn bounding_box_spans_inserted_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(-3, 5, 7), Cell::with_memory(vec![1]));
    w.insert(Coord::new(2, -1, 7), Cell::with_memory(vec![1]));
    w.insert(Coord::new(0, 5, -4), Cell::with_memory(vec![1]));
    // x: -3..2, y: -1..5, z: -4..7
    assert_eq!(w.bounding_box(), Some((-3, 2, -1, 5, -4, 7)));
}

#[test]
fn bounding_box_y_max_extends_when_later_cell_has_larger_y() {
    // BTreeMap iteration is sorted by (x, y, z), so the first cell wins
    // initial y_max. We need a cell whose y is strictly greater than the
    // first cell's y (and whose x is greater so it iterates *after* the
    // first) to verify the `>` comparison actually fires for y_max.
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 1, 0), Cell::with_memory(vec![1])); // first
    w.insert(Coord::new(1, 9, 0), Cell::with_memory(vec![1])); // y bumps
    let bb = w.bounding_box().unwrap();
    assert_eq!(bb.3, 9, "expected y_max = 9, got {bb:?}");
}

#[test]
fn bounding_box_z_max_extends_when_later_cell_has_larger_z() {
    // Same idea for z_max. Without this, both `>` → `==` and `>` → `>=`
    // mutations leave z_max untouched (assignment is no-op when neither
    // strictly greater nor equal), and the test never observes the
    // comparison.
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 1), Cell::with_memory(vec![1])); // first
    w.insert(Coord::new(1, 0, 9), Cell::with_memory(vec![1])); // z bumps
    let bb = w.bounding_box().unwrap();
    assert_eq!(bb.5, 9, "expected z_max = 9, got {bb:?}");
}

#[test]
fn is_empty_returns_false_when_world_has_cells() {
    // A world with at least one cell must not report empty. Pins down the
    // truthful return value so an `is_empty -> true` mutation is caught.
    let mut w = SparseWorld::new(0);
    w.insert(Coord::ORIGIN, Cell::with_memory(vec![1]));
    assert!(!w.is_empty());
}

#[test]
fn rng_kind_persisted_on_world() {
    // The choice survives through the world struct so subsequent ticks
    // (fresh_cell on alloc-on-write, compute_natural_rates on layout)
    // see the right backend without callers having to thread it.
    let w = SparseWorld::big_bang_with_program_and_kind(1, 4, &[], RngKind::Xorshift32);
    assert_eq!(w.rng_kind, RngKind::Xorshift32);
    let pcg = SparseWorld::big_bang(1, 4);
    assert_eq!(pcg.rng_kind, RngKind::Pcg);
}

#[test]
fn big_bang_with_program_fills_rest_from_rng() {
    // The first 3 slots come from program; the remaining 13 from RNG.
    // Two runs with different programs but the same seed/energy must
    // produce the same suffix because the RNG isn't advanced for
    // program-covered slots.
    let a = SparseWorld::big_bang_with_program(42, 16, &[1, 2, 3]);
    let b = SparseWorld::big_bang_with_program(42, 16, &[9, 9, 9]);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    // Prefixes differ as supplied.
    assert_ne!(&ca.memory[..3], &cb.memory[..3]);
    // Suffixes match — RNG advanced identically.
    assert_eq!(&ca.memory[3..], &cb.memory[3..]);
}

#[test]
fn big_bang_with_program_truncates_oversized() {
    let program: Vec<u32> = (0..100).collect();
    let w = SparseWorld::big_bang_with_program(0, 5, &program);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.memory.len(), 5);
    assert_eq!(cell.memory, vec![0, 1, 2, 3, 4]);
}

#[test]
fn big_bang_with_empty_program_matches_big_bang() {
    let a = SparseWorld::big_bang(123, 32);
    let b = SparseWorld::big_bang_with_program(123, 32, &[]);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_eq!(ca.memory, cb.memory);
    assert_eq!(ca.origin_tag, cb.origin_tag);
}

#[test]
fn big_bang_origin_tag_is_set() {
    let w = SparseWorld::big_bang(99, 8);
    let cell = w.get(Coord::ORIGIN).unwrap();
    // The tag is whatever the RNG produced after the memory slots; we
    // can't assert on the value, but we *can* check it's not the default
    // zero (which would indicate the field was never written).
    // (False-positive risk: 1 / 2^32 that the RNG actually emits zero.)
    assert_ne!(cell.origin_tag, 0);
}

// ----- contains / get / get_mut / insert / remove -----

#[test]
fn contains_reflects_insert_and_remove() {
    let mut w = SparseWorld::new(0);
    let c = Coord::new(1, 2, 3);
    assert!(!w.contains(c));
    w.insert(c, Cell::with_memory(vec![1]));
    assert!(w.contains(c));
    let removed = w.remove(c);
    assert!(removed.is_some());
    assert!(!w.contains(c));
}

#[test]
fn get_returns_none_for_missing() {
    let w = SparseWorld::new(0);
    assert!(w.get(Coord::new(99, 99, 99)).is_none());
}

#[test]
fn get_mut_allows_modification() {
    let mut w = SparseWorld::new(0);
    let c = Coord::new(0, 0, 0);
    w.insert(c, Cell::with_memory(vec![1, 2, 3]));
    w.get_mut(c).unwrap().memory.push(4);
    assert_eq!(w.get(c).unwrap().memory, vec![1, 2, 3, 4]);
}

#[test]
fn insert_returns_previous_cell_on_replace() {
    let mut w = SparseWorld::new(0);
    let c = Coord::new(0, 0, 0);
    w.insert(c, Cell::with_memory(vec![1]));
    let prev = w
        .insert(c, Cell::with_memory(vec![9, 9]))
        .expect("expected previous");
    assert_eq!(prev.memory, vec![1]);
    assert_eq!(w.get(c).unwrap().memory, vec![9, 9]);
}

#[test]
fn remove_returns_none_for_missing() {
    let mut w = SparseWorld::new(0);
    assert!(w.remove(Coord::new(5, 5, 5)).is_none());
}

#[test]
fn len_tracks_inserts_and_removes() {
    let mut w = SparseWorld::new(0);
    assert_eq!(w.len(), 0);
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(2, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(3, 0, 0), Cell::with_memory(vec![1]));
    assert_eq!(w.len(), 3);
    w.remove(Coord::new(2, 0, 0));
    assert_eq!(w.len(), 2);
}

// ----- neighbor / neighbor_energy -----

#[test]
fn neighbor_returns_none_for_missing_cell() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::ORIGIN, Cell::with_memory(vec![1]));
    assert!(w.neighbor(Coord::ORIGIN, Direction::Xp).is_none());
}

#[test]
fn neighbor_returns_some_for_existing_neighbor() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::ORIGIN, Cell::with_memory(vec![1]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![2, 3]));
    let n = w
        .neighbor(Coord::ORIGIN, Direction::Xp)
        .expect("neighbor should exist");
    assert_eq!(n.memory, vec![2, 3]);
}

#[test]
fn neighbor_works_for_all_six_directions() {
    let mut w = SparseWorld::new(0);
    let center = Coord::new(0, 0, 0);
    for &d in &Direction::ALL {
        w.insert(
            center.neighbor(d),
            Cell::with_memory(vec![d.index() as u32]),
        );
    }
    for &d in &Direction::ALL {
        let n = w
            .neighbor(center, d)
            .unwrap_or_else(|| panic!("missing neighbor {d:?}"));
        assert_eq!(n.memory, vec![d.index() as u32]);
    }
}

#[test]
fn neighbor_energy_zero_for_missing() {
    let w = SparseWorld::new(0);
    assert_eq!(w.neighbor_energy(Coord::ORIGIN, Direction::Xp), 0);
}

#[test]
fn neighbor_energy_matches_neighbor_cell_energy() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1, 2, 3, 4, 5]));
    assert_eq!(w.neighbor_energy(Coord::ORIGIN, Direction::Xp), 5);
}

// ----- total_energy -----

#[test]
fn total_energy_zero_for_empty() {
    assert_eq!(SparseWorld::new(0).total_energy(), 0);
}

#[test]
fn total_energy_sums_all_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1; 3]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1; 5]));
    w.insert(Coord::new(0, 1, 0), Cell::with_memory(vec![1; 7]));
    assert_eq!(w.total_energy(), 15);
}

#[test]
fn total_energy_after_big_bang_matches_initial_energy() {
    let w = SparseWorld::big_bang(42, 1024);
    assert_eq!(w.total_energy(), 1024);
}

#[test]
fn total_energy_returns_u64() {
    // Type assertion: if the signature ever drifts away from u64 (e.g.
    // someone changes it to u32 to "save a few bytes"), this binding
    // stops compiling. The assertion is at the type level, not in a
    // runtime check, so the test stays small.
    let w = SparseWorld::big_bang(0, 100);
    let total: u64 = w.total_energy();
    assert_eq!(total, 100);
}

// ----- gc_empty -----

#[test]
fn gc_empty_removes_empty_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::new()); // empty
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1])); // non-empty
    w.insert(Coord::new(2, 0, 0), Cell::new()); // empty
    assert_eq!(w.len(), 3);
    w.gc_empty();
    assert_eq!(w.len(), 1);
    assert!(w.contains(Coord::new(1, 0, 0)));
    assert!(!w.contains(Coord::new(0, 0, 0)));
    assert!(!w.contains(Coord::new(2, 0, 0)));
}

#[test]
fn gc_empty_is_noop_when_no_empty_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1, 2]));
    w.gc_empty();
    assert_eq!(w.len(), 2);
}

#[test]
fn gc_empty_clears_world_with_only_empty_cells() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::new());
    w.insert(Coord::new(1, 0, 0), Cell::new());
    w.gc_empty();
    assert!(w.is_empty());
}

// ----- iter / iter_mut / coords -----

#[test]
fn iter_walks_cells_in_canonical_order() {
    let mut w = SparseWorld::new(0);
    // Insert in reverse-canonical order to confirm BTreeMap re-sorts.
    w.insert(Coord::new(2, 0, 0), Cell::with_memory(vec![3]));
    w.insert(Coord::new(0, 1, 0), Cell::with_memory(vec![2]));
    w.insert(Coord::new(0, 0, 1), Cell::with_memory(vec![1]));

    let collected: Vec<Coord> = w.coords().copied().collect();
    // BTreeMap order is lexicographic by (x, y, z).
    assert_eq!(
        collected,
        vec![
            Coord::new(0, 0, 1),
            Coord::new(0, 1, 0),
            Coord::new(2, 0, 0),
        ]
    );
}

#[test]
fn iter_yields_coord_cell_pairs() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![2, 3]));

    let pairs: Vec<(Coord, u32)> = w.iter().map(|(c, cell)| (*c, cell.energy())).collect();
    assert_eq!(
        pairs,
        vec![(Coord::new(0, 0, 0), 1), (Coord::new(1, 0, 0), 2)]
    );
}

#[test]
fn iter_mut_allows_modification() {
    // Uses `iter_mut().for_each(...)` rather than `for (_, c) in w.iter_mut()`
    // so this exercises the method form of `iter_mut`. The `for (_, c) in
    // &mut world` form is covered by `into_iter_for_mut_reference_*`.
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![2]));

    w.iter_mut().for_each(|(_, cell)| cell.memory.push(99));

    assert_eq!(w.get(Coord::new(0, 0, 0)).unwrap().memory, vec![1, 99]);
    assert_eq!(w.get(Coord::new(1, 0, 0)).unwrap().memory, vec![2, 99]);
}

#[test]
fn into_iter_for_shared_reference_walks_pairs() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1, 2]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![3]));
    let energies: Vec<u32> = (&w).into_iter().map(|(_, cell)| cell.energy()).collect();
    assert_eq!(energies, vec![2, 1]);
}

#[test]
fn into_iter_for_mut_reference_allows_modification() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![2]));
    for (_, cell) in &mut w {
        cell.memory.push(99);
    }
    assert_eq!(w.get(Coord::new(0, 0, 0)).unwrap().memory, vec![1, 99]);
    assert_eq!(w.get(Coord::new(1, 0, 0)).unwrap().memory, vec![2, 99]);
}

#[test]
fn coords_yields_just_keys_in_order() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(5, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(2, 0, 0), Cell::with_memory(vec![1]));
    w.insert(Coord::new(3, 0, 0), Cell::with_memory(vec![1]));
    let keys: Vec<Coord> = w.coords().copied().collect();
    assert_eq!(
        keys,
        vec![
            Coord::new(2, 0, 0),
            Coord::new(3, 0, 0),
            Coord::new(5, 0, 0)
        ]
    );
}

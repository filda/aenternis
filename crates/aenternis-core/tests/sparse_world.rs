//! Integration tests for the sparse world container.

use aenternis_core::rng::cell_seed;
use aenternis_core::{Cell, Coord, Direction, SparseWorld};

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
    assert_eq!(cell.memory_len(), 16);
}

#[test]
fn big_bang_is_deterministic() {
    let a = SparseWorld::big_bang(0xDEAD_BEEF, 32);
    let b = SparseWorld::big_bang(0xDEAD_BEEF, 32);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_eq!(ca.memory(a.arena()), cb.memory(b.arena()));
    assert_eq!(ca.origin_tag, cb.origin_tag);
}

#[test]
fn big_bang_different_seeds_produce_different_memory() {
    let a = SparseWorld::big_bang(1, 32);
    let b = SparseWorld::big_bang(2, 32);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_ne!(ca.memory(a.arena()), cb.memory(b.arena()));
}

#[test]
fn big_bang_with_program_writes_prefix() {
    let program = [0xCAFE, 0xBABE, 0xDEAD];
    let w = SparseWorld::big_bang_with_program(7, 16, &program);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.memory(w.arena())[0], 0xCAFE);
    assert_eq!(cell.memory(w.arena())[1], 0xBABE);
    assert_eq!(cell.memory(w.arena())[2], 0xDEAD);
}

#[test]
fn big_bang_uses_cell_seed_as_origin_tag() {
    // `origin_tag = cell_seed(seed, x, y, z)` — the seed value itself,
    // not the first RNG draw. This contract is part of the lineage
    // tracker's identity definition.
    let w = SparseWorld::big_bang(7, 16);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.origin_tag, cell_seed(7, Coord::ORIGIN));
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
    w.insert_with_memory(Coord::new(-3, 5, 7), &[1]);
    w.insert_with_memory(Coord::new(2, -1, 7), &[1]);
    w.insert_with_memory(Coord::new(0, 5, -4), &[1]);
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
    w.insert_with_memory(Coord::new(0, 1, 0), &[1]); // first
    w.insert_with_memory(Coord::new(1, 9, 0), &[1]); // y bumps
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
    w.insert_with_memory(Coord::new(0, 0, 1), &[1]); // first
    w.insert_with_memory(Coord::new(1, 0, 9), &[1]); // z bumps
    let bb = w.bounding_box().unwrap();
    assert_eq!(bb.5, 9, "expected z_max = 9, got {bb:?}");
}

#[test]
fn is_empty_returns_false_when_world_has_cells() {
    // A world with at least one cell must not report empty. Pins down the
    // truthful return value so an `is_empty -> true` mutation is caught.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::ORIGIN, &[1]);
    assert!(!w.is_empty());
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
    assert_ne!(&ca.memory(a.arena())[..3], &cb.memory(b.arena())[..3]);
    // Suffixes match — RNG advanced identically.
    assert_eq!(&ca.memory(a.arena())[3..], &cb.memory(b.arena())[3..]);
}

#[test]
fn big_bang_with_program_truncates_oversized() {
    let program: Vec<u32> = (0..100).collect();
    let w = SparseWorld::big_bang_with_program(0, 5, &program);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.memory_len(), 5);
    assert_eq!(cell.memory(w.arena()), vec![0, 1, 2, 3, 4]);
}

#[test]
fn big_bang_with_empty_program_matches_big_bang() {
    let a = SparseWorld::big_bang(123, 32);
    let b = SparseWorld::big_bang_with_program(123, 32, &[]);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_eq!(ca.memory(a.arena()), cb.memory(b.arena()));
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
    w.insert_with_memory(c, &[1]);
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
    // After the arena refactor, slot-resize ops on a cell require
    // `&mut arena` too — which `w.get_mut(c)` doesn't give the
    // caller (it holds `&mut w.cells`). What `get_mut` still does
    // is hand back `&mut Cell` for fixed-size field changes; verify
    // that path with a scalar mutation (`pc`) rather than memory
    // growth. Memory growth from outside the tick path uses
    // `w.alloc_cell` / `w.insert_with_memory` instead.
    let mut w = SparseWorld::new(0);
    let c = Coord::new(0, 0, 0);
    w.insert_with_memory(c, &[1, 2, 3]);
    w.get_mut(c).unwrap().pc = 7;
    assert_eq!(w.get(c).unwrap().pc, 7);
    assert_eq!(w.get(c).unwrap().memory(w.arena()), vec![1, 2, 3]);
}

#[test]
fn insert_returns_previous_cell_on_replace() {
    // After the arena refactor (Phase 2), `insert` frees the replaced
    // cell's arena range before returning the metadata — so `prev`
    // comes back with `mem_len = 0` and the slot data is no longer
    // addressable through the world. What the test still guarantees:
    // *some* metadata is returned (not `None`), and the new cell is
    // installed at the coord with the expected contents.
    let mut w = SparseWorld::new(0);
    let c = Coord::new(0, 0, 0);
    w.insert_with_memory(c, &[1]);
    let prev = w
        .insert_with_memory(c, &[9, 9])
        .expect("expected previous");
    assert!(prev.is_empty(), "freed prev should report empty memory");
    assert_eq!(w.get(c).unwrap().memory_len(), 2);
    assert_eq!(w.cell_memory(c).unwrap(), &[9, 9]);
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
    w.insert_with_memory(Coord::new(1, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(2, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(3, 0, 0), &[1]);
    assert_eq!(w.len(), 3);
    w.remove(Coord::new(2, 0, 0));
    assert_eq!(w.len(), 2);
}

// ----- neighbor / neighbor_energy -----

#[test]
fn neighbor_returns_none_for_missing_cell() {
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::ORIGIN, &[1]);
    assert!(w.neighbor(Coord::ORIGIN, Direction::Xp).is_none());
}

#[test]
fn neighbor_returns_some_for_existing_neighbor() {
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::ORIGIN, &[1]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[2, 3]);
    let n = w
        .neighbor(Coord::ORIGIN, Direction::Xp)
        .expect("neighbor should exist");
    assert_eq!(n.memory(w.arena()), vec![2, 3]);
}

#[test]
fn neighbor_works_for_all_six_directions() {
    let mut w = SparseWorld::new(0);
    let center = Coord::new(0, 0, 0);
    for &d in &Direction::ALL {
        w.insert_with_memory(center.neighbor(d), &[d.index() as u32]);
    }
    for &d in &Direction::ALL {
        let n_coord = center.neighbor(d);
        assert!(
            w.neighbor(center, d).is_some(),
            "missing neighbor {d:?}",
        );
        assert_eq!(w.cell_memory(n_coord).unwrap(), &[d.index() as u32]);
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
    w.insert_with_memory(Coord::new(1, 0, 0), &[1, 2, 3, 4, 5]);
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
    w.insert_with_memory(Coord::new(0, 0, 0), &[1; 3]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[1; 5]);
    w.insert_with_memory(Coord::new(0, 1, 0), &[1; 7]);
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
    w.insert_with_memory(Coord::new(1, 0, 0), &[1]); // non-empty
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
    w.insert_with_memory(Coord::new(0, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[1, 2]);
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
fn sorted_iter_walks_cells_in_canonical_order() {
    let mut w = SparseWorld::new(0);
    // Insert in reverse-canonical order; sorted_iter must still yield
    // them in `(x, y, z)` lex order regardless of insertion sequence
    // and regardless of the underlying FxHashMap's hash order.
    w.insert_with_memory(Coord::new(2, 0, 0), &[3]);
    w.insert_with_memory(Coord::new(0, 1, 0), &[2]);
    w.insert_with_memory(Coord::new(0, 0, 1), &[1]);
    // Tests that mutate outside the tick loop have to refresh the
    // sorted/bbox cache themselves before reading — the tick loop
    // does this after `gc_empty`, but a bare `insert` does not.
    w.rebuild_indices_if_dirty();

    let collected: Vec<Coord> = w.sorted_iter().map(|(c, _)| *c).collect();
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
fn coords_yields_keys_independent_of_order() {
    // After the FxHashMap switch, `coords()` no longer guarantees lex
    // order — only that every inserted key appears exactly once. The
    // sort happens at the snapshot boundary or via `sorted_iter`.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(2, 0, 0), &[3]);
    w.insert_with_memory(Coord::new(0, 1, 0), &[2]);
    w.insert_with_memory(Coord::new(0, 0, 1), &[1]);

    let mut keys: Vec<Coord> = w.coords().copied().collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
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
    w.insert_with_memory(Coord::new(0, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[2, 3]);

    let pairs: Vec<(Coord, u32)> = w.iter().map(|(c, cell)| (*c, cell.energy())).collect();
    assert_eq!(
        pairs,
        vec![(Coord::new(0, 0, 0), 1), (Coord::new(1, 0, 0), 2)]
    );
}

#[test]
fn iter_mut_allows_modification() {
    // Uses `iter_mut().for_each(...)` rather than `for (_, c) in
    // w.iter_mut()` so this exercises the method form of `iter_mut`.
    // The `for (_, c) in &mut world` form is covered by
    // `into_iter_for_mut_reference_*`. After the arena refactor the
    // mutation has to stay within fields the cell owns (no slot
    // resize, since that needs `&mut arena` too) — use `pc` here.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(0, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[2]);

    w.iter_mut().for_each(|(_, cell)| cell.pc = 99);

    assert_eq!(w.get(Coord::new(0, 0, 0)).unwrap().pc, 99);
    assert_eq!(w.get(Coord::new(1, 0, 0)).unwrap().pc, 99);
}

#[test]
fn into_iter_for_shared_reference_walks_pairs() {
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(0, 0, 0), &[1, 2]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[3]);
    let energies: Vec<u32> = (&w).into_iter().map(|(_, cell)| cell.energy()).collect();
    assert_eq!(energies, vec![2, 1]);
}

#[test]
fn into_iter_for_mut_reference_allows_modification() {
    // Same constraint as `iter_mut_allows_modification`: arena
    // ownership means slot-resize through iter_mut would require a
    // simultaneous `&mut arena` borrow. Stay within scalar fields.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(0, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[2]);
    for (_, cell) in &mut w {
        cell.pc = 99;
    }
    assert_eq!(w.get(Coord::new(0, 0, 0)).unwrap().pc, 99);
    assert_eq!(w.get(Coord::new(1, 0, 0)).unwrap().pc, 99);
}

#[test]
fn coords_yields_just_inserted_keys() {
    // `coords()` returns hash order — only check the key set, not the
    // sequence. A separate test (`sorted_iter_walks_cells_in_canonical_order`)
    // covers the canonical-order contract.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(5, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(2, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(3, 0, 0), &[1]);
    let keys: std::collections::HashSet<Coord> = w.coords().copied().collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&Coord::new(2, 0, 0)));
    assert!(keys.contains(&Coord::new(3, 0, 0)));
    assert!(keys.contains(&Coord::new(5, 0, 0)));
}

// ----- sorted index + bbox cache (see docs/optimalizace-2026-05.md) -----

#[test]
fn big_bang_initializes_cache() {
    // The big-bang path bypasses `insert` and pokes `cells` directly,
    // so it has to seed the indices itself; otherwise `sorted_iter` /
    // `bounding_box` before the first tick would read stale state.
    let w = SparseWorld::big_bang(7, 8);
    let collected: Vec<Coord> = w.sorted_iter().map(|(c, _)| *c).collect();
    assert_eq!(collected, vec![Coord::ORIGIN]);
    assert_eq!(w.bounding_box(), Some((0, 0, 0, 0, 0, 0)));
}

#[test]
fn bbox_extends_on_get_or_alloc_outside_current_box() {
    // `get_or_alloc` is on the hot path of `apply_outflow`; an
    // incremental bbox extend lets us skip the per-tick `O(n)` fold.
    let mut w = SparseWorld::big_bang(0, 4);
    w.get_or_alloc(Coord::new(5, -3, 7));
    // Eager extend — no `rebuild_indices_if_dirty` between mutation
    // and read. Both axes must stretch on the right side.
    assert_eq!(w.bounding_box(), Some((0, 5, -3, 0, 0, 7)));
}

#[test]
fn bbox_extends_on_insert_outside_current_box() {
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(0, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(-4, 2, -1), &[1]);
    w.insert_with_memory(Coord::new(3, -5, 6), &[1]);
    assert_eq!(w.bounding_box(), Some((-4, 3, -5, 2, -1, 6)));
}

#[test]
fn sorted_iter_after_insert_remove_cycle_is_lex_ordered() {
    // Drive a pseudo-random mutation sequence through the side-table
    // and check that the canonical-order contract still holds. Using
    // a fixed xorshift32 stream keeps the test deterministic.
    let mut w = SparseWorld::new(0);
    let mut rng: u32 = 0x1234_5678;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    for _ in 0..200 {
        // `next() % 21` is in `0..=20` — fits in i32 without wrap.
        let x = i32::try_from(next() % 21).unwrap() - 10;
        let y = i32::try_from(next() % 21).unwrap() - 10;
        let z = i32::try_from(next() % 21).unwrap() - 10;
        let coord = Coord::new(x, y, z);
        match next() % 3 {
            0 => {
                w.insert_with_memory(coord, &[1]);
            }
            1 => {
                w.get_or_alloc(coord);
            }
            _ => {
                w.remove(coord);
            }
        }
    }
    w.rebuild_indices_if_dirty();
    let collected: Vec<Coord> = w.sorted_iter().map(|(c, _)| *c).collect();
    let mut expected = collected.clone();
    expected.sort_unstable();
    assert_eq!(collected, expected);
    assert_eq!(collected.len(), w.len());
}

#[test]
fn bbox_invariant_matches_naive_after_mutations() {
    // After every rebuild, the cached bbox must match a fresh fold
    // over `cells.keys()`. This catches both stale-extend bugs and
    // missing dirty flags.
    let mut w = SparseWorld::big_bang(0, 1);
    w.insert_with_memory(Coord::new(4, -2, 6), &[1]);
    w.insert_with_memory(Coord::new(-7, 3, -8), &[1]);
    w.remove(Coord::ORIGIN);
    w.rebuild_indices_if_dirty();
    let naive = w.coords().fold(None, |acc, c| {
        Some(acc.map_or(
            (c.x, c.x, c.y, c.y, c.z, c.z),
            |(xmn, xmx, ymn, ymx, zmn, zmx): (i32, i32, i32, i32, i32, i32)| {
                (
                    xmn.min(c.x),
                    xmx.max(c.x),
                    ymn.min(c.y),
                    ymx.max(c.y),
                    zmn.min(c.z),
                    zmx.max(c.z),
                )
            },
        ))
    });
    assert_eq!(w.bounding_box(), naive);
}

#[test]
fn bbox_recomputes_after_gc_clears_extremes() {
    // gc_empty drops every empty cell; if the cell that anchored
    // x_min is among them, bbox has to shrink. An incremental
    // extend-only update would leave x_min stale.
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(-10, 0, 0), Cell::new()); // empty → gc target
    w.insert_with_memory(Coord::new(10, 0, 0), &[1]);
    w.gc_empty();
    w.rebuild_indices_if_dirty();
    let bb = w.bounding_box().unwrap();
    assert_eq!(bb.0, 10, "x_min must shrink to 10, got {bb:?}");
    assert_eq!(bb.1, 10);
}

#[test]
fn bbox_is_none_after_gc_clears_all() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::new(1, 2, 3), Cell::new());
    w.insert(Coord::new(-1, -2, -3), Cell::new());
    w.gc_empty();
    w.rebuild_indices_if_dirty();
    assert_eq!(w.bounding_box(), None);
    assert_eq!(w.sorted_iter().count(), 0);
}

#[test]
fn clone_preserves_cache_validity() {
    // Derived Clone copies the Vec + Option fields; the clone must
    // therefore read its sorted_iter / bounding_box without an extra
    // rebuild — the invariant carries.
    let mut w = SparseWorld::big_bang(0, 4);
    w.insert_with_memory(Coord::new(2, 0, 0), &[1]);
    w.rebuild_indices_if_dirty();
    let clone = w.clone();
    let collected: Vec<Coord> = clone.sorted_iter().map(|(c, _)| *c).collect();
    assert_eq!(collected, vec![Coord::ORIGIN, Coord::new(2, 0, 0)]);
    assert_eq!(clone.bounding_box(), Some((0, 2, 0, 0, 0, 0)));
}

#[test]
fn insert_replace_does_not_invalidate_sorted_cache() {
    // Replacing the value at an existing key leaves the keyset
    // identical, so sorted_cache should stay valid — no rebuild
    // needed before the next read. This pins the optimization so a
    // "set dirty on every insert" mutant gets caught.
    let mut w = SparseWorld::new(0);
    w.insert_with_memory(Coord::new(1, 0, 0), &[1]);
    w.insert_with_memory(Coord::new(2, 0, 0), &[1]);
    w.rebuild_indices_if_dirty();
    let cache_before: Vec<Coord> = w.sorted_iter().map(|(c, _)| *c).collect();
    // Replace at an existing coord — should NOT dirty the cache.
    w.insert_with_memory(Coord::new(1, 0, 0), &[99]);
    // Read without an intervening rebuild — relies on the cache
    // still being valid.
    let cache_after: Vec<Coord> = w.sorted_iter().map(|(c, _)| *c).collect();
    assert_eq!(cache_before, cache_after);
}

//! Integration tests for [`SparseWorld::possess`] — the energy-neutral
//! "load a program into an existing cell" tool that Project Pilgrim uses
//! to inject an entity into a running world (see `docs/pilgrim.md`).

use aenternis_core::{Coord, PossessError, SparseWorld};

/// World with a single host cell at the origin holding `slots`, pre-stamped
/// with distinctive metadata so a passing test can't be a coincidence of
/// zeros (and so we can prove possess overwrites / leaves it alone).
fn world_with_host(slots: &[u32]) -> (SparseWorld, Coord) {
    let mut w = SparseWorld::new(0xABCD);
    let at = Coord::new(0, 0, 0);
    w.insert_with_memory(at, slots);
    let cell = w.get_mut(at).expect("host just inserted");
    cell.origin_tag = 0xDEAD_BEEF;
    cell.appearance = 0x0BAD_F00D;
    cell.pc = 5;
    (w, at)
}

#[test]
fn possess_overwrites_prefix_preserves_tail_and_metadata() {
    let (mut w, at) = world_with_host(&[10, 11, 12, 13, 14, 15, 16, 17]);
    let before = w.total_energy();

    w.possess(at, &[0xAA, 0xBB, 0xCC], 0x1234, 0x5678)
        .expect("3-slot code fits in an 8-slot host");

    // Leading slots replaced; trailing slots kept verbatim.
    assert_eq!(
        w.cell_memory(at).unwrap(),
        &[0xAA, 0xBB, 0xCC, 13, 14, 15, 16, 17],
    );
    let cell = w.get(at).unwrap();
    assert_eq!(cell.origin_tag, 0x1234, "tag stamped");
    assert_eq!(cell.appearance, 0x5678, "appearance stamped");
    assert_eq!(cell.pc, 0, "pc reset to program entry");
    assert_eq!(cell.energy(), 8, "mem_len (= energy) unchanged");
    assert_eq!(w.total_energy(), before, "energy conserved");
}

#[test]
fn possess_full_overwrite_leaves_no_tail() {
    // code_len == capacity is the boundary case: it must succeed (guards
    // the `>` vs `>=` comparison).
    let (mut w, at) = world_with_host(&[1, 2, 3, 4]);
    w.possess(at, &[9, 8, 7, 6], 1, 2)
        .expect("code_len == capacity is allowed");
    assert_eq!(w.cell_memory(at).unwrap(), &[9, 8, 7, 6]);
}

#[test]
fn possess_empty_code_only_stamps_metadata() {
    let (mut w, at) = world_with_host(&[1, 2, 3]);
    w.possess(at, &[], 7, 8).expect("empty code is allowed");
    assert_eq!(w.cell_memory(at).unwrap(), &[1, 2, 3], "memory untouched");
    let cell = w.get(at).unwrap();
    assert_eq!(cell.origin_tag, 7);
    assert_eq!(cell.appearance, 8);
    assert_eq!(cell.pc, 0);
}

#[test]
fn possess_rejects_missing_cell_and_conjures_nothing() {
    let (mut w, _) = world_with_host(&[1, 2, 3]);
    let empty = Coord::new(9, 9, 9);
    assert_eq!(w.possess(empty, &[1], 0, 0), Err(PossessError::NoCell));
    assert!(
        w.get(empty).is_none(),
        "no cell conjured at the empty coord"
    );
}

#[test]
fn possess_rejects_oversized_code_and_changes_nothing() {
    let (mut w, at) = world_with_host(&[1, 2, 3]);
    let before_energy = w.total_energy();

    assert_eq!(
        w.possess(at, &[9, 9, 9, 9], 0x1111, 0x2222),
        Err(PossessError::CodeTooLarge {
            code_len: 4,
            capacity: 3,
        }),
    );

    // The early bail-out must leave content AND metadata exactly as before.
    assert_eq!(w.cell_memory(at).unwrap(), &[1, 2, 3], "content untouched");
    let cell = w.get(at).unwrap();
    assert_eq!(cell.origin_tag, 0xDEAD_BEEF, "tag untouched on reject");
    assert_eq!(
        cell.appearance, 0x0BAD_F00D,
        "appearance untouched on reject"
    );
    assert_eq!(cell.pc, 5, "pc untouched on reject");
    assert_eq!(
        w.total_energy(),
        before_energy,
        "energy untouched on reject"
    );
}

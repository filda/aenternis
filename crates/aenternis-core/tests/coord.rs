//! Integration tests for the coord module.
//!
//! These run against the public API (`use aenternis_core::...`), so they
//! also act as a smoke test that the crate's surface matches expectations.

use aenternis_core::{Coord, Direction};

#[test]
fn origin_is_all_zero() {
    assert_eq!(Coord::ORIGIN, Coord::new(0, 0, 0));
    assert_eq!(Coord::ORIGIN.x, 0);
    assert_eq!(Coord::ORIGIN.y, 0);
    assert_eq!(Coord::ORIGIN.z, 0);
}

#[test]
fn new_constructs_from_components() {
    let c = Coord::new(-3, 7, 42);
    assert_eq!(c.x, -3);
    assert_eq!(c.y, 7);
    assert_eq!(c.z, 42);
}

#[test]
fn neighbor_in_each_direction() {
    let c = Coord::new(5, 7, 3);
    assert_eq!(c.neighbor(Direction::Xp), Coord::new(6, 7, 3));
    assert_eq!(c.neighbor(Direction::Xn), Coord::new(4, 7, 3));
    assert_eq!(c.neighbor(Direction::Yp), Coord::new(5, 8, 3));
    assert_eq!(c.neighbor(Direction::Yn), Coord::new(5, 6, 3));
    assert_eq!(c.neighbor(Direction::Zp), Coord::new(5, 7, 4));
    assert_eq!(c.neighbor(Direction::Zn), Coord::new(5, 7, 2));
}

#[test]
fn round_trip_across_each_face() {
    let c = Coord::new(10, -20, 30);
    for &d in &Direction::ALL {
        let n = c.neighbor(d);
        let back = n.neighbor(d.opposite());
        assert_eq!(back, c, "round trip failed for {d:?}");
    }
}

#[test]
fn opposite_is_self_inverse() {
    for &d in &Direction::ALL {
        assert_eq!(d.opposite().opposite(), d);
    }
}

#[test]
fn opposite_pairs_match_xyz_axis() {
    assert_eq!(Direction::Xp.opposite(), Direction::Xn);
    assert_eq!(Direction::Xn.opposite(), Direction::Xp);
    assert_eq!(Direction::Yp.opposite(), Direction::Yn);
    assert_eq!(Direction::Yn.opposite(), Direction::Yp);
    assert_eq!(Direction::Zp.opposite(), Direction::Zn);
    assert_eq!(Direction::Zn.opposite(), Direction::Zp);
}

#[test]
fn canonical_direction_order() {
    // The order is load-bearing for deterministic iteration. If this test
    // fails, every snapshot test in the project will fail too.
    assert_eq!(
        Direction::ALL,
        [
            Direction::Xp,
            Direction::Xn,
            Direction::Yp,
            Direction::Yn,
            Direction::Zp,
            Direction::Zn,
        ]
    );
}

#[test]
fn direction_index_matches_repr() {
    for (i, &d) in Direction::ALL.iter().enumerate() {
        assert_eq!(d.index(), i);
    }
}

#[test]
fn delta_components_match_neighbor() {
    let origin = Coord::ORIGIN;
    for &d in &Direction::ALL {
        let n = origin.neighbor(d);
        let (dx, dy, dz) = d.delta();
        assert_eq!(n, Coord::new(dx, dy, dz));
    }
}

#[test]
fn coords_are_hashable_and_orderable() {
    use std::collections::{BTreeSet, HashSet};

    let coords = [
        Coord::new(0, 0, 0),
        Coord::new(1, 0, 0),
        Coord::new(0, 1, 0),
        Coord::new(0, 0, 1),
    ];

    let hash: HashSet<Coord> = coords.iter().copied().collect();
    let tree: BTreeSet<Coord> = coords.iter().copied().collect();
    assert_eq!(hash.len(), 4);
    assert_eq!(tree.len(), 4);
}

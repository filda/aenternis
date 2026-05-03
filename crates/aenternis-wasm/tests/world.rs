//! Integration tests for the WASM wrapper. Run on the host target via
//! `cargo test --workspace` (no WASM toolchain involved).
//!
//! These don't exercise the `wasm-bindgen` JS glue — that needs
//! `wasm-pack test` against a browser. They verify the wrapper's Rust
//! surface: types, conversions, conservation invariants preserved
//! across the wrapper boundary.

use aenternis_wasm::World;

#[test]
fn new_constructs_world_with_initial_energy() {
    let w = World::new(42, 100);
    assert_eq!(w.total_energy(), 100);
    assert_eq!(w.cell_count(), 1);
    assert_eq!(w.tick(), 0);
}

#[test]
fn new_with_zero_energy_yields_empty_world() {
    let w = World::new(7, 0);
    assert_eq!(w.total_energy(), 0);
    assert_eq!(w.cell_count(), 0);
    assert_eq!(w.tick(), 0);
}

#[test]
fn step_advances_tick_by_one() {
    let mut w = World::new(42, 100);
    w.step(0.15, 1);
    assert_eq!(w.tick(), 1);
    w.step(0.15, 1);
    assert_eq!(w.tick(), 2);
}

#[test]
fn step_conserves_total_energy() {
    let mut w = World::new(0xDEAD_BEEF, 200);
    let before = w.total_energy();
    for _ in 0..30 {
        w.step(0.15, 1);
        assert_eq!(w.total_energy(), before);
    }
}

#[test]
fn cell_count_grows_after_first_tick() {
    let mut w = World::new(0, 100);
    assert_eq!(w.cell_count(), 1);
    w.step(0.30, 1);
    assert!(
        w.cell_count() > 1,
        "expected expansion, got {}",
        w.cell_count()
    );
}

#[test]
fn same_seed_produces_same_state_through_wrapper() {
    let mut a = World::new(2024, 100);
    let mut b = World::new(2024, 100);
    for _ in 0..20 {
        a.step(0.15, 1);
        b.step(0.15, 1);
    }
    // Reflected through the public surface: tick, total_energy, and
    // cell count must all match. (Full byte-identity goes via the
    // core SparseWorld, not the wrapper.)
    assert_eq!(a.tick(), b.tick());
    assert_eq!(a.total_energy(), b.total_energy());
    assert_eq!(a.cell_count(), b.cell_count());
}

// ----- cells_snapshot -----

#[test]
fn snapshot_is_empty_for_empty_world() {
    let w = World::new(0, 0);
    assert!(w.cells_snapshot().is_empty());
}

#[test]
fn snapshot_length_is_stride_times_cell_count() {
    let w = World::new(7, 16);
    let snap = w.cells_snapshot();
    assert_eq!(snap.len(), w.cell_count() as usize * 6);
    assert_eq!(snap.len(), 6); // single big-bang cell
}

#[test]
fn snapshot_first_cell_after_big_bang_is_at_origin() {
    let w = World::new(7, 16);
    let snap = w.cells_snapshot();
    // First cell: x=0, y=0, z=0 (origin), then energy, origin_tag, appearance.
    assert_eq!(snap[0], 0); // x
    assert_eq!(snap[1], 0); // y
    assert_eq!(snap[2], 0); // z
    assert_eq!(snap[3], 16); // energy
    assert_ne!(snap[4], 0); // origin_tag should be a real PCG output
    assert_eq!(snap[5], 0); // appearance defaults to 0
}

#[test]
fn snapshot_total_energy_matches_world_total_energy() {
    let mut w = World::new(0xDEAD_BEEF, 200);
    for _ in 0..10 {
        w.step(0.20, 1);
    }
    let snap = w.cells_snapshot();
    let energy_sum: u64 = snap.chunks_exact(6).map(|cell| u64::from(cell[3])).sum();
    assert_eq!(energy_sum, u64::from(w.total_energy()));
}

#[test]
fn snapshot_is_deterministic_for_same_seed() {
    let a = World::new(42, 50);
    let b = World::new(42, 50);
    assert_eq!(a.cells_snapshot(), b.cells_snapshot());
}

#[test]
fn snapshot_walks_cells_in_canonical_order_after_expansion() {
    let mut w = World::new(0, 100);
    w.step(0.30, 1); // expand outward
    let snap = w.cells_snapshot();

    // Decode (x, y, z) for every cell and check lexicographic order.
    // The `as i32` casts are intentional bit-reinterpretations — the
    // snapshot stores i32 coords as u32 bits, and we're recovering
    // them. clippy::cast_possible_wrap is the right warning *for
    // unintended* wraps; here it's the whole point.
    #[allow(clippy::cast_possible_wrap)]
    let decode = |chunk: &[u32]| (chunk[0] as i32, chunk[1] as i32, chunk[2] as i32);

    let mut prev: Option<(i32, i32, i32)> = None;
    for chunk in snap.chunks_exact(6) {
        let coord = decode(chunk);
        if let Some(p) = prev {
            assert!(
                p < coord,
                "snapshot coords out of canonical order: {p:?} >= {coord:?}"
            );
        }
        prev = Some(coord);
    }
}

#[test]
fn snapshot_stride_getter_returns_six() {
    let w = World::new(0, 0);
    assert_eq!(w.snapshot_stride(), 6);
}

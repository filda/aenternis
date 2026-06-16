//! Canonical flat-`u32` projections of world state for the viewer.
//!
//! Both runtime backends — the WASM wrapper (`aenternis-wasm`) and the
//! native server (`aenternis-server`) — feed the **same** TypeScript
//! viewer, so the cell layout they emit must be byte-for-byte
//! identical. Keeping that layout here, in the core, makes it a single
//! source of truth: neither backend re-derives it, so the two cannot
//! drift apart. The WASM wrapper exposes these directly over its
//! zero-copy `Uint32Array` views; the server wraps the same payload in
//! a binary frame header (see `aenternis-server::protocol`). Both
//! re-export [`SNAPSHOT_STRIDE`] / [`INSPECT_PREFIX`] rather than
//! redefining the values.

use crate::{Coord, SparseWorld};

/// Number of `u32` fields per cell in [`snapshot_into`]:
/// `[x, y, z, energy, origin_tag, appearance]`.
pub const SNAPSHOT_STRIDE: usize = 6;

/// Number of `u32` fields in [`inspect_into`]'s fixed-width prefix,
/// before the variable-length memory tail.
///
/// 4 scalars (`pc, energy, origin_tag, appearance`) + 4 × 6
/// directional arrays (`pointers, rates, active_outflow, inflow`) = 28.
pub const INSPECT_PREFIX: usize = 28;

/// Fill `out` with the flat snapshot payload — one
/// `[x, y, z, energy, origin_tag, appearance]` group per cell, in
/// `(x, y, z)` lexicographic order.
///
/// `out` is cleared first and re-filled in place; capacity is retained
/// across calls, so a persistent scratch buffer amortizes the
/// allocation away after the first peak-sized world.
pub fn snapshot_into(world: &SparseWorld, out: &mut Vec<u32>) {
    out.clear();
    out.reserve(world.len() * SNAPSHOT_STRIDE);
    // `sorted_iter` walks cells in `(x, y, z)` lex order — the
    // snapshot's documented contract. The world's internal storage
    // iterates in hash order, which is deterministic but not lex.
    for (coord, cell) in world.sorted_iter() {
        out.push(coord.x as u32);
        out.push(coord.y as u32);
        out.push(coord.z as u32);
        out.push(cell.energy());
        out.push(cell.origin_tag);
        out.push(cell.appearance);
    }
}

/// Fill `out` with the cellDetail payload for the cell at `coord`:
/// `[pc, energy, origin_tag, appearance, pointers[6], rates[6],
/// active_outflow[6], inflow[6], memory[E]]`.
///
/// `out` is cleared first and left **empty** when no cell exists at
/// `coord`, so the total length is either `0` or
/// `INSPECT_PREFIX + memory_len`. The six-element directional arrays
/// use the canonical `[xp, xn, yp, yn, zp, zn]` order.
pub fn inspect_into(world: &SparseWorld, coord: Coord, out: &mut Vec<u32>) {
    out.clear();
    let Some(cell) = world.get(coord) else {
        return;
    };
    out.reserve(INSPECT_PREFIX + cell.memory_len());
    out.push(cell.pc);
    out.push(cell.energy());
    out.push(cell.origin_tag);
    out.push(cell.appearance);
    out.extend_from_slice(&cell.pointers);
    out.extend_from_slice(&cell.rates);
    out.extend_from_slice(&cell.active_outflow);
    out.extend_from_slice(&cell.inflow);
    if let Some(memory) = world.cell_memory(coord) {
        out.extend_from_slice(memory);
    }
}

#[cfg(test)]
mod tests {
    use super::{inspect_into, snapshot_into, INSPECT_PREFIX, SNAPSHOT_STRIDE};
    use crate::{Coord, SparseWorld};

    /// Pin the wire constants. The TypeScript viewer derives its frame
    /// header lengths from these (snapshot header = 49, cellDetail
    /// header = 25) and unpacks payloads with stride 6 / prefix 28, so
    /// a change here must be a conscious, cross-language edit.
    #[test]
    fn wire_constants_are_pinned() {
        assert_eq!(SNAPSHOT_STRIDE, 6);
        assert_eq!(INSPECT_PREFIX, 28);
    }

    #[test]
    fn snapshot_layout_and_lex_order() {
        let world = SparseWorld::big_bang(1234, 100);
        let mut out = vec![0xdead_beef]; // pre-seed to prove `clear`.
        snapshot_into(&world, &mut out);

        assert!(!out.is_empty(), "big_bang must produce >= 1 cell");
        assert_eq!(
            out.len() % SNAPSHOT_STRIDE,
            0,
            "payload is a whole number of cells"
        );
        assert_eq!(out.len() / SNAPSHOT_STRIDE, world.len());

        // Field-for-field, the payload must equal the same projection
        // taken straight off `sorted_iter` — same order, same fields.
        // (`sorted_iter`'s lex-ordering guarantee is covered in the
        // world module; here we only assert the payload mirrors it.)
        let expected: Vec<u32> = world
            .sorted_iter()
            .flat_map(|(coord, cell)| {
                [
                    coord.x as u32,
                    coord.y as u32,
                    coord.z as u32,
                    cell.energy(),
                    cell.origin_tag,
                    cell.appearance,
                ]
            })
            .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn inspect_layout_for_present_cell() {
        let world = SparseWorld::big_bang(1, 100);
        let origin = Coord::new(0, 0, 0);
        let cell = world.get(origin).expect("origin cell exists");

        let mut out = vec![0xdead_beef];
        inspect_into(&world, origin, &mut out);

        assert_eq!(out.len(), INSPECT_PREFIX + cell.memory_len());
        assert_eq!(out[0], cell.pc);
        assert_eq!(out[1], cell.energy());
        assert_eq!(out[2], cell.origin_tag);
        assert_eq!(out[3], cell.appearance);
        assert_eq!(&out[4..10], &cell.pointers);
        assert_eq!(&out[10..16], &cell.rates);
        assert_eq!(&out[16..22], &cell.active_outflow);
        assert_eq!(&out[22..28], &cell.inflow);
        if let Some(memory) = world.cell_memory(origin) {
            assert_eq!(&out[INSPECT_PREFIX..], memory);
        }
    }

    #[test]
    fn inspect_empty_for_absent_cell() {
        let world = SparseWorld::big_bang(1, 100);
        let mut out = vec![1, 2, 3];
        inspect_into(&world, Coord::new(99_999, 0, 0), &mut out);
        assert!(out.is_empty(), "absent cell → empty payload");
    }
}

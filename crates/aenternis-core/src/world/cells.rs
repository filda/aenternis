//! Slot-indexed cell storage with `FxHashMap`-compatible iteration API.
//!
//! Replaces the old `FxHashMap<Coord, Cell>` direct storage with a
//! `(Coord, usize)`-keyed hashmap of slot indices into a dense `Vec`
//! of `(Coord, Cell)` pairs. The motivation is cache locality on the
//! coord-lookup hot path: hashmap buckets shrink from ~150 B (12 B
//! `Coord` + ~138 B inline `Cell`) to ~20 B (12 B `Coord` + 8 B
//! `usize`), so the hashmap working set fits much more comfortably
//! in L1/L2 on dense worlds.
//!
//! The wrapper deliberately mirrors the `FxHashMap` subset that the
//! tick / world code already used (`get`, `get_mut`, `insert`,
//! `remove`, `contains_key`, `len`, `is_empty`, `keys`, `iter`,
//! `iter_mut`, `values_mut`, `retain`, plus rayon-parallel
//! `par_iter_mut`). That keeps the migration mostly mechanical at
//! the callsite level: `world.cells.get(&c)` style accesses keep
//! working unchanged.
//!
//! ## Slot recycling
//!
//! Slot indices are stable for the lifetime of a cell. Removing a
//! cell pushes its slot to `free_slots`; the next `insert` for a new
//! coord pops from there (LIFO) before growing `slots`. This keeps
//! the slot vector dense — wasted slots from deletion churn are
//! reclaimed immediately rather than fragmenting the storage.
//!
//! ## Iteration order
//!
//! `iter` / `keys` walk in `coord_to_slot`'s hash order (same as the
//! old `FxHashMap` did). `iter_mut`, `values_mut`, and
//! `par_iter_mut` walk in **slot order** — that's a behaviour
//! change from the old hashmap, but the per-cell closures in
//! [`crate::tick`] are all order-independent (each cell reads its
//! own state plus a read-only neighbor snapshot, writes only its
//! own state), so bit-parity is preserved. `dump_state_for_diff`
//! sorts by `(x, y, z)` at the boundary regardless of iteration
//! order, so the diff harness is unaffected.

use rustc_hash::{FxBuildHasher, FxHashMap};

// Rayon prelude is needed on every target where `par_iter_mut` is
// callable — native unconditionally, plus wasm32 with `wasm-threads`
// (via `wasm-bindgen-rayon`). Keep the cfg in lockstep with
// `par_iter_mut`'s own gate below.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
use rayon::prelude::*;

use crate::{Cell, Coord};

/// Slot-indexed cell container. See module docs for the layout
/// reasoning and the FxHashMap-compat API surface.
#[derive(Debug, Clone)]
pub(crate) struct Cells {
    /// Live coord → slot index in `slots`. Same hasher as the old
    /// direct hashmap, so iteration order is identical for the same
    /// insertion sequence.
    coord_to_slot: FxHashMap<Coord, usize>,

    /// Dense per-slot storage. `Some(...)` for live slots; `None`
    /// for slots whose cell was removed but not yet recycled. Slot
    /// indices are stable across the cell's lifetime.
    slots: Vec<Option<(Coord, Cell)>>,

    /// LIFO stack of recyclable slot indices. Pop here before
    /// extending `slots` on a new insert.
    free_slots: Vec<usize>,
}

impl Cells {
    /// Build an empty container. No allocations until the first insert.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            coord_to_slot: FxHashMap::with_hasher(FxBuildHasher),
            slots: Vec::new(),
            free_slots: Vec::new(),
        }
    }

    /// Build an empty container pre-sized for up to `capacity` live
    /// cells. The `coord_to_slot` map and the slot vector both
    /// reserve `capacity` entries up front, so once a world fills
    /// to its energy-bounded peak the storage backing the cells
    /// never has to grow.
    #[must_use]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            coord_to_slot: FxHashMap::with_capacity_and_hasher(capacity, FxBuildHasher),
            slots: Vec::with_capacity(capacity),
            free_slots: Vec::new(),
        }
    }

    /// Number of live cells.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.coord_to_slot.len()
    }

    /// `true` iff there are no live cells.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.coord_to_slot.is_empty()
    }

    /// `true` iff a live cell exists at `coord`.
    #[must_use]
    pub(crate) fn contains_key(&self, coord: &Coord) -> bool {
        self.coord_to_slot.contains_key(coord)
    }

    /// Borrow the cell at `coord`, if any.
    #[must_use]
    pub(crate) fn get(&self, coord: &Coord) -> Option<&Cell> {
        let &slot = self.coord_to_slot.get(coord)?;
        // `coord_to_slot` invariant: every key maps to a `Some` slot.
        self.slots[slot].as_ref().map(|(_, cell)| cell)
    }

    /// Mutably borrow the cell at `coord`, if any.
    pub(crate) fn get_mut(&mut self, coord: &Coord) -> Option<&mut Cell> {
        let &slot = self.coord_to_slot.get(coord)?;
        self.slots[slot].as_mut().map(|(_, cell)| cell)
    }

    /// Insert or replace the cell at `coord`. Returns the previous
    /// cell if one existed there, mirroring `FxHashMap::insert`.
    pub(crate) fn insert(&mut self, coord: Coord, cell: Cell) -> Option<Cell> {
        if let Some(&slot) = self.coord_to_slot.get(&coord) {
            // Replacement — slot index stays the same.
            let prev = self.slots[slot].take().map(|(_, c)| c);
            self.slots[slot] = Some((coord, cell));
            prev
        } else {
            let slot = self.free_slots.pop().unwrap_or_else(|| {
                let s = self.slots.len();
                self.slots.push(None);
                s
            });
            self.slots[slot] = Some((coord, cell));
            self.coord_to_slot.insert(coord, slot);
            None
        }
    }

    /// Remove the cell at `coord`, returning it if it was there.
    pub(crate) fn remove(&mut self, coord: &Coord) -> Option<Cell> {
        let slot = self.coord_to_slot.remove(coord)?;
        let (_, cell) = self.slots[slot].take()?;
        self.free_slots.push(slot);
        Some(cell)
    }

    /// Insert a fresh cell if no entry exists; return a mutable
    /// reference to the (possibly newly-allocated) cell either way.
    /// Returns `(was_vacant, &mut Cell)` so callers can react to the
    /// alloc-on-write case (e.g. bbox extension).
    pub(crate) fn get_or_insert_with(
        &mut self,
        coord: Coord,
        f: impl FnOnce() -> Cell,
    ) -> (bool, &mut Cell) {
        if let Some(&slot) = self.coord_to_slot.get(&coord) {
            let cell = self.slots[slot]
                .as_mut()
                .map(|(_, c)| c)
                .expect("coord_to_slot points to a live slot");
            (false, cell)
        } else {
            let slot = self.free_slots.pop().unwrap_or_else(|| {
                let s = self.slots.len();
                self.slots.push(None);
                s
            });
            self.slots[slot] = Some((coord, f()));
            self.coord_to_slot.insert(coord, slot);
            let cell = self.slots[slot]
                .as_mut()
                .map(|(_, c)| c)
                .expect("just inserted");
            (true, cell)
        }
    }

    /// Coordinates of all live cells, in hash order.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &Coord> + '_ {
        self.coord_to_slot.keys()
    }

    /// `(coord, cell)` pairs of all live cells, in hash order
    /// (matches the old `FxHashMap::iter` order).
    #[must_use]
    pub(crate) fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.coord_to_slot.iter(),
            slots: &self.slots,
        }
    }

    /// Mutable `(coord, cell)` pairs, in **slot order** (different
    /// from `iter` — see module docs). Skips dead slots.
    pub(crate) fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut {
            inner: self.slots.iter_mut(),
        }
    }

    /// Mutable cell references, in slot order. Skips dead slots.
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut Cell> + '_ {
        self.slots
            .iter_mut()
            .filter_map(|opt| opt.as_mut().map(|(_, c)| c))
    }

    /// Drop every cell where `predicate(coord, cell)` returns `false`.
    pub(crate) fn retain<F: FnMut(&Coord, &mut Cell) -> bool>(&mut self, mut predicate: F) {
        for (slot, opt) in self.slots.iter_mut().enumerate() {
            let Some((coord, cell)) = opt.as_mut() else {
                continue;
            };
            if !predicate(&*coord, cell) {
                let coord_copy = *coord;
                *opt = None;
                self.coord_to_slot.remove(&coord_copy);
                self.free_slots.push(slot);
            }
        }
    }

    /// Parallel mutable iteration over live cells in slot order.
    /// Mirrors the old `FxHashMap::par_iter_mut` semantics — order
    /// is unspecified but iteration is bit-parity-safe because the
    /// per-cell closures are order-independent.
    ///
    /// Available on every target where rayon ships: native targets
    /// unconditionally, plus wasm32 with the `wasm-threads` feature
    /// (via `wasm-bindgen-rayon`). The macro callsite in
    /// [`crate::parallel::par_or_seq_iter_mut!`] is gated to the same
    /// cfg, so callers never reach this method on a target that
    /// doesn't have it.
    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
    pub(crate) fn par_iter_mut(
        &mut self,
    ) -> impl ParallelIterator<Item = (&Coord, &mut Cell)> + '_ {
        self.slots.par_iter_mut().filter_map(|opt| {
            opt.as_mut().map(|(coord, cell)| {
                let coord_ref: &Coord = &*coord;
                (coord_ref, cell)
            })
        })
    }
}

impl Default for Cells {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable iterator over live `(coord, cell)` pairs in
/// `coord_to_slot` hash order. Returned by [`Cells::iter`].
pub struct Iter<'a> {
    inner: std::collections::hash_map::Iter<'a, Coord, usize>,
    slots: &'a Vec<Option<(Coord, Cell)>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a Coord, &'a Cell);

    fn next(&mut self) -> Option<Self::Item> {
        let (coord, &slot) = self.inner.next()?;
        let cell = self.slots[slot]
            .as_ref()
            .map(|(_, c)| c)
            .expect("coord_to_slot points to a live slot");
        Some((coord, cell))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for Iter<'_> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Mutable iterator over live `(coord, cell)` pairs in slot order.
/// Returned by [`Cells::iter_mut`]. Skips dead slots.
pub struct IterMut<'a> {
    inner: std::slice::IterMut<'a, Option<(Coord, Cell)>>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (&'a Coord, &'a mut Cell);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let opt = self.inner.next()?;
            if let Some((coord, cell)) = opt.as_mut() {
                return Some((&*coord, cell));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Arena;

    fn dummy_cell(arena: &mut Arena, seed: u32) -> Cell {
        Cell::with_memory(arena, &[seed, seed.wrapping_mul(2), seed.wrapping_mul(3)])
    }

    #[test]
    fn new_is_empty() {
        let c = Cells::new();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
    }

    #[test]
    fn insert_and_get_round_trip() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        let coord = Coord::new(1, 2, 3);
        let cell = dummy_cell(&mut arena, 10);
        assert!(c.insert(coord, cell.clone()).is_none());
        assert_eq!(c.get(&coord), Some(&cell));
        assert!(c.contains_key(&coord));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn insert_replacement_returns_previous() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        let coord = Coord::new(0, 0, 0);
        let a = dummy_cell(&mut arena, 1);
        let b = dummy_cell(&mut arena, 2);
        c.insert(coord, a.clone());
        let prev = c.insert(coord, b.clone());
        assert_eq!(prev, Some(a));
        assert_eq!(c.get(&coord), Some(&b));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn remove_returns_cell_and_frees_slot() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        let coord_a = Coord::new(0, 0, 0);
        let coord_b = Coord::new(1, 0, 0);
        let cell_a = dummy_cell(&mut arena, 7);
        let cell_b = dummy_cell(&mut arena, 8);
        c.insert(coord_a, cell_a.clone());
        c.insert(coord_b, cell_b.clone());

        let removed = c.remove(&coord_a);
        assert_eq!(removed, Some(cell_a));
        assert!(!c.contains_key(&coord_a));
        assert_eq!(c.get(&coord_b), Some(&cell_b));
        assert_eq!(c.len(), 1);
        // Slot 0 (coord_a) is recycled; next insert should land in it.
        let coord_c = Coord::new(2, 0, 0);
        let cell_c = dummy_cell(&mut arena, 9);
        c.insert(coord_c, cell_c);
        // coord_c reuses slot 0; coord_b is still slot 1.
        // Inspect via the public iter to verify both are present.
        let mut found: Vec<_> = c.iter().map(|(k, _)| *k).collect();
        found.sort_by_key(|c| (c.x, c.y, c.z));
        assert_eq!(found, vec![coord_b, coord_c]);
    }

    #[test]
    fn remove_missing_is_noop() {
        let mut c = Cells::new();
        let coord = Coord::new(5, 5, 5);
        let removed = c.remove(&coord);
        assert!(removed.is_none());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn get_or_insert_with_creates_then_returns_existing() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        let coord = Coord::new(3, 3, 3);
        let (was_vacant, cell) = {
            let arena_ref = &mut arena;
            c.get_or_insert_with(coord, || dummy_cell(arena_ref, 42))
        };
        assert!(was_vacant);
        // Distinct marker on a non-memory field so the second
        // `get_or_insert_with` for the same coord returns the *same*
        // entry, not a freshly-constructed `dummy_cell(0)`.
        cell.origin_tag = 0x1234;
        let (was_vacant_again, cell) = {
            let arena_ref = &mut arena;
            c.get_or_insert_with(coord, || dummy_cell(arena_ref, 0))
        };
        assert!(!was_vacant_again);
        assert_eq!(cell.origin_tag, 0x1234);
    }

    #[test]
    fn retain_drops_unwanted_cells() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        c.insert(Coord::new(1, 0, 0), dummy_cell(&mut arena, 1));
        c.insert(Coord::new(2, 0, 0), dummy_cell(&mut arena, 2));
        c.insert(Coord::new(3, 0, 0), dummy_cell(&mut arena, 3));
        c.retain(|coord, _| coord.x != 2);
        assert_eq!(c.len(), 2);
        assert!(c.contains_key(&Coord::new(1, 0, 0)));
        assert!(!c.contains_key(&Coord::new(2, 0, 0)));
        assert!(c.contains_key(&Coord::new(3, 0, 0)));
    }

    #[test]
    fn iter_visits_all_live_cells() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        c.insert(Coord::new(0, 0, 0), dummy_cell(&mut arena, 1));
        c.insert(Coord::new(1, 0, 0), dummy_cell(&mut arena, 2));
        c.insert(Coord::new(2, 0, 0), dummy_cell(&mut arena, 3));
        let count = c.iter().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn iter_mut_skips_dead_slots() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        c.insert(Coord::new(0, 0, 0), dummy_cell(&mut arena, 1));
        c.insert(Coord::new(1, 0, 0), dummy_cell(&mut arena, 2));
        c.insert(Coord::new(2, 0, 0), dummy_cell(&mut arena, 3));
        c.remove(&Coord::new(1, 0, 0));
        let count = c.iter_mut().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn slot_reuse_keeps_storage_dense() {
        let mut arena = Arena::with_capacity(64);
        let mut c = Cells::new();
        // Insert 5 cells, remove 3 of them, insert 3 more.
        for i in 0..5 {
            c.insert(Coord::new(i, 0, 0), dummy_cell(&mut arena, i as u32));
        }
        for i in [1, 2, 3] {
            c.remove(&Coord::new(i, 0, 0));
        }
        for i in 5..8 {
            c.insert(Coord::new(i, 0, 0), dummy_cell(&mut arena, i as u32));
        }
        assert_eq!(c.len(), 5);
        // Storage size shouldn't exceed peak live count (5) because
        // every removal freed a slot for later reuse.
        assert_eq!(c.slots.len(), 5);
        assert!(c.free_slots.is_empty());
    }
}

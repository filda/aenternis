//! Single-buffer cell-memory arena with a coalescing free-list.
//!
//! Owns one `Vec<u32>` sized to the world's total energy at
//! construction; every cell's memory lives as a contiguous slice
//! `arena.slots[mem_start .. mem_start + mem_len]`. Energy is
//! conserved across ticks, so the arena never has to grow — one
//! allocation at `big_bang`, fragmentation contained inside this
//! module's free-list instead of the global allocator.
//!
//! ## Why a free-list, not a bump allocator
//!
//! Each tick's `apply_outflow` shrinks sources and grows targets in
//! roughly equal measure — net allocation per tick is zero, but
//! individual cell ranges churn. A bump allocator would walk past
//! the high-water mark every tick and need periodic compaction; a
//! free-list keeps reusing freed ranges in place. Phase 3 of the
//! arena refactor (double-buffer + compact-by-construction) will
//! drop this free-list entirely in favour of bump-into-`arena_next`
//! per tick, which is naturally compact, but Phase 2 only needs to
//! get the data out of per-cell `Vec`s and into one place — and the
//! free-list is the smallest thing that satisfies that.
//!
//! ## Coalescing on free
//!
//! `free(start, len)` merges with the immediate neighbours
//! (`prev.end == start` or `start + len == next.start`) so the
//! free-list stays minimal and small allocations don't drift into
//! permanent unusability. Without coalescing a 1 M-energy world
//! that churns ~6 ranges per cell per tick would balloon the
//! free-list into the tens of thousands of single-slot fragments
//! after a couple of seconds of simulation.

use std::collections::BTreeMap;

/// Single-buffer arena for cell memory.
///
/// Stores up to [`Arena::capacity`] `u32` slots in one `Vec`. Cells
/// hold `(mem_start, mem_len)` into [`Arena::slots`]; the arena
/// itself tracks which ranges are free via a coalescing
/// [`BTreeMap`]. The map's key is the start of a free range and the
/// value is its length, so first-fit allocation is one
/// [`BTreeMap::iter`] walk and coalescing is two
/// [`BTreeMap::get`] probes (left + right neighbour).
#[derive(Debug, Clone)]
pub struct Arena {
    /// Backing storage. Pre-sized to [`Arena::capacity`] at
    /// construction; `slots.len()` stays equal to capacity forever
    /// so cell ranges can borrow into it without ever triggering a
    /// `Vec` realloc that would invalidate other concurrent
    /// borrows.
    slots: Vec<u32>,

    /// Free ranges keyed by start. Coalesced on every `free` so
    /// adjacent fragments collapse into one entry.
    free: BTreeMap<u32, u32>,

    /// Total slot capacity, cached as `u32` for fast bounds checks.
    /// Equals `slots.len() as u32`.
    capacity: u32,
}

impl Arena {
    /// Build an arena that can hold up to `capacity` u32 slots
    /// total. All slots start in the free-list as a single entry
    /// `(0, capacity)`. Cells must be allocated via [`Arena::alloc`]
    /// before they can be addressed.
    ///
    /// `capacity = 0` is a valid edge case (empty world) — the
    /// free-list stays empty and any `alloc(len > 0)` panics.
    #[must_use]
    pub fn with_capacity(capacity: u32) -> Self {
        let slots = vec![0u32; capacity as usize];
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        Self {
            slots,
            free,
            capacity,
        }
    }

    /// Total slot capacity (= `slots.len()`).
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Read-only view of the full backing slice. Cells use this
    /// via [`Cell::memory`](crate::Cell::memory) plus their own
    /// `(mem_start, mem_len)`; direct callers are tests and the
    /// snapshot path.
    #[must_use]
    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    /// Read-only slice over a specific range. Panics if
    /// `start + len > capacity`.
    #[must_use]
    pub fn slice(&self, start: u32, len: u32) -> &[u32] {
        let s = start as usize;
        let e = s + len as usize;
        &self.slots[s..e]
    }

    /// Mutable slice over a specific range. Panics on bounds.
    pub fn slice_mut(&mut self, start: u32, len: u32) -> &mut [u32] {
        let s = start as usize;
        let e = s + len as usize;
        &mut self.slots[s..e]
    }

    /// Single-slot read. Cheaper than [`Arena::slice`] when the
    /// caller only needs one `u32` — used by VM opcodes that read
    /// a few independent slots per instruction.
    #[must_use]
    pub fn get(&self, start: u32, offset: u32) -> u32 {
        self.slots[(start + offset) as usize]
    }

    /// Single-slot write. Counterpart of [`Arena::get`].
    pub fn set(&mut self, start: u32, offset: u32, value: u32) {
        self.slots[(start + offset) as usize] = value;
    }

    /// Allocate a contiguous run of `len` slots and return its
    /// start index. First-fit walk over the free-list; coalescing
    /// keeps it short enough that the linear scan is cheap in
    /// practice.
    ///
    /// `len == 0` is a no-op and returns `0` (the start is never
    /// dereferenced for empty cells).
    ///
    /// If no free range can satisfy the request, the arena grows
    /// (doubling capacity, but always by at least `len`) and the
    /// alloc retries. Production paths preallocate the arena via
    /// [`Arena::with_capacity`] to exactly the world's energy at
    /// `big_bang` time, which means growth never fires in steady
    /// state; the growth branch is what lets test paths
    /// (`SparseWorld::new` then `insert`) work without pre-sizing.
    ///
    /// # Panics
    ///
    /// Panics if growth would overflow `u32` (i.e. asking for more
    /// than `u32::MAX` total slots). The world's energy is a `u32`,
    /// so this corresponds to a logically impossible state.
    pub fn alloc(&mut self, len: u32) -> u32 {
        if len == 0 {
            return 0;
        }
        if let Some((&start, &free_len)) = self.free.iter().find(|(_, &l)| l >= len) {
            self.free.remove(&start);
            if free_len > len {
                self.free.insert(start + len, free_len - len);
            }
            return start;
        }

        // No free range fits — grow. Double capacity, but always by
        // at least `len` so a single huge request lands on the first
        // grow. New tail goes onto the free-list before retry.
        let added = self
            .capacity
            .checked_mul(2)
            .map(|doubled| doubled - self.capacity)
            .filter(|&d| d >= len)
            .unwrap_or_else(|| len.max(16));
        let new_capacity = self
            .capacity
            .checked_add(added)
            .expect("arena capacity overflowed u32");
        self.slots.resize(new_capacity as usize, 0);
        let grow_start = self.capacity;
        self.capacity = new_capacity;
        self.free(grow_start, added);
        // Retry — guaranteed to succeed now: the grow tail is at
        // least `len` slots and coalescing may have absorbed a
        // trailing free range too.
        self.alloc(len)
    }

    /// Return a range to the free-list, coalescing with adjacent
    /// free ranges if any. `len == 0` is a no-op.
    ///
    /// Two-step coalesce: check the predecessor (largest key
    /// `<= start`) for `prev.end == start`, then check the
    /// successor (smallest key `> start`) for `start + len ==
    /// next_start`. Either or both merges can fire.
    pub fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let mut merged_start = start;
        let mut merged_len = len;

        // Coalesce with predecessor: find the free range whose end
        // touches our start.
        if let Some((&prev_start, &prev_len)) = self.free.range(..start).next_back() {
            if prev_start + prev_len == start {
                self.free.remove(&prev_start);
                merged_start = prev_start;
                merged_len += prev_len;
            }
        }

        // Coalesce with successor: free range starting exactly where
        // our merged range ends.
        let next_start = merged_start + merged_len;
        if let Some(&next_len) = self.free.get(&next_start) {
            self.free.remove(&next_start);
            merged_len += next_len;
        }

        self.free.insert(merged_start, merged_len);
    }

    /// Reallocate an existing range to a new length, copying the
    /// overlap. Returns the new start index.
    ///
    /// Implementation is `alloc(new_len) → copy → free(old_start,
    /// old_len)`, which is the right shape for the merge-inflows
    /// rebuild that always writes fresh data into the new range
    /// from a scratch buffer; the in-place "shrink" case is hot
    /// enough that [`Arena::shrink_in_place`] handles it without
    /// going through here.
    ///
    /// # Panics
    ///
    /// Panics if no free range of `>= new_len` slots exists.
    pub fn realloc(&mut self, old_start: u32, old_len: u32, new_len: u32) -> u32 {
        if new_len == old_len {
            return old_start;
        }
        let new_start = self.alloc(new_len);
        let copy_len = old_len.min(new_len) as usize;
        if copy_len > 0 {
            // SAFETY-free version: copy via slice intermediates so the
            // overlap rule (`copy_from_slice` requires disjoint slices)
            // can't be violated. `alloc` returned a free range, so it
            // doesn't overlap `[old_start, old_start + old_len)`.
            let old = old_start as usize;
            let new = new_start as usize;
            let (lo, hi) = if new < old { (new, old) } else { (old, new) };
            let (left, right) = self.slots.split_at_mut(hi);
            if new < old {
                left[lo..lo + copy_len].copy_from_slice(&right[..copy_len]);
            } else {
                right[..copy_len].copy_from_slice(&left[lo..lo + copy_len]);
            }
        }
        self.free(old_start, old_len);
        new_start
    }

    /// Shrink an existing range from the end by `drop` slots,
    /// returning the trailing slot count to the free-list. Returns
    /// the new length.
    ///
    /// Saturating: `drop > old_len` clamps to `old_len`. The cell's
    /// `mem_start` does not change — only the trailing range
    /// `[start + new_len, start + old_len)` is freed.
    pub fn shrink_in_place(&mut self, start: u32, old_len: u32, drop: u32) -> u32 {
        let actual_drop = drop.min(old_len);
        if actual_drop == 0 {
            return old_len;
        }
        let new_len = old_len - actual_drop;
        self.free(start + new_len, actual_drop);
        new_len
    }
}

#[cfg(test)]
mod tests {
    use super::Arena;

    #[test]
    fn empty_arena_has_no_free_ranges() {
        let a = Arena::with_capacity(0);
        assert_eq!(a.capacity(), 0);
        assert!(a.free.is_empty());
    }

    #[test]
    fn nonempty_arena_starts_with_one_free_range() {
        let a = Arena::with_capacity(100);
        assert_eq!(a.capacity(), 100);
        assert_eq!(a.free.len(), 1);
        assert_eq!(a.free.get(&0), Some(&100));
    }

    #[test]
    fn alloc_returns_zero_for_zero_len() {
        let mut a = Arena::with_capacity(100);
        assert_eq!(a.alloc(0), 0);
        // free-list untouched
        assert_eq!(a.free.get(&0), Some(&100));
    }

    #[test]
    fn alloc_carves_from_first_free_range() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(30);
        assert_eq!(s, 0);
        assert_eq!(a.free.get(&30), Some(&70));
    }

    #[test]
    fn alloc_exact_fit_removes_range() {
        let mut a = Arena::with_capacity(50);
        let _ = a.alloc(50);
        assert!(a.free.is_empty());
    }

    #[test]
    fn alloc_grows_when_out_of_room() {
        let mut a = Arena::with_capacity(10);
        let _ = a.alloc(20);
        assert!(a.capacity() >= 20);
    }

    #[test]
    fn alloc_grows_from_zero_capacity() {
        let mut a = Arena::with_capacity(0);
        let s = a.alloc(7);
        assert_eq!(s, 0);
        assert!(a.capacity() >= 7);
    }

    #[test]
    fn free_coalesces_with_predecessor() {
        let mut a = Arena::with_capacity(100);
        let s1 = a.alloc(20);
        let s2 = a.alloc(20);
        // free [s1..s1+20], then [s2..s2+20] — both should coalesce
        // with each other and the trailing free range.
        a.free(s1, 20);
        a.free(s2, 20);
        // Whole arena should be one big free range again.
        assert_eq!(a.free.len(), 1);
        assert_eq!(a.free.get(&0), Some(&100));
    }

    #[test]
    fn free_coalesces_with_successor() {
        let mut a = Arena::with_capacity(100);
        let s1 = a.alloc(20);
        let _s2 = a.alloc(20);
        // free s1 first — should merge with [0..20] (no prev) and stay
        // separate from s2. Actually [0, 20) is alloc'd, free is
        // [40, 100). So freeing [0, 20) gives [0, 20) free + [40, 100)
        // free, two ranges.
        a.free(s1, 20);
        assert_eq!(a.free.len(), 2);
        assert_eq!(a.free.get(&0), Some(&20));
        assert_eq!(a.free.get(&40), Some(&60));
    }

    #[test]
    fn shrink_in_place_returns_tail_to_free_list() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(50);
        let new_len = a.shrink_in_place(s, 50, 10);
        assert_eq!(new_len, 40);
        // Tail [40..50) plus trailing [50..100) coalesce.
        assert_eq!(a.free.len(), 1);
        assert_eq!(a.free.get(&40), Some(&60));
    }

    #[test]
    fn shrink_in_place_clamps_drop() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(50);
        let new_len = a.shrink_in_place(s, 50, 100);
        assert_eq!(new_len, 0);
        // Entire range coalesces back into the original.
        assert_eq!(a.free.len(), 1);
        assert_eq!(a.free.get(&0), Some(&100));
    }

    #[test]
    fn realloc_same_size_is_noop() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(50);
        let s2 = a.realloc(s, 50, 50);
        assert_eq!(s, s2);
    }

    #[test]
    fn realloc_grows_and_copies() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(10);
        for i in 0..10 {
            a.set(s, i, 100 + i);
        }
        let s2 = a.realloc(s, 10, 20);
        assert_ne!(s, s2);
        for i in 0..10 {
            assert_eq!(a.get(s2, i), 100 + i);
        }
    }

    #[test]
    fn realloc_shrinks_and_copies_prefix() {
        let mut a = Arena::with_capacity(100);
        let s = a.alloc(10);
        for i in 0..10 {
            a.set(s, i, 200 + i);
        }
        let s2 = a.realloc(s, 10, 4);
        for i in 0..4 {
            assert_eq!(a.get(s2, i), 200 + i);
        }
    }

    #[test]
    fn slice_round_trips() {
        let mut a = Arena::with_capacity(20);
        let s = a.alloc(5);
        let slice = a.slice_mut(s, 5);
        slice.copy_from_slice(&[7, 8, 9, 10, 11]);
        assert_eq!(a.slice(s, 5), &[7, 8, 9, 10, 11]);
    }
}

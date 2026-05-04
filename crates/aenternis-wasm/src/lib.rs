//! WebAssembly bindings for Aenternis.
//!
//! Wraps [`aenternis_core::SparseWorld`] in a `#[wasm_bindgen]` handle
//! that exposes the simulation API to JavaScript. Build with:
//!
//! ```text
//! wasm-pack build crates/aenternis-wasm --target web
//! ```
//!
//! The result lands in `crates/aenternis-wasm/pkg/` as a small bundle
//! of `.wasm` plus generated JS glue, suitable for `import` from the
//! Vite frontend.
//!
//! ## API style
//!
//! The wrapper exposes an **owned handle** — JavaScript constructs a
//! `World` instance, calls methods on it, and lets `wasm-bindgen`'s
//! generated `free()` reclaim the WASM-side memory when the handle
//! goes out of scope. Multiple worlds can coexist; there is no global
//! singleton.
//!
//! ## Numeric widths
//!
//! `seed` and `tick` are exposed as `u32` rather than the core's `u64`
//! because `wasm-bindgen` lowers `u64` to a JS `BigInt`, which is
//! awkward for everyday use. The wrapper widens internally. The
//! single-`u32` seed gives `2^32` distinct simulations — plenty for a
//! prototype, and easy to upgrade to `u64`/`BigInt` later if needed.

use aenternis_core::{tick, SparseWorld};
use wasm_bindgen::prelude::*;

/// Aenternis simulation world handle.
///
/// Constructed with [`World::new`], stepped with [`World::step`],
/// queried with [`World::total_energy`] / [`World::cell_count`] /
/// [`World::tick`]. Free with the auto-generated `free()` method on
/// the JS side.
#[wasm_bindgen]
pub struct World {
    inner: SparseWorld,
}

#[wasm_bindgen]
impl World {
    /// Construct a new world initialized as a big bang at the origin.
    ///
    /// `seed` and `energy` are deterministic — same pair yields the
    /// same initial state on every run, on every host platform.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(seed: u32, energy: u32) -> Self {
        Self {
            inner: SparseWorld::big_bang(u64::from(seed), energy),
        }
    }

    /// Construct a new world with a programmer-supplied prefix written
    /// into the origin cell's memory. The first `min(program.length,
    /// energy)` slots are taken verbatim from `program`, the rest from
    /// the deterministic per-cell RNG.
    ///
    /// Matches prototype 9's `bigBang(eTotal, programSlots)` semantics:
    /// program-covered slots do not advance the RNG, so the suffix
    /// generated from `(seed, energy)` is identical regardless of the
    /// program supplied.
    #[wasm_bindgen(js_name = newWithProgram)]
    #[must_use]
    pub fn new_with_program(seed: u32, energy: u32, program: &[u32]) -> Self {
        Self {
            inner: SparseWorld::big_bang_with_program(u64::from(seed), energy, program),
        }
    }

    /// Run one simulation tick.
    ///
    /// `coeff` is the diffusion coefficient (typical range 0.15-0.30);
    /// `k` is the CPU compute constant (typical value 1, where
    /// `instructions_per_cell = floor(energy / k)`).
    pub fn step(&mut self, coeff: f32, k: u32) {
        tick::step(&mut self.inner, coeff, k);
    }

    /// Total energy summed across every cell. Conserved across ticks
    /// (cardinal physical invariant).
    ///
    /// Saturates at `u32::MAX` if the world's `E_total` ever exceeds
    /// that value — but in any realistic simulation the cap is far
    /// below.
    #[must_use]
    #[wasm_bindgen(js_name = totalEnergy)]
    pub fn total_energy(&self) -> u32 {
        u32::try_from(self.inner.total_energy()).unwrap_or(u32::MAX)
    }

    /// Number of cells currently allocated in the sparse world.
    /// Bounded above by [`World::total_energy`].
    #[must_use]
    #[wasm_bindgen(js_name = cellCount)]
    pub fn cell_count(&self) -> u32 {
        u32::try_from(self.inner.cells.len()).unwrap_or(u32::MAX)
    }

    /// Current tick count. Starts at zero, increments by one per
    /// [`World::step`] call.
    #[must_use]
    pub fn tick(&self) -> u32 {
        u32::try_from(self.inner.tick).unwrap_or(u32::MAX)
    }

    /// Snapshot of every cell, packed into a flat array. Returns
    /// `cell_count * STRIDE` `u32` values where `STRIDE = 6` and the
    /// per-cell layout is:
    ///
    /// | offset | meaning      | type                  |
    /// |--------|--------------|-----------------------|
    /// | `+0`   | `x`          | `i32` (as `u32` bits) |
    /// | `+1`   | `y`          | `i32` (as `u32` bits) |
    /// | `+2`   | `z`          | `i32` (as `u32` bits) |
    /// | `+3`   | `energy`     | `u32`                 |
    /// | `+4`   | `origin_tag` | `u32`                 |
    /// | `+5`   | `appearance` | `u32`                 |
    ///
    /// Coordinates are `i32` reinterpreted as `u32` — JS decodes them
    /// via a signed `Int32Array` view over the same buffer if it cares
    /// about the sign bit; for rendering, a plain `Uint32Array` view
    /// gives the right values via two's-complement.
    ///
    /// Iteration order is deterministic — the underlying `BTreeMap`
    /// walks coordinates in `(x, y, z)` lexicographic order. That
    /// matches the iteration order seen by every other read API.
    ///
    /// Returns a freshly-allocated `Vec` each call. For multi-million
    /// cell worlds this is wasteful; a persistent buffer + length API
    /// is on the roadmap for sub-phase 3c when it becomes a measurable
    /// bottleneck.
    #[must_use]
    #[wasm_bindgen(js_name = cellsSnapshot)]
    pub fn cells_snapshot(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.inner.cells.len() * Self::SNAPSHOT_STRIDE);
        for (coord, cell) in &self.inner {
            out.push(coord.x as u32);
            out.push(coord.y as u32);
            out.push(coord.z as u32);
            out.push(cell.energy());
            out.push(cell.origin_tag);
            out.push(cell.appearance);
        }
        out
    }

    /// Number of `u32` fields per cell in [`World::cells_snapshot`].
    /// Provided as a JS-visible getter so callers can unpack the
    /// snapshot without hard-coding `6`.
    ///
    /// `wasm_bindgen` rejects `const fn` exports (it generates
    /// runtime trampolines), so `clippy::missing_const_for_fn` is
    /// silenced locally.
    #[must_use]
    #[wasm_bindgen(getter, js_name = snapshotStride)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn snapshot_stride(&self) -> u32 {
        Self::SNAPSHOT_STRIDE as u32
    }

    /// Full state dump of the cell at `(x, y, z)`. Returns an empty
    /// `Uint32Array` if no cell exists at that coordinate.
    ///
    /// Layout (all `u32`, fixed-width prefix then variable memory):
    ///
    /// | offset       | meaning           | length |
    /// |--------------|-------------------|--------|
    /// | `0`          | `pc`              | 1      |
    /// | `1`          | `energy`          | 1      |
    /// | `2`          | `origin_tag`      | 1      |
    /// | `3`          | `appearance`      | 1      |
    /// | `4..10`      | `pointers[6]`     | 6      |
    /// | `10..16`     | `rates[6]`        | 6      |
    /// | `16..22`     | `active_outflow[6]` | 6    |
    /// | `22..28`     | `inflow[6]`       | 6      |
    /// | `28..28+E`   | `memory[E]`       | E      |
    ///
    /// Total length = `INSPECT_PREFIX + energy`, so JS can derive
    /// `memSize = arr.length - INSPECT_PREFIX` without a separate
    /// metadata call. Use [`World::inspect_prefix`] to get the
    /// constant from JS.
    ///
    /// Direction order in the six-element arrays matches the
    /// canonical `[xp, xn, yp, yn, zp, zn]` used everywhere else.
    #[must_use]
    #[wasm_bindgen(js_name = cellInspect)]
    pub fn cell_inspect(&self, x: i32, y: i32, z: i32) -> Vec<u32> {
        let coord = aenternis_core::Coord::new(x, y, z);
        let Some(cell) = self.inner.cells.get(&coord) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(Self::INSPECT_PREFIX + cell.memory.len());
        out.push(cell.pc);
        out.push(cell.energy());
        out.push(cell.origin_tag);
        out.push(cell.appearance);
        out.extend_from_slice(&cell.pointers);
        out.extend_from_slice(&cell.rates);
        out.extend_from_slice(&cell.active_outflow);
        out.extend_from_slice(&cell.inflow);
        out.extend_from_slice(&cell.memory);
        out
    }

    /// Number of `u32` fields in the fixed-width prefix of
    /// [`World::cell_inspect`] before the memory slots start (= 28).
    #[must_use]
    #[wasm_bindgen(getter, js_name = inspectPrefix)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn inspect_prefix(&self) -> u32 {
        Self::INSPECT_PREFIX as u32
    }
}

impl World {
    /// Number of `u32` fields per cell in [`World::cells_snapshot`].
    /// Available as a constant for Rust-side callers; JS uses the
    /// `snapshotStride` getter.
    pub const SNAPSHOT_STRIDE: usize = 6;

    /// Number of `u32` fields in the fixed-width prefix of
    /// [`World::cell_inspect`] before the variable-length memory
    /// region starts (4 scalars + 4 × 6 directional arrays = 28).
    pub const INSPECT_PREFIX: usize = 28;
}

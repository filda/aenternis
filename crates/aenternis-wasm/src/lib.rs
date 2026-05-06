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

use aenternis_core::{tick, RngKind, SparseWorld};
use wasm_bindgen::prelude::*;

/// Map a `u8` flag from the JS bridge to a Rust [`RngKind`]. The two
/// JS-visible values (`0` = PCG, `1` = xorshift32) match the order in
/// which the toggles appear in the Aenternis web UI.
const fn rng_kind_from_u8(value: u8) -> RngKind {
    match value {
        1 => RngKind::Xorshift32,
        _ => RngKind::Pcg,
    }
}

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
    /// Uses the default PCG backend; pass [`World::new_with_kind`] to
    /// pick a backend explicitly.
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

    /// Construct a new world with both a program and an explicit RNG
    /// backend choice. `rng_kind` is `0` for PCG (Aenternis default) or
    /// `1` for xorshift32 (matches JS prototype 9-B bit-for-bit).
    ///
    /// Use this when you need to compare against the JS prototype — the
    /// xorshift32 path reproduces the exact same per-cell-tick stream and
    /// origin-tag derivation that 9-B's `world.js` produces.
    #[wasm_bindgen(js_name = newWithProgramAndKind)]
    #[must_use]
    pub fn new_with_program_and_kind(
        seed: u32,
        energy: u32,
        program: &[u32],
        rng_kind: u8,
    ) -> Self {
        Self {
            inner: SparseWorld::big_bang_with_program_and_kind(
                u64::from(seed),
                energy,
                program,
                rng_kind_from_u8(rng_kind),
            ),
        }
    }

    /// Run one simulation tick.
    ///
    /// `coeff` is the diffusion coefficient (typical range 0.15-0.30);
    /// `k` is the CPU compute constant (typical value 1, where
    /// `instructions_per_cell = floor(energy / k)`). `coeff` is passed
    /// as `f64` so that JS `Number(0.15)` reaches the rate computation
    /// without a lossy `f32` round-trip — important for bit-identity
    /// against JS prototype 9-B in `legacy_full_precision` mode.
    pub fn step(&mut self, coeff: f64, k: u32) {
        tick::step(&mut self.inner, coeff, k);
    }

    /// Set the dominance / intrusion `move_threshold`.
    ///
    /// Higher = more aggressive metempsychosis (target cells get
    /// overwritten by stronger neighbors faster). Default is `2.0`,
    /// matches the `mechanics.md` spec; prototype 9 used `1.0` for a
    /// less-aggressive, more wispy regime.
    #[wasm_bindgen(js_name = setMoveThreshold)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_move_threshold(&mut self, threshold: f32) {
        self.inner.move_threshold = threshold;
    }

    /// Current `move_threshold` value.
    ///
    /// `wasm_bindgen` rejects `const fn` exports, so
    /// `clippy::missing_const_for_fn` is silenced locally.
    #[wasm_bindgen(getter, js_name = moveThreshold)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn move_threshold(&self) -> f32 {
        self.inner.move_threshold
    }

    /// Toggle the JS-prototype-9-B layout-tick-offset quirk. When
    /// enabled, `compute_natural_rates` keys its per-cell-tick RNG
    /// with `tick - 1` so xorshift32 + legacy reproduces 9-B's
    /// per-tick stream bit-for-bit. Toggling mid-run is safe; the
    /// change applies on the next `step` call.
    #[wasm_bindgen(js_name = setLegacyTickOffset)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_legacy_tick_offset(&mut self, enabled: bool) {
        self.inner.legacy_tick_offset = enabled;
    }

    /// Current `legacy_tick_offset` value. See
    /// [`World::set_legacy_tick_offset`] for what it controls.
    #[wasm_bindgen(getter, js_name = legacyTickOffset)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn legacy_tick_offset(&self) -> bool {
        self.inner.legacy_tick_offset
    }

    /// Toggle the JS-prototype-9-B `f64`-arithmetic precision mode.
    /// When enabled, `compute_natural_rates` runs the stochastic-floor
    /// comparison in `f64` with all 32 bits of RNG entropy, matching
    /// 9-B exactly. Toggling mid-run is safe; the change applies on
    /// the next `step` call.
    #[wasm_bindgen(js_name = setLegacyFullPrecision)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_legacy_full_precision(&mut self, enabled: bool) {
        self.inner.legacy_full_precision = enabled;
    }

    /// Current `legacy_full_precision` value. See
    /// [`World::set_legacy_full_precision`] for what it controls.
    #[wasm_bindgen(getter, js_name = legacyFullPrecision)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn legacy_full_precision(&self) -> bool {
        self.inner.legacy_full_precision
    }

    /// Toggle JS-prototype-9-B's wrapping `port` accumulation. When
    /// enabled, the `port` opcode's contribution to `active_outflow`
    /// uses `wrapping_add` (matches `(activeOutflow + arg1) >>> 0`)
    /// instead of `saturating_add`. This is what makes 9-B's
    /// asymmetric expansion appear when noise memory triggers many
    /// `port` ops in a tick — without it, every targeted direction
    /// saturates and the proportional clamp distributes outflow
    /// evenly. Toggling mid-run is safe.
    #[wasm_bindgen(js_name = setLegacyPortWrap)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_legacy_port_wrap(&mut self, enabled: bool) {
        self.inner.legacy_port_wrap = enabled;
    }

    /// Current `legacy_port_wrap` value. See
    /// [`World::set_legacy_port_wrap`] for what it controls.
    #[wasm_bindgen(getter, js_name = legacyPortWrap)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn legacy_port_wrap(&self) -> bool {
        self.inner.legacy_port_wrap
    }

    /// Toggle the JS-prototype-9-B opcode-set restriction. When
    /// enabled, the VM treats opcodes `0x14..=0x16` (`sinflow`,
    /// `sself`, `srate`) as unknown — same as any byte `> 0x16`. JS
    /// prototype 9-B stops at `0x13` (`paint`), so noise memory that
    /// happens to encode `0x14`/`0x15`/`0x16` produces a single-slot
    /// nop in JS but a 3-slot opcode in default Rust. Toggling
    /// mid-run is safe.
    #[wasm_bindgen(js_name = setLegacyOpcodeSet)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_legacy_opcode_set(&mut self, enabled: bool) {
        self.inner.legacy_opcode_set = enabled;
    }

    /// Current `legacy_opcode_set` value. See
    /// [`World::set_legacy_opcode_set`] for what it controls.
    #[wasm_bindgen(getter, js_name = legacyOpcodeSet)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn legacy_opcode_set(&self) -> bool {
        self.inner.legacy_opcode_set
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

    /// Bounding box across all live cells, returned as a flat 6-element
    /// `Int32Array`: `[x_min, x_max, y_min, y_max, z_min, z_max]`. Empty
    /// `Int32Array` if the world has no cells.
    ///
    /// Compute is `O(n)` over cells — fine at the prototype's typical
    /// million-cell ceiling, but call once per snapshot rather than once
    /// per render frame.
    #[must_use]
    #[wasm_bindgen(js_name = boundingBox)]
    pub fn bounding_box(&self) -> Vec<i32> {
        match self.inner.bounding_box() {
            Some((x_min, x_max, y_min, y_max, z_min, z_max)) => {
                vec![x_min, x_max, y_min, y_max, z_min, z_max]
            }
            None => Vec::new(),
        }
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
    /// Iteration order is `(x, y, z)` lexicographic — the snapshot
    /// boundary sorts the underlying `FxHashMap` (which iterates in
    /// non-lex hash order) so JS callers see the canonical layout.
    ///
    /// Returns a freshly-allocated `Vec` each call. For multi-million
    /// cell worlds this is wasteful; a persistent buffer + length API
    /// is on the roadmap for sub-phase 3c when it becomes a measurable
    /// bottleneck.
    #[must_use]
    #[wasm_bindgen(js_name = cellsSnapshot)]
    pub fn cells_snapshot(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.inner.cells.len() * Self::SNAPSHOT_STRIDE);
        // sorted_iter walks cells in `(x, y, z)` lex order — the snapshot's
        // documented contract. The world's internal FxHashMap iterates in
        // hash order, which is deterministic but not lex.
        for (coord, cell) in self.inner.sorted_iter() {
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

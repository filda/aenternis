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

// `js_sys::Uint32Array::view` backs the zero-copy snapshot path
// (`cells_snapshot_view`, `cell_inspect_view`) and is `unsafe` because
// the returned view aliases WASM linear memory: a subsequent call into
// WASM that grows memory or reallocates the underlying `Vec` would
// invalidate it. The unsafety is contained to two callsites in this
// file, each with a SAFETY comment naming the JS-side contract (copy
// before any further WASM call). Workspace-wide `unsafe_code = "deny"`
// is overridden here, not at the workspace level, so every other crate
// stays unsafe-free. See `docs/optimalizace-2026-05.md`.
#![allow(unsafe_code)]
// `std::alloc::set_alloc_error_hook` is on nightly behind the
// `alloc_error_hook` feature gate. The threaded WASM build already
// pins nightly (`scripts/build-wasm.sh`), so enabling it here is
// free for that target. Gated on both `target_arch = "wasm32"` AND
// the `wasm-threads` feature so the default single-threaded build
// (CI's `wasm-pack build --target web --release` with stable
// toolchain) doesn't see `#![feature(...)]` — stable rejects feature
// gates with E0554. Host build (rlib via `cargo test --workspace`)
// also stays unaffected since both gates are false there.
#![cfg_attr(
    all(target_arch = "wasm32", feature = "wasm-threads"),
    feature(alloc_error_hook)
)]

use aenternis_core::{tick, SparseWorld};
#[cfg(target_arch = "wasm32")]
use js_sys::Uint32Array;
use wasm_bindgen::prelude::*;

// Replace the default `dlmalloc` allocator on the threaded WASM
// build. The default `dlmalloc` shipped with `wasm32-unknown-unknown`
// serializes every alloc/free through a single mutex when `+atomics`
// is enabled and coalesces freed chunks lazily; combined with the N
// rayon-worker churn over per-tick `Vec`s in `collect_outflow_into`
// / `apply_outflow` / `MERGE_SCRATCH`, that fragments the heap fast
// enough to fail a ~5 MB contiguous request after a couple thousand
// ticks at E = 1 M, even though total live memory stays in the low
// hundreds of MB and `--max-memory=4294967296` leaves 4 GiB of
// headroom.
//
// `talc::WasmHandler` grows the linear memory via `memory.grow` on
// demand and is what talc's built-in `TalckWasm` is built on. The
// stock `TalckWasm` wraps the handler in `AssumeUnlockable` and is
// only sound on single-threaded WASM — assuming-unlockable while
// rayon workers race the allocator would be UB. We pair the same
// handler with a real `spinning_top::RawSpinlock` instead. That
// spinlock implements `lock_api::RawMutex`, so it slots straight
// into talc's generic `Talck<R, O>` wrapper. Coalescing is much
// more aggressive than dlmalloc's, which is what closes the
// fragmentation gap.
//
// Single-threaded WASM keeps dlmalloc — it has no contention or
// fragmentation pressure there, and pulling talc in unconditionally
// would inflate the stable-toolchain CI parity bundle for no win.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[global_allocator]
static TALC: talc::Talck<spinning_top::RawSpinlock, talc::WasmHandler> =
    unsafe { talc::Talc::new(talc::WasmHandler::new()).lock() };

// When the `wasm-threads` feature is on (and we're building for
// wasm32), re-export `init_thread_pool` from `wasm-bindgen-rayon` so
// JS can call it via the generated bindings. JS must `await` this once
// after `await init()` and before any `step()` call, e.g.
//
//     await init();
//     await initThreadPool(navigator.hardwareConcurrency);
//
// The pool stays alive for the lifetime of the page. On targets /
// configurations without the feature this re-export is absent and JS
// must not call it (and doesn't need to — the bundle is single-
// threaded). See `scripts/build-wasm.sh` for the threaded build, and
// `docs/optimalizace-2026-05.md` for the JS-side setup.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use wasm_bindgen_rayon::init_thread_pool;

/// `console.error` extern used by [`aenternis_alloc_error_hook`].
/// Calling through a hand-rolled wasm-bindgen extern instead of
/// pulling in `web_sys` keeps the crate dep graph small (web_sys is
/// ~3 MB of generated bindings for a one-line diagnostic), and the
/// generated lowering hands the `&str` to JS as a ptr+len pair
/// without copying into a fresh WASM-side allocation — which matters
/// because the hook fires *because* WASM allocation just failed.
///
/// Gated on `wasm-threads` for the same reason as the hook itself:
/// `set_alloc_error_hook` is nightly-only, so the alloc-diagnostic
/// path only exists in the threaded (nightly) bundle. The default
/// single-threaded build keeps the legacy silent-`unreachable`
/// behaviour on OOM; that bundle is a stable-toolchain fallback,
/// not the production deploy path.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error_alloc_fail(msg: &str, size: u32, align: u32);
}

/// `std::alloc` error hook installed at module start. Default
/// behaviour on `wasm32-unknown-unknown` is to abort via the
/// `unreachable` instruction with no diagnostic, so JS sees only
/// `RuntimeError: unreachable` and can't tell an OOM apart from a
/// real bug. This hook logs the failing layout (size, align) to
/// `console.error` first; the default abort still runs after the
/// hook returns, but DevTools now shows *why*.
///
/// Deliberately allocation-free: a `panic!` here would format a
/// `String` and re-enter the allocator that just failed, double-
/// faulting before the message reaches the console.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
fn aenternis_alloc_error_hook(layout: std::alloc::Layout) {
    let size = u32::try_from(layout.size()).unwrap_or(u32::MAX);
    let align = u32::try_from(layout.align()).unwrap_or(u32::MAX);
    console_error_alloc_fail(
        "Aenternis WASM allocation failure (out of memory). Following: requested size, align.",
        size,
        align,
    );
}

/// Runs once when the WASM module is instantiated. Installs the
/// diagnostic hooks so opaque `RuntimeError: unreachable` traps
/// always come with context in DevTools:
///
/// 1. `console_error_panic_hook` for ordinary Rust panics (formats
///    the panic payload + Rust source location into the console).
///    Active on every wasm32 build.
/// 2. `aenternis_alloc_error_hook` for `std::alloc` failures (logs
///    the failing layout). The default `__rust_alloc_error_handler`
///    on wasm32 aborts silently, which is indistinguishable from
///    a real bug at the JS level. Active only on the threaded
///    bundle, since `set_alloc_error_hook` is nightly-only.
///
/// Both `set_once`-style: idempotent and thread-local-aware, so they
/// cover every worker thread that `wasm-bindgen-rayon` spawns (each
/// one re-instantiates the module and re-runs the start function on
/// its own context).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn __aenternis_wasm_start() {
    console_error_panic_hook::set_once();
    #[cfg(feature = "wasm-threads")]
    std::alloc::set_alloc_error_hook(aenternis_alloc_error_hook);
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
    /// Persistent scratch buffer for [`World::cells_snapshot`] and
    /// [`World::cells_snapshot_view`]. Reused across ticks so the
    /// steady-state cost is a `clear` + fill rather than a fresh
    /// allocation per snapshot; capacity grows monotonically to peak
    /// cell count. The view variant returns a `Uint32Array` that
    /// aliases this buffer's WASM-memory storage directly.
    snapshot_buf: Vec<u32>,
    /// Persistent scratch buffer for [`World::cell_inspect`] and
    /// [`World::cell_inspect_view`]. Smaller than `snapshot_buf` (one
    /// cell's worth of memory, not the whole world), but the same
    /// reuse rationale: avoid a per-call allocation, and own the
    /// backing storage for the zero-copy view path.
    inspect_buf: Vec<u32>,
}

#[wasm_bindgen]
impl World {
    /// Construct a new world initialized as a big bang at the origin.
    ///
    /// `seed` and `energy` are deterministic — same pair yields the
    /// same initial state on every run, on every host platform. The
    /// RNG path is `xorshift32` keyed via `cell_seed`; see
    /// `aenternis_core::rng` for the frozen-reference-stream contract.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(seed: u32, energy: u32) -> Self {
        Self {
            inner: SparseWorld::big_bang(u64::from(seed), energy),
            snapshot_buf: Vec::new(),
            inspect_buf: Vec::new(),
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
            snapshot_buf: Vec::new(),
            inspect_buf: Vec::new(),
        }
    }

    /// Run one simulation tick.
    ///
    /// `coeff` is the diffusion coefficient (typical range 0.15-0.30);
    /// `k` is the CPU compute constant (typical value 1, where
    /// `instructions_per_cell = floor(energy / k)`). `coeff` is passed
    /// as `f64` so that JS `Number(0.15)` reaches the rate computation
    /// without a lossy `f32` round-trip.
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
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
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
    /// Internally fills the persistent `snapshot_buf` and clones it
    /// out — `wasm_bindgen` lowers `Vec<u32>` to a JS-side copy
    /// regardless, so the clone is unavoidable on the boundary, but
    /// the working buffer's capacity is retained across calls and
    /// the steady-state per-tick allocation cost vanishes after the
    /// first peak-sized world.
    ///
    /// For the zero-copy variant that skips the Rust-side clone, see
    /// [`World::cells_snapshot_view`].
    #[must_use]
    #[wasm_bindgen(js_name = cellsSnapshot)]
    pub fn cells_snapshot(&mut self) -> Vec<u32> {
        self.fill_snapshot_buf();
        self.snapshot_buf.clone()
    }

    /// Zero-copy variant of [`World::cells_snapshot`]: fills the same
    /// persistent buffer, then returns a `Uint32Array` that aliases it
    /// directly in WASM linear memory. JS must copy the data out
    /// (e.g. `new Uint32Array(world.cellsSnapshotView())`) **before**
    /// the next call into WASM — a subsequent `step()` or another
    /// snapshot call may reallocate the underlying buffer or grow the
    /// linear memory, which invalidates the view. Likewise, the view's
    /// `.buffer` is the WASM memory itself; never `postMessage`-transfer
    /// it directly — transferring it would detach WASM memory.
    ///
    /// Saves one ~24 MB memcpy per snapshot on a million-cell world by
    /// avoiding the Rust-side `Vec` clone that the safe variant needs
    /// to satisfy `wasm-bindgen`'s `Vec<u32>` ABI.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    #[wasm_bindgen(js_name = cellsSnapshotView)]
    pub fn cells_snapshot_view(&mut self) -> Uint32Array {
        self.fill_snapshot_buf();
        // SAFETY: The returned view aliases `self.snapshot_buf`'s
        // storage in WASM linear memory. JS callers must copy out (via
        // `new Uint32Array(view)` or `.slice()`) before any further
        // call into WASM, and must not `postMessage`-transfer the
        // underlying buffer. Both invariants are documented above and
        // in `docs/optimalizace-2026-05.md`.
        unsafe { Uint32Array::view(&self.snapshot_buf[..]) }
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
    ///
    /// Internally fills the persistent `inspect_buf` and clones it
    /// out, mirroring [`World::cells_snapshot`]. For the zero-copy
    /// variant, see [`World::cell_inspect_view`].
    #[must_use]
    #[wasm_bindgen(js_name = cellInspect)]
    pub fn cell_inspect(&mut self, x: i32, y: i32, z: i32) -> Vec<u32> {
        self.fill_inspect_buf(x, y, z);
        self.inspect_buf.clone()
    }

    /// Zero-copy variant of [`World::cell_inspect`]: fills the same
    /// persistent buffer, then returns a `Uint32Array` aliasing it in
    /// WASM linear memory. Same JS-side contract as
    /// [`World::cells_snapshot_view`] — copy before any further WASM
    /// call, never transfer the buffer.
    ///
    /// Returns an empty `Uint32Array` if no cell exists at `(x, y, z)`.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    #[wasm_bindgen(js_name = cellInspectView)]
    pub fn cell_inspect_view(&mut self, x: i32, y: i32, z: i32) -> Uint32Array {
        self.fill_inspect_buf(x, y, z);
        // SAFETY: See `cells_snapshot_view`. Same invariants — view
        // aliases WASM linear memory backing `self.inspect_buf`; JS
        // must copy out before the next WASM call.
        unsafe { Uint32Array::view(&self.inspect_buf[..]) }
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

    /// Refresh `snapshot_buf` with the current world state in lex
    /// order. Shared between [`World::cells_snapshot`] (clones the
    /// buffer out) and [`World::cells_snapshot_view`] (returns a view
    /// over it). Capacity is retained across calls.
    fn fill_snapshot_buf(&mut self) {
        self.snapshot_buf.clear();
        self.snapshot_buf
            .reserve(self.inner.len() * Self::SNAPSHOT_STRIDE);
        // sorted_iter walks cells in `(x, y, z)` lex order — the snapshot's
        // documented contract. The world's internal FxHashMap iterates in
        // hash order, which is deterministic but not lex.
        for (coord, cell) in self.inner.sorted_iter() {
            self.snapshot_buf.push(coord.x as u32);
            self.snapshot_buf.push(coord.y as u32);
            self.snapshot_buf.push(coord.z as u32);
            self.snapshot_buf.push(cell.energy());
            self.snapshot_buf.push(cell.origin_tag);
            self.snapshot_buf.push(cell.appearance);
        }
    }

    /// Refresh `inspect_buf` with the cell at `(x, y, z)`, or leave
    /// it empty if no such cell exists. Shared between
    /// [`World::cell_inspect`] and [`World::cell_inspect_view`].
    fn fill_inspect_buf(&mut self, x: i32, y: i32, z: i32) {
        self.inspect_buf.clear();
        let coord = aenternis_core::Coord::new(x, y, z);
        let Some(cell) = self.inner.get(coord) else {
            return;
        };
        self.inspect_buf
            .reserve(Self::INSPECT_PREFIX + cell.memory_len());
        self.inspect_buf.push(cell.pc);
        self.inspect_buf.push(cell.energy());
        self.inspect_buf.push(cell.origin_tag);
        self.inspect_buf.push(cell.appearance);
        self.inspect_buf.extend_from_slice(&cell.pointers);
        self.inspect_buf.extend_from_slice(&cell.rates);
        self.inspect_buf.extend_from_slice(&cell.active_outflow);
        self.inspect_buf.extend_from_slice(&cell.inflow);
        self.inspect_buf.extend_from_slice(cell.memory());
    }
}

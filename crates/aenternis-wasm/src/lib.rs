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

use aenternis_core::{tick, Base, PossessError, SparseWorld};
#[cfg(target_arch = "wasm32")]
use js_sys::Uint32Array;
use wasm_bindgen::prelude::*;

// Allocator: stock `dlmalloc` on every target. The previous
// `talc` + `spinning_top` global allocator that the threaded
// build pulled in (2026-05-16 → 2026-05-17) was a band-aid for
// the heap fragmentation that came from ~250 k per-cell
// `Vec<u32>` allocations churning each tick. Phases 1–3 of the
// arena refactor moved cell memory into a single world-owned
// `Arena` (with a double-buffer `arena_next` for compact-by-
// construction writes), so the global allocator now sees one
// big `Vec<u32>` per arena instead of 250 k small ones —
// dlmalloc handles that without contention or coalescing
// pressure even under the rayon worker pool. See
// `docs/optimalizace-2026-05.md`.

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
    fn console_error_alloc_fail(msg: &str, size: u32, align: u32, wasm_memory_pages: u32);
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
    // `memory.size` is a pure WASM instruction with no allocation — safe
    // to call from the alloc-error hook. Returns the current linear-
    // memory size in 64 KiB pages, so JS can distinguish a hit-the-cap
    // failure (`pages` near 65536 = 4 GiB) from a `memory.grow` refusal
    // at a smaller footprint (fragmentation, browser/OS limit, etc.).
    let pages = u32::try_from(core::arch::wasm32::memory_size(0)).unwrap_or(u32::MAX);
    console_error_alloc_fail(
        "Aenternis WASM allocation failure (out of memory). Following: requested size, align, current WASM memory pages (64 KiB each).",
        size,
        align,
        pages,
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

/// Render a [`PossessError`] into a JS-facing message for [`World::possess`].
fn possess_error_message(x: i32, y: i32, z: i32, err: PossessError) -> String {
    match err {
        PossessError::NoCell => format!("possess: no cell at ({x}, {y}, {z})"),
        PossessError::CodeTooLarge { code_len, capacity } => format!(
            "possess: program ({code_len} slots) exceeds host energy ({capacity}) at ({x}, {y}, {z})"
        ),
    }
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
            // Macro genesis (`docs/genesis-plan.md`) is the production
            // default: the origin cell's whole memory is a seed-driven
            // weighted stream of macros — information-bearing, not noise.
            // `Base::Noise` stays for baselines/tests via the core API.
            inner: SparseWorld::big_bang_macros(u64::from(seed), energy),
            // `snapshot_buf` grows on demand via `Vec::reserve`
            // inside `fill_snapshot_buf`. Pre-reserving to
            // `energy * STRIDE` (~24 MB at E = 1 M) tempted us in
            // Phase 4, but a single contiguous request that big at
            // construction time has been observed to fail in the
            // shared-memory WASM environment even when the arenas
            // (4 MB each) succeed. Incremental doubling caps at the
            // same peak as the one-shot pre-reserve, without
            // asking the global allocator for an outsized block on
            // tick 0.
            snapshot_buf: Vec::new(),
            inspect_buf: Vec::new(),
        }
    }

    /// Construct a new world on the **macro-genesis** base with a
    /// programmer-supplied prefix overlaid on the origin cell's memory.
    /// The whole memory is first generated from the seed tape (macro
    /// genesis); then the first `min(program.length, energy)` slots are
    /// overwritten verbatim by `program`. The generated tail past the
    /// prefix is independent of the prefix, so it stays identical
    /// regardless of the program supplied.
    ///
    /// This is the production viewer path — an empty `program` gives pure
    /// macro genesis; a non-empty one seeds the start with the player's
    /// code. (`Base::Noise` + program remains in the core API for
    /// baselines.)
    #[wasm_bindgen(js_name = newWithProgram)]
    #[must_use]
    pub fn new_with_program(seed: u32, energy: u32, program: &[u32]) -> Self {
        Self {
            inner: SparseWorld::big_bang_with(u64::from(seed), energy, Base::Macros, program),
            // See `World::new` for the rationale on not pre-reserving.
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

    /// Possess the cell at `(x, y, z)`: overwrite its leading slots with
    /// `code`, stamp `tag` (lineage marker, e.g. the Pilgrim tag) and
    /// `appearance`, and reset its program counter so the loaded program
    /// runs from the start.
    ///
    /// Energy-neutral: the cell's `mem_len` is unchanged, so the world's
    /// total-energy invariant holds — this is a tool operation, not a
    /// physics event. The host's trailing slots past `code` are left as
    /// dirty scratch. See `docs/pilgrim.md`.
    ///
    /// # Errors
    ///
    /// Throws if no cell exists at `(x, y, z)` (possession can't create
    /// a cell from void — that would conjure energy) or if `code` is
    /// larger than the host cell's energy. The world is left unchanged.
    pub fn possess(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        code: &[u32],
        tag: u32,
        appearance: u32,
    ) -> Result<(), JsValue> {
        let coord = aenternis_core::Coord::new(x, y, z);
        self.inner
            .possess(coord, code, tag, appearance)
            .map_err(|e| JsValue::from_str(&possess_error_message(x, y, z, e)))
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

    /// Set the gravity coupling strength (default `0.0` = off). Energy is
    /// pulled toward local mass; `0.0` keeps the frozen pre-gravity rate
    /// path. See `docs/mechanics.md`.
    #[wasm_bindgen(js_name = setGravity)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_gravity(&mut self, gravity: f64) {
        self.inner.gravity = gravity;
    }

    /// Current gravity strength.
    #[wasm_bindgen(getter, js_name = gravity)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn gravity(&self) -> f64 {
        self.inner.gravity
    }

    /// Set the mass coupling `alpha` in `m = alpha · E` (default `0.0`).
    #[wasm_bindgen(js_name = setGravityAlpha)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_gravity_alpha(&mut self, alpha: f64) {
        self.inner.gravity_alpha = alpha;
    }

    /// Current mass coupling `alpha`.
    #[wasm_bindgen(getter, js_name = gravityAlpha)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn gravity_alpha(&self) -> f64 {
        self.inner.gravity_alpha
    }

    /// Set the gravity cutoff radius `R` (default `1`). `R = 1` is local
    /// (six face neighbors); larger `R` gives genuine long-range
    /// attraction across voids at an `O(N·R³)` cost. See
    /// `docs/mechanics.md`.
    #[wasm_bindgen(js_name = setGravityRadius)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_gravity_radius(&mut self, radius: i32) {
        self.inner.gravity_radius = radius;
    }

    /// Current gravity cutoff radius `R`.
    #[wasm_bindgen(getter, js_name = gravityRadius)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn gravity_radius(&self) -> i32 {
        self.inner.gravity_radius
    }

    /// Set the pressure amplitude (default `0.0` = off).
    #[wasm_bindgen(js_name = setPressure)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_pressure(&mut self, pressure: f64) {
        self.inner.pressure = pressure;
    }

    /// Current pressure amplitude.
    #[wasm_bindgen(getter, js_name = pressure)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn pressure(&self) -> f64 {
        self.inner.pressure
    }

    /// Set the polytropic index γ for the pressure law. **Snapped to the
    /// nearest portable value** `{1.0, 1.5, 2.0, 2.5, 3.0}` — these are
    /// evaluated via multiply/`sqrt` chains so the rate path stays
    /// bit-for-bit reproducible across native and wasm. Arbitrary γ would
    /// need a non-portable `powf` and is out of scope.
    #[wasm_bindgen(js_name = setPressureGamma)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_pressure_gamma(&mut self, gamma: f64) {
        self.inner.pressure_gamma = snap_gamma(gamma);
    }

    /// Current polytropic index γ (already snapped to a portable value).
    #[wasm_bindgen(getter, js_name = pressureGamma)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn pressure_gamma(&self) -> f64 {
        self.inner.pressure_gamma
    }

    /// Set the reference energy `eref` for the pressure law (default
    /// `1.0`). Must be positive; non-positive values would only matter
    /// while pressure is on, where they degrade gracefully to zero rate.
    #[wasm_bindgen(js_name = setPressureEref)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_pressure_eref(&mut self, eref: f64) {
        self.inner.pressure_eref = eref;
    }

    /// Current reference energy `eref`.
    #[wasm_bindgen(getter, js_name = pressureEref)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn pressure_eref(&self) -> f64 {
        self.inner.pressure_eref
    }

    /// Set the mutation strength / ceiling (default `0.0` = off). The
    /// per-slot bit-flip probability for a cell of energy `E` is
    /// `strength · E / (E + half_density)` — a saturating curve, so dense
    /// gravity wells mutate most while sparse cells stay gentle. `0.0`
    /// makes the mutation phase a strict no-op. See `docs/mechanics.md`.
    #[wasm_bindgen(js_name = setMutationStrength)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_mutation_strength(&mut self, strength: f64) {
        self.inner.mutation_strength = strength;
    }

    /// Current mutation strength.
    #[wasm_bindgen(getter, js_name = mutationStrength)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn mutation_strength(&self) -> f64 {
        self.inner.mutation_strength
    }

    /// Set the mutation half-saturation density `K` — the energy at which
    /// the flip probability reaches `strength / 2`. High = only dense
    /// cores mutate hard. See `docs/mechanics.md`.
    #[wasm_bindgen(js_name = setMutationHalfDensity)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_mutation_half_density(&mut self, half_density: f64) {
        self.inner.mutation_half_density = half_density;
    }

    /// Current mutation half-saturation density `K`.
    #[wasm_bindgen(getter, js_name = mutationHalfDensity)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn mutation_half_density(&self) -> f64 {
        self.inner.mutation_half_density
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

    /// Diagnostic snapshot of every container's allocated size, plus
    /// the current WASM linear-memory page count. Returns a flat
    /// `Uint32Array` of length [`World::MEMORY_REPORT_LEN`] = 21.
    ///
    /// Layout — each slot is a `u32`, saturating-cast from the
    /// underlying `usize` / `u64`:
    ///
    /// | offset | field                                       |
    /// |--------|---------------------------------------------|
    /// | `0`    | `wasm_memory_pages` (64 KiB each; native = 0) |
    /// | `1`    | `tick`                                      |
    /// | `2`    | `cell_count`                                |
    /// | `3`    | `cells_slots_len`                           |
    /// | `4`    | `cells_slots_cap`                           |
    /// | `5`    | `cells_free_slots_len`                      |
    /// | `6`    | `cells_free_slots_cap`                      |
    /// | `7`    | `cells_coord_to_slot_cap`                   |
    /// | `8`    | `scratch_neighbor_energies_cap`             |
    /// | `9`    | `scratch_outflow_cap`                       |
    /// | `10`   | `scratch_outflow_inner_vec_cap_sum`         |
    /// | `11`   | `scratch_inflows_by_target_cap`             |
    /// | `12`   | `scratch_inflows_inner_vec_cap_sum`         |
    /// | `13`   | `scratch_per_source_total_outflow_cap`      |
    /// | `14`   | `sorted_cache_len`                          |
    /// | `15`   | `sorted_cache_cap`                          |
    /// | `16`   | `arena_capacity`                            |
    /// | `17`   | `arena_slots_vec_cap`                       |
    /// | `18`   | `arena_next_capacity`                       |
    /// | `19`   | `arena_next_slots_vec_cap`                  |
    /// | `20`   | reserved / zero                             |
    ///
    /// Reserved slot is kept so the layout is a round 21 fields —
    /// adding a new metric uses the slot, no width change. JS unpacks
    /// this against a hand-coded label table in the worker.
    #[must_use]
    #[wasm_bindgen(js_name = memoryReport)]
    pub fn memory_report(&self) -> Vec<u32> {
        let r = self.inner.memory_report();
        #[cfg(target_arch = "wasm32")]
        let pages = u32::try_from(core::arch::wasm32::memory_size(0)).unwrap_or(u32::MAX);
        #[cfg(not(target_arch = "wasm32"))]
        let pages: u32 = 0;
        let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
        let tick = u32::try_from(r.tick).unwrap_or(u32::MAX);
        vec![
            pages,
            tick,
            to_u32(r.cell_count),
            to_u32(r.cells_slots_len),
            to_u32(r.cells_slots_cap),
            to_u32(r.cells_free_slots_len),
            to_u32(r.cells_free_slots_cap),
            to_u32(r.cells_coord_to_slot_cap),
            to_u32(r.scratch_neighbor_energies_cap),
            to_u32(r.scratch_outflow_cap),
            to_u32(r.scratch_outflow_inner_vec_cap_sum),
            to_u32(r.scratch_inflows_by_target_cap),
            to_u32(r.scratch_inflows_inner_vec_cap_sum),
            to_u32(r.scratch_per_source_total_outflow_cap),
            to_u32(r.sorted_cache_len),
            to_u32(r.sorted_cache_cap),
            to_u32(r.arena_capacity),
            to_u32(r.arena_slots_vec_cap),
            to_u32(r.arena_next_capacity),
            to_u32(r.arena_next_slots_vec_cap),
            0,
        ]
    }

    /// Length of the [`World::memory_report`] flat array (= 21). JS
    /// uses this to validate the layout it unpacks against.
    #[must_use]
    #[wasm_bindgen(getter, js_name = memoryReportLen)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn memory_report_len(&self) -> u32 {
        Self::MEMORY_REPORT_LEN as u32
    }
}

impl World {
    /// Number of `u32` fields per cell in [`World::cells_snapshot`].
    /// Available as a constant for Rust-side callers; JS uses the
    /// `snapshotStride` getter. Re-exported from the core so the
    /// snapshot layout has a single definition shared with the native
    /// server (see [`aenternis_core::snapshot`]).
    pub const SNAPSHOT_STRIDE: usize = aenternis_core::snapshot::SNAPSHOT_STRIDE;

    /// Number of `u32` fields in the fixed-width prefix of
    /// [`World::cell_inspect`] before the variable-length memory
    /// region starts. Re-exported from the core alongside
    /// [`Self::SNAPSHOT_STRIDE`].
    pub const INSPECT_PREFIX: usize = aenternis_core::snapshot::INSPECT_PREFIX;

    /// Number of `u32` fields in [`World::memory_report`]'s flat array.
    /// JS validates against [`World::memory_report_len`] (the JS-side
    /// getter) before unpacking, so a layout change here is caught at
    /// the boundary rather than producing silently wrong labels.
    pub const MEMORY_REPORT_LEN: usize = 21;

    /// Refresh `snapshot_buf` with the current world state in lex
    /// order. Shared between [`World::cells_snapshot`] (clones the
    /// buffer out) and [`World::cells_snapshot_view`] (returns a view
    /// over it). Capacity is retained across calls.
    fn fill_snapshot_buf(&mut self) {
        // Layout lives in the core so the WASM and native-server
        // backends emit byte-identical payloads. See
        // [`aenternis_core::snapshot`].
        aenternis_core::snapshot::snapshot_into(&self.inner, &mut self.snapshot_buf);
    }

    /// Refresh `inspect_buf` with the cell at `(x, y, z)`, or leave
    /// it empty if no such cell exists. Shared between
    /// [`World::cell_inspect`] and [`World::cell_inspect_view`].
    fn fill_inspect_buf(&mut self, x: i32, y: i32, z: i32) {
        // Layout lives in the core; see [`World::fill_snapshot_buf`].
        let coord = aenternis_core::Coord::new(x, y, z);
        aenternis_core::snapshot::inspect_into(&self.inner, coord, &mut self.inspect_buf);
    }
}

/// Snap an arbitrary γ to the nearest portable polytropic index in
/// `{1.0, 1.5, 2.0, 2.5, 3.0}`. The core's pressure law only evaluates
/// these (via multiply/`sqrt` chains, all IEEE correctly-rounded), so the
/// boundary clamps user input here rather than letting a non-portable
/// `powf` slip into the deterministic rate path.
fn snap_gamma(gamma: f64) -> f64 {
    const SUPPORTED: [f64; 5] = [1.0, 1.5, 2.0, 2.5, 3.0];
    let mut best = SUPPORTED[0];
    let mut best_dist = (gamma - best).abs();
    for &candidate in &SUPPORTED[1..] {
        let dist = (gamma - candidate).abs();
        if dist < best_dist {
            best = candidate;
            best_dist = dist;
        }
    }
    best
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // snap targets are exactly-representable f64 values
mod snap_gamma_tests {
    use super::snap_gamma;

    #[test]
    fn supported_values_pass_through_unchanged() {
        for g in [1.0, 1.5, 2.0, 2.5, 3.0] {
            assert_eq!(snap_gamma(g), g);
        }
    }

    #[test]
    fn nearby_values_snap_to_the_closest_supported() {
        assert_eq!(snap_gamma(2.1), 2.0);
        assert_eq!(snap_gamma(2.3), 2.5);
        assert_eq!(snap_gamma(1.7), 1.5);
        assert_eq!(snap_gamma(2.9), 3.0);
    }

    #[test]
    fn out_of_range_values_clamp_to_the_ends() {
        assert_eq!(snap_gamma(0.5), 1.0);
        assert_eq!(snap_gamma(-4.0), 1.0);
        assert_eq!(snap_gamma(5.0), 3.0);
        assert_eq!(snap_gamma(100.0), 3.0);
    }

    #[test]
    fn exact_ties_resolve_to_the_lower_index() {
        // 1.25 is equidistant from 1.0 and 1.5; the strict `<` keeps the
        // first (lower) candidate. A `<=` would instead jump to 1.5, and
        // 2.25 → 2.5 — so these pin the tie-break direction.
        assert_eq!(snap_gamma(1.25), 1.0);
        assert_eq!(snap_gamma(2.25), 2.0);
    }
}

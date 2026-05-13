//! Iteration helper that picks between sequential and rayon-parallel
//! walks based on map size, with a single point of WASM-fallback truth.
//!
//! Every per-tick phase in [`crate::tick`] used to repeat the same
//! `#[cfg(target_arch = "wasm32")]` dance — `par_iter` on native, plain
//! `iter_mut` on WASM. Six copies of the same five-line block was the
//! visible cost; the hidden cost was that small worlds always paid
//! rayon's task-spawning overhead even when the work fit comfortably
//! in a single sequential pass. The 1-cell big-bang at simulation
//! start is the extreme case — fork-join overhead dwarfs the actual
//! per-cell update.
//!
//! [`par_or_seq_iter_mut!`] solves both: callers stop worrying about
//! the cfg branch, and the runtime size check picks the cheap path
//! when the parallel one wouldn't earn back its overhead.
//!
//! ## Why a macro, not a generic function
//!
//! Both a generic `fn par_or_seq_iter_mut<K, V, F>(...)` (with
//! `#[inline(always)]`) and a `macro_rules!` form were benched against
//! open-coded `par_iter_mut` on the parallel path
//! (`tick_step/dense_grid/side_32`, 32 768 cells, see
//! `docs/plan-par-or-seq.md` § 6). Both showed a ~40 % regression vs.
//! the original code despite producing structurally similar IR. The
//! macro is chosen because it carries the size-threshold dispatch
//! and the `#[cfg]` fan-out without the function-call layer, and
//! `$body:ident` keeps the user's closure capture context unchanged
//! across substitution (no rebind, no extra closure layer).
//!
//! ## Parallel-path overhead, accepted
//!
//! The ~40 % parallel-path regression survives every variant of the
//! helper we tried (function with `#[inline]` / `#[inline(always)]`,
//! macro with `$body:expr` rebind, macro with `$body:ident`
//! substitution, with/without local `let map = …;` binding,
//! `use rayon::prelude::*` at module or block scope). Open-coding the
//! same call structure at the callsite without the threshold check
//! always wins by ~40 %, so the overhead is structural — adding a
//! runtime size check before the rayon call breaks an LLVM
//! optimisation we couldn't pin down.
//!
//! We accept this because **no realistic Aenternis workload crosses
//! the threshold**: the `tick_step/warm_huge` benches at 500 k and
//! 1 M energy collapse to 604 / 761 cells after a 10-tick warmup
//! (sparse-cluster diffusion). Real simulations stay on the
//! sequential path, where the refactor wins **50–87 %**. The
//! parallel-path cost only surfaces in synthetic dense-grid scenarios
//! that aren't part of the production workload.

/// Below this many entries the per-tick walks stay on the sequential
/// path. At or above it, rayon's `par_iter_mut` takes over.
///
/// First-guess value from `docs/plan-par-or-seq.md` § 3. Calibrate
/// against `cargo bench --bench tick` (`tick_step/cold/*` should
/// improve, `tick_step/dense_grid/*` should stay flat). Tunable only
/// at compile time — a runtime branch inside the per-cell loop is the
/// thing we are removing.
///
/// Gated to "rayon is available" targets: native unconditionally, and
/// `wasm32 + feature = "wasm-threads"` via `wasm-bindgen-rayon`. The
/// default wasm32 build (no feature) skips parallelism entirely and
/// leaves this constant unreferenced — without the cfg gate that would
/// trip `dead_code`.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
pub(crate) const PAR_THRESHOLD: usize = 8_192;

/// Walk `$map: &mut FxHashMap<K, V>` with closure `$body: Fn(&K, &mut V)`.
///
/// Dispatch is `cfg`-driven, three paths:
///
/// - **Native** (`not(target_arch = "wasm32")`) — rayon unconditional,
///   `par_iter_mut().for_each` above [`PAR_THRESHOLD`], sequential
///   below. Rayon ships in `[dependencies]`, no feature gate.
/// - **WASM + `wasm-threads`** — same threshold dispatch as native, but
///   `par_iter_mut` routes through `wasm-bindgen-rayon`'s pthread-over-
///   Web-Workers bridge. Requires `init_thread_pool` called from JS at
///   startup, plus host page in `crossOriginIsolated` mode (COOP/COEP
///   headers or `coi-serviceworker` shim).
/// - **WASM no feature** — plain `iter_mut`, no rayon footprint. The
///   default wasm-pack build; runs single-threaded in any browser, no
///   `SharedArrayBuffer` requirement.
///
/// `$body` must satisfy `Fn(&K, &mut V) + Send + Sync` on either rayon
/// path; the existing per-tick closures in [`crate::tick`] do because
/// their captures are either `Copy` scalars or `&`-borrows of `Sync`
/// types.
macro_rules! par_or_seq_iter_mut {
    ($map:expr, $body:ident $(,)?) => {{
        #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
        if ($map).len() < $crate::parallel::PAR_THRESHOLD {
            for (k, v) in ($map).iter_mut() {
                $body(k, v);
            }
        } else {
            ($map).par_iter_mut().for_each(|(k, v)| $body(k, v));
        }
        #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
        for (k, v) in ($map).iter_mut() {
            $body(k, v);
        }
    }};
}

pub(crate) use par_or_seq_iter_mut;

// Tests exercise the parallel branch; gated to "rayon is available"
// targets along with `PAR_THRESHOLD` so the no-feature wasm32 build
// doesn't trip on a missing constant.
#[cfg(all(test, any(not(target_arch = "wasm32"), feature = "wasm-threads")))]
mod tests {
    use super::PAR_THRESHOLD;
    use rustc_hash::FxHashMap;
    // The macro expansion references `par_iter_mut`/`ParallelIterator`
    // methods; tests resolve them via this prelude (callsite scope).
    use rayon::prelude::*;

    /// Build a `0..n` keyed map with `v = u64::from(k)`.
    fn map_with_n(n: u32) -> FxHashMap<u32, u64> {
        let mut m = FxHashMap::default();
        m.reserve(n as usize);
        for k in 0..n {
            m.insert(k, u64::from(k));
        }
        m
    }

    /// Reference: every value becomes `2 * k`.
    fn assert_doubled(m: &FxHashMap<u32, u64>) {
        for (k, v) in m {
            assert_eq!(*v, u64::from(*k) * 2, "key {k} mismatched");
        }
    }

    #[test]
    fn sequential_path_below_threshold() {
        // PAR_THRESHOLD - 1 forces the sequential branch on native.
        let n = u32::try_from(PAR_THRESHOLD - 1).expect("threshold fits u32");
        let mut m = map_with_n(n);
        let body = |k: &u32, v: &mut u64| *v += u64::from(*k);
        crate::parallel::par_or_seq_iter_mut!(&mut m, body);
        assert_eq!(m.len(), n as usize);
        assert_doubled(&m);
    }

    #[test]
    fn parallel_path_at_threshold() {
        // Exactly PAR_THRESHOLD crosses into the parallel branch on native;
        // on WASM the helper always runs sequentially, but the result
        // must match.
        let n = u32::try_from(PAR_THRESHOLD).expect("threshold fits u32");
        let mut m = map_with_n(n);
        let body = |k: &u32, v: &mut u64| *v += u64::from(*k);
        crate::parallel::par_or_seq_iter_mut!(&mut m, body);
        assert_eq!(m.len(), n as usize);
        assert_doubled(&m);
    }

    #[test]
    fn boundary_matches_reference_loop() {
        // Same body on both sides of the threshold must produce the same
        // per-entry result as a plain sequential loop — guards against a
        // future change to the par_iter side that would silently diverge.
        for n in [
            u32::try_from(PAR_THRESHOLD - 1).expect("fits u32"),
            u32::try_from(PAR_THRESHOLD).expect("fits u32"),
        ] {
            let mut via_helper = map_with_n(n);
            let body = |k: &u32, v: &mut u64| *v += u64::from(*k);
            crate::parallel::par_or_seq_iter_mut!(&mut via_helper, body);

            let mut via_loop = map_with_n(n);
            for (k, v) in &mut via_loop {
                *v += u64::from(*k);
            }

            assert_eq!(via_helper, via_loop, "diverged at n = {n}");
        }
    }

    #[test]
    fn empty_map_is_a_noop() {
        let mut m: FxHashMap<u32, u64> = FxHashMap::default();
        let body = |_k: &u32, v: &mut u64| *v += 1;
        crate::parallel::par_or_seq_iter_mut!(&mut m, body);
        assert!(m.is_empty());
    }
}

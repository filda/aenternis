#!/usr/bin/env bash
# Aenternis — threaded WASM build script.
#
# Builds the `aenternis-wasm` crate with the `wasm-threads` feature
# enabled, producing a multi-threaded WASM bundle that uses
# `wasm-bindgen-rayon` for pthread-over-Web-Workers parallelism.
# Output lands in `crates/aenternis-wasm/pkg/` (the same directory as
# the default single-threaded build), so `web/worker.ts` imports the
# correct bundle regardless of which build was last run.
#
# Captures combined output to `reports/wasm-build.log` (same pattern
# as `scripts/check.sh` with `reports/rust-check.log`), so failures
# can be re-read without scrolling back through terminal history.
#
# Why a separate script: the threaded build requires a pinned nightly
# Rust toolchain (`-Z build-std` is nightly-only) and three target
# features that the default `wasm-pack build` invocation doesn't set.
# Folding all that into `scripts/check.sh` would make the default
# verification gate depend on nightly, which we don't want.
#
# Setup (one-time per dev machine):
#
#     rustup toolchain install nightly-2026-04-15 \
#         --component rust-src \
#         --target wasm32-unknown-unknown
#
# Bumping the pinned nightly: edit `NIGHTLY_TOOLCHAIN` below, install
# the new toolchain with the same `rustup toolchain install` command,
# then re-run this script and verify the bundle still works in the
# browser (Chrome DevTools console: `crossOriginIsolated` must be
# `true`, `await world.step()` must not throw on thread-pool init).
#
# Usage:
#
#     bash scripts/build-wasm.sh
#
# Exit code 0 on success, non-zero on any failure (missing toolchain,
# missing rust-src, build error). On failure, `reports/wasm-build.log`
# holds the full captured output.

set -u

# Resolve repo root from the script location, so this runs from any cwd.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

REPORT_DIR="reports"
LOG="${REPORT_DIR}/wasm-build.log"

mkdir -p "${REPORT_DIR}"
: > "${LOG}"

# Shared with scripts/check.sh — defines `WASM_THREADED_TOOLCHAIN`
# (the pinned nightly) and `wasm_threaded_build_available` (predicate
# this script doesn't use, but check.sh does).
# shellcheck source=_wasm-threaded-toolchain.sh
source "$(dirname "$0")/_wasm-threaded-toolchain.sh"

# Cross-platform ISO-8601 timestamp.
timestamp() {
    date -Iseconds 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ
}

# Mirrors `scripts/check.sh`'s `step()`: prints a banner + command,
# tees the combined stdout/stderr to both terminal and log file, and
# records the failure tag without short-circuiting the rest of the
# script. Returns the wrapped command's exit code so the caller can
# decide what to do.
step() {
    local label="$1"; shift
    {
        echo
        echo "==================================================================="
        echo " step: ${label}"
        echo " \$ $*"
        echo "==================================================================="
    } | tee -a "${LOG}"
    "$@" 2>&1 | tee -a "${LOG}"
    local rc="${PIPESTATUS[0]}"
    if [ "${rc}" -ne 0 ]; then
        echo "[step '${label}' failed: rc=${rc}]" | tee -a "${LOG}"
    fi
    return "${rc}"
}

# Convenience for messages outside of a `step` — keeps them in the log.
log() {
    echo "$@" | tee -a "${LOG}"
}

# Pinned nightly toolchain. The pin itself lives in
# `_wasm-threaded-toolchain.sh` so `scripts/check.sh` can read it
# without duplicating the string; bumping it is a deliberate,
# reviewed change.
NIGHTLY_TOOLCHAIN="${WASM_THREADED_TOOLCHAIN}"

{
    echo "Aenternis threaded WASM build"
    echo " repo:      ${REPO_ROOT}"
    echo " toolchain: ${NIGHTLY_TOOLCHAIN}"
    echo " started:   $(timestamp)"
} | tee -a "${LOG}"

# --- prerequisite checks --------------------------------------------------

if ! command -v rustup >/dev/null 2>&1; then
    log "ERROR: rustup not found on PATH."
    log "Install rustup from https://rustup.rs/."
    exit 1
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
    log "ERROR: wasm-pack not found on PATH."
    log "Install with: cargo install wasm-pack"
    exit 1
fi

if ! rustup toolchain list | grep -q "^${NIGHTLY_TOOLCHAIN}"; then
    log "ERROR: required nightly toolchain '${NIGHTLY_TOOLCHAIN}' is not installed."
    log "Install with:"
    log "  rustup toolchain install ${NIGHTLY_TOOLCHAIN} \\"
    log "      --component rust-src \\"
    log "      --target wasm32-unknown-unknown"
    exit 1
fi

# `rust-src` is what enables `-Z build-std` to rebuild std with the
# wasm32 atomics target-feature. Without it, build-std fails late in
# the cargo run with a confusing message.
if ! rustup component list \
        --toolchain "${NIGHTLY_TOOLCHAIN}" --installed \
        | grep -q "^rust-src"; then
    log "ERROR: 'rust-src' component missing on '${NIGHTLY_TOOLCHAIN}'."
    log "Install with:"
    log "  rustup component add rust-src --toolchain ${NIGHTLY_TOOLCHAIN}"
    exit 1
fi

# --- the actual build ------------------------------------------------------

# RUSTFLAGS is the canonical `wasm-bindgen-rayon` recipe (see the
# project's README). Each piece is load-bearing — dropping any of
# them makes the threaded build fail somewhere downstream in a way
# that doesn't immediately point back to the missing flag.
#
# 1. `target-feature=+atomics,+bulk-memory` — wasm32 features that
#    `wasm-bindgen-rayon`'s pthread support depends on. Atomic
#    instructions and bulk memory ops.
# 2. `link-arg=--shared-memory` — declares the module's memory as
#    `shared`. Without this, `postMessage` can't structured-clone
#    `WebAssembly.Memory` to worker threads → runtime
#    `DataCloneError: #<Memory> could not be cloned`.
# 3. `link-arg=--max-memory=1073741824` — required by the WASM spec
#    when memory is shared (1 GiB chosen to match the recipe; the
#    actual heap grows on demand, this is just the upper bound).
# 4. `link-arg=--import-memory` — emit memory as an import rather
#    than a definition. wasm-bindgen's threading transform asserts
#    `mem.import.is_some()` and panics otherwise; wasm-bindgen-rayon
#    creates the shared `WebAssembly.Memory` once on the JS side and
#    passes it to every instantiation (main + each rayon worker).
# 5. `link-arg=--export=__wasm_init_tls`, `--export=__tls_size`,
#    `--export=__tls_align`, `--export=__tls_base` — TLS bootstrap
#    symbols that LLD emits but does not export by default. The
#    threading transform looks them up by name; without the explicit
#    exports it errors with `failed to find __wasm_init_tls`.
#
# The matching `-Z build-std` flag (which rebuilds std with these
# settings) lives in `crates/aenternis-wasm/.cargo/config.toml` —
# wasm-pack's CLI parser rejects `-Z` even after `--`, so the
# config-file route is the working alternative.
#
# RUSTUP_TOOLCHAIN overrides the workspace's `rust-toolchain.toml`
# (which pins stable) only for this invocation — native builds and the
# default `cargo` commands keep using stable.
#
# Expected (and unsuppressible) warning during this build:
#
#     warning: unstable feature specified for `-Ctarget-feature`: `atomics`
#       = note: this feature is not stably supported; its behavior can change in the future
#
# `atomics` IS officially unstable as a wasm32 target-feature — that's
# the entire reason this build needs nightly. The warning is emitted
# below the lint-suppression layer (it comes from codegen, not the
# normal lint pipeline), so there is no `#[allow(...)]` or
# `RUSTFLAGS` setting that quiets it. Two copies appear (one per
# downstream crate that consumes the flag). Ignore them; they are
# not a regression to chase.
export RUSTUP_TOOLCHAIN="${NIGHTLY_TOOLCHAIN}"
export RUSTFLAGS="\
-C target-feature=+atomics,+bulk-memory \
-C link-arg=--shared-memory \
-C link-arg=--max-memory=1073741824 \
-C link-arg=--import-memory \
-C link-arg=--export=__wasm_init_tls \
-C link-arg=--export=__tls_size \
-C link-arg=--export=__tls_align \
-C link-arg=--export=__tls_base"

OVERALL=0
step "wasm-pack" wasm-pack build crates/aenternis-wasm \
    --target web \
    --features wasm-threads \
    || OVERALL=1

# Post-build: rewrite `import('../../..')` in wasm-bindgen-rayon's
# worker bootstrap to an explicit file URL. See the function's
# header for why this is necessary on a bundlerless production
# host like GitHub Pages.
patch_wasm_bindgen_rayon_worker_helpers

# Final gate before declaring the build OK: run the artifact-shape
# verifier on the freshly built `pkg/`. Each check there names the
# class of regression it catches; if anything fires, the error
# message points straight at the misbehaving step.
step "verify-threaded-wasm" bash scripts/verify-threaded-wasm.sh \
    || OVERALL=1

{
    echo
    echo "==================================================================="
    if [ "${OVERALL}" -eq 0 ]; then
        echo " WASM build OK — output at crates/aenternis-wasm/pkg/"
        echo " JS side must call \`await initThreadPool(navigator.hardwareConcurrency)\`"
        echo " after \`await init()\` to actually spawn the worker pool."
    else
        echo " WASM BUILD FAILED (rc=${OVERALL})"
    fi
    echo " finished: $(timestamp)"
    echo "==================================================================="
} | tee -a "${LOG}"

# Flush filesystem buffers before exit. See `scripts/check.sh` for why.
sync 2>/dev/null || true

exit "${OVERALL}"

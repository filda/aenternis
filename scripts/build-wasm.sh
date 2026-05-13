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
# missing rust-src, build error). The script aborts early when the
# prerequisites are missing rather than spending a minute compiling
# things that can't finish.

set -euo pipefail

# Resolve repo root from the script location, so this runs from any cwd.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

# Pinned nightly toolchain. Explicit pin (not "latest nightly") so the
# build is reproducible — bumping is a deliberate, reviewed change.
NIGHTLY_TOOLCHAIN="nightly-2026-04-15"

# --- prerequisite checks --------------------------------------------------

if ! command -v rustup >/dev/null 2>&1; then
    echo "ERROR: rustup not found on PATH." >&2
    echo "Install rustup from https://rustup.rs/." >&2
    exit 1
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "ERROR: wasm-pack not found on PATH." >&2
    echo "Install with: cargo install wasm-pack" >&2
    exit 1
fi

if ! rustup toolchain list | grep -q "^${NIGHTLY_TOOLCHAIN}"; then
    echo "ERROR: required nightly toolchain '${NIGHTLY_TOOLCHAIN}' is not installed." >&2
    echo "Install with:" >&2
    echo "  rustup toolchain install ${NIGHTLY_TOOLCHAIN} \\" >&2
    echo "      --component rust-src \\" >&2
    echo "      --target wasm32-unknown-unknown" >&2
    exit 1
fi

# `rust-src` is what enables `-Z build-std` to rebuild std with the
# wasm32 atomics target-feature. Without it, build-std fails late in
# the cargo run with a confusing message.
if ! rustup component list \
        --toolchain "${NIGHTLY_TOOLCHAIN}" --installed \
        | grep -q "^rust-src"; then
    echo "ERROR: 'rust-src' component missing on '${NIGHTLY_TOOLCHAIN}'." >&2
    echo "Install with:" >&2
    echo "  rustup component add rust-src --toolchain ${NIGHTLY_TOOLCHAIN}" >&2
    exit 1
fi

# --- the actual build ------------------------------------------------------

# RUSTFLAGS enables the three wasm32 target-features that
# `wasm-bindgen-rayon`'s pthread support depends on. The prebuilt
# wasm32-unknown-unknown std does not have these enabled, which is why
# `-Z build-std` (below) rebuilds std and panic_abort locally with
# RUSTFLAGS applied.
#
# RUSTUP_TOOLCHAIN overrides the workspace's `rust-toolchain.toml`
# (which pins stable) only for this invocation — native builds and the
# default `cargo` commands keep using stable.
export RUSTUP_TOOLCHAIN="${NIGHTLY_TOOLCHAIN}"
export RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals"

# `--` separates wasm-pack args from cargo args. `-Z build-std=...`
# flows through to cargo and triggers a local std rebuild.
wasm-pack build crates/aenternis-wasm \
    --target web \
    --features wasm-threads \
    -- \
    -Z build-std=panic_abort,std

echo
echo "Threaded WASM bundle built at crates/aenternis-wasm/pkg/."
echo "JS side must call \`await initThreadPool(navigator.hardwareConcurrency)\`"
echo "after \`await init()\` to actually spawn the worker pool."

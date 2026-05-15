#!/usr/bin/env bash
# Aenternis — Rust verification gate.
#
# Runs the three Rust checks (fmt, clippy, test) and writes their combined
# output to reports/rust-check.log. The log can then be copied back into
# the live mount so the agent can read it without a chat round-trip.
#
# Usage (from the sandbox clone, NOT the live mount — AGENTS.local.md):
#
#     bash scripts/check.sh
#
# Then copy the log back to the mount, e.g.:
#
#     cp reports/rust-check.log /c/Users/fsubr/workspace/aenternis/reports/
#
# Exit code:
#     0  — all three steps passed
#     1  — at least one step failed (each step still runs; we never short-
#          circuit, so a fmt nit doesn't hide a clippy or test failure)

set -u

# Resolve repo root from the script location, so it works regardless of cwd.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

REPORT_DIR="reports"
LOG="${REPORT_DIR}/rust-check.log"

mkdir -p "${REPORT_DIR}"
: > "${LOG}"

# Pinned nightly + predicate, shared with scripts/build-wasm.sh. We use
# the predicate below to pick between threaded and default wasm-pack
# builds, so the gate's wasm step leaves `pkg/` matching whichever
# bundle the dev's runtime is going to want.
# shellcheck source=_wasm-threaded-toolchain.sh
source "$(dirname "$0")/_wasm-threaded-toolchain.sh"

# Cross-platform ISO-8601 timestamp. GNU `date -Iseconds` is fine on Linux
# but BusyBox/MinGW date may not support it; fall back to plain UTC format.
timestamp() {
    date -Iseconds 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ
}

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

{
    echo "Aenternis Rust check"
    echo " repo: ${REPO_ROOT}"
    echo " started: $(timestamp)"
} | tee -a "${LOG}"

# On Windows (Git Bash / MSYS2) the default gnu toolchain pulls in
# `windows-sys` via tokio + axum, and linking it requires `dlltool.exe`
# from mingw-w64 binutils. The MSVC toolchain takes a different path
# that needs no dlltool, so on Windows we force it via `+toolchain`.
# `rust-toolchain.toml` stays portable (`channel = "stable"`); CI and
# non-Windows hosts keep using whatever it resolves to.
case "${OSTYPE:-}" in
    msys*|cygwin*|win32*) CARGO=(cargo "+stable-x86_64-pc-windows-msvc") ;;
    *)                    CARGO=(cargo) ;;
esac

OVERALL=0
step "rustfmt" "${CARGO[@]}" fmt --all -- --check                                || OVERALL=1
step "clippy"  "${CARGO[@]}" clippy --workspace --all-targets -- -D warnings     || OVERALL=1
step "tests"   "${CARGO[@]}" test --workspace                                    || OVERALL=1
# step "tests"   "${CARGO[@]}" install cargo-llvm-cov                              || OVERALL=1
# step "tests"   "${CARGO[@]}" llvm-cov --workspace --html                         || OVERALL=1

# Optional WASM bundle build. Skipped if wasm-pack isn't on PATH so
# the script keeps working before the toolchain is set up.
#
# Dispatch: when the pinned nightly toolchain (`rust-src` included) is
# installed, build with the `wasm-threads` feature. That way the
# gate's wasm step leaves `pkg/` containing a threaded bundle —
# matching what `web/worker.ts` initializes at runtime when the host
# page is `crossOriginIsolated`. Without this dispatch the gate would
# silently overwrite `pkg/` with the single-threaded bundle every run,
# breaking threading on the next dev session until `build-wasm.sh` is
# manually re-run.
#
# Fallback path (no nightly available) builds the default single-
# threaded bundle, same as before. Both paths exercise wasm-bindgen +
# wasm-opt, so the gate's "the crate compiles to wasm and packages"
# guarantee is preserved either way.
if command -v wasm-pack >/dev/null 2>&1; then
    if wasm_threaded_build_available; then
        # Same `RUSTFLAGS` + config.toml + nightly setup as
        # scripts/build-wasm.sh; kept inline rather than delegated so
        # output lands in this single log file. See build-wasm.sh's
        # RUSTFLAGS comment for what each flag does and why it's
        # required.
        RUSTUP_TOOLCHAIN="${WASM_THREADED_TOOLCHAIN}" \
        RUSTFLAGS="-C target-feature=+atomics,+bulk-memory -C link-arg=--shared-memory -C link-arg=--max-memory=1073741824 -C link-arg=--import-memory -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base" \
        step "wasm-pack (threaded)" \
            wasm-pack build crates/aenternis-wasm --target web --features wasm-threads \
            || OVERALL=1
        # Mirror build-wasm.sh's post-build patch — see the function
        # header in `_wasm-threaded-toolchain.sh` for why.
        patch_wasm_bindgen_rayon_worker_helpers
        # Same gate as build-wasm.sh — catches build-flag regressions
        # and missed patches before the bundle ships.
        step "verify-threaded-wasm" bash scripts/verify-threaded-wasm.sh \
            || OVERALL=1
    else
        step "wasm-pack" wasm-pack build crates/aenternis-wasm --target web   || OVERALL=1
    fi
else
    {
        echo
        echo "==================================================================="
        echo " step: wasm-pack (skipped — wasm-pack not on PATH)"
        echo "==================================================================="
    } | tee -a "${LOG}"
fi

{
    echo
    echo "==================================================================="
    if [ "${OVERALL}" -eq 0 ]; then
        echo " ALL GREEN"
    else
        echo " SOMETHING FAILED (overall rc=${OVERALL})"
    fi
    echo " finished: $(timestamp)"
    echo "==================================================================="
} | tee -a "${LOG}"

# Flush filesystem buffers before exit. Without this, copying the log
# back to the live mount immediately after script termination can
# capture a truncated view — Git Bash's tee on Windows holds onto
# parts of stdout in the pipe buffer until the next sync.
sync 2>/dev/null || true

exit "${OVERALL}"

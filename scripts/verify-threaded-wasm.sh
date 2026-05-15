#!/usr/bin/env bash
# Aenternis — threaded WASM bundle verifier.
#
# Asserts that a `wasm-pack` output directory carries a working
# threaded bundle. Each check below catches a specific class of
# silent regression — when one of them fires, the matching root
# cause is named in the error message so the failure does not turn
# into another live-in-browser debug session.
#
# The checks mirror the live bugs we hit while landing
# `plan-wasm-zerocopy-threads`:
#
# 1. Required wasm-pack outputs present (sanity).
# 2. `wasm-bindgen-rayon` snippet was emitted (threaded build ran).
# 3. `workerHelpers.js` had its `import('../../..')` rewritten to an
#    explicit file URL — without the patch the rayon worker bootstrap
#    requests `pkg/` as a directory on bundlerless hosts (GitHub Pages
#    returns 404).
# 4. `aenternis_wasm.d.ts` exports `initThreadPool` — without it the
#    JS-side runtime detection in `web/worker.ts` falls back to single
#    threaded silently.
# 5. `aenternis_wasm.js` instantiates a `WebAssembly.Memory` with
#    `shared: true` — proof that `-C link-arg=--shared-memory` and
#    `--import-memory` reached the linker. Without this, `postMessage`
#    in the rayon worker init throws `DataCloneError: #<Memory>`.
# 6. The `__aenternis_wasm_start` symbol is present — proof that the
#    `#[wasm_bindgen(start)]`-bound panic hook was wired through. Its
#    absence would mean Rust panics surface as `RuntimeError:
#    unreachable` with no message.
#
# Usage:
#
#     bash scripts/verify-threaded-wasm.sh [path/to/pkg]
#
# Defaults to `crates/aenternis-wasm/pkg`. Called from
# `scripts/build-wasm.sh` and `scripts/check.sh`'s threaded path so
# every locally-produced bundle runs through this gate before anyone
# uploads it.

set -eu

# Resolve repo root so default paths work from any cwd.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG_DIR="${1:-${REPO_ROOT}/crates/aenternis-wasm/pkg}"

# Convert relative arg to absolute so error messages are unambiguous.
case "${PKG_DIR}" in
    /*) ;;
    *) PKG_DIR="$(cd "${REPO_ROOT}" && cd "${PKG_DIR}" && pwd)" ;;
esac

fail() {
    echo "ERROR: $*" >&2
    exit 1
}

# --- 1. required outputs ---------------------------------------------------

for rel in aenternis_wasm.js aenternis_wasm_bg.wasm aenternis_wasm.d.ts; do
    [ -f "${PKG_DIR}/${rel}" ] || fail "missing ${PKG_DIR}/${rel}"
done

# --- 2. wasm-bindgen-rayon snippet present --------------------------------

helpers="$(find "${PKG_DIR}/snippets" -name workerHelpers.js 2>/dev/null | head -1 || true)"
[ -n "${helpers}" ] || fail \
    "workerHelpers.js not found under ${PKG_DIR}/snippets — wasm-pack ran without --features wasm-threads, or wasm-bindgen-rayon was not pulled in"

# --- 3. workerHelpers.js patched ------------------------------------------

if ! grep -q "import('../../../aenternis_wasm.js')" "${helpers}"; then
    fail "${helpers} still has unpatched \`import('../../..')\` — \`patch_wasm_bindgen_rayon_worker_helpers\` was not called after wasm-pack, production deploy will 404 on \`pkg/\`"
fi

# --- 4. initThreadPool exported in .d.ts ----------------------------------

grep -q "initThreadPool" "${PKG_DIR}/aenternis_wasm.d.ts" || fail \
    "${PKG_DIR}/aenternis_wasm.d.ts is missing \`initThreadPool\` — \`--features wasm-threads\` was not effective, threaded bundle reverts to single-threaded silently in worker.ts"

# --- 5. shared WebAssembly.Memory in generated JS -------------------------

grep -q "shared:true" "${PKG_DIR}/aenternis_wasm.js" || fail \
    "${PKG_DIR}/aenternis_wasm.js does not create a shared WebAssembly.Memory — \`-C link-arg=--shared-memory\` and \`--import-memory\` linker flags were lost"

# --- 6. panic hook start symbol present -----------------------------------

grep -q "__aenternis_wasm_start" "${PKG_DIR}/aenternis_wasm.js" || fail \
    "${PKG_DIR}/aenternis_wasm.js is missing \`__aenternis_wasm_start\` — \`#[wasm_bindgen(start)]\` panic hook installer was not generated, future Rust panics will not surface as readable console errors"

echo "OK: threaded WASM bundle at ${PKG_DIR} passes 6/6 shape checks"

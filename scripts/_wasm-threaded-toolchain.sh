# Shared between scripts/build-wasm.sh and scripts/check.sh.
# Source-only, not directly executable — the leading underscore is the
# convention. Defines the pinned nightly toolchain that aenternis-wasm's
# `wasm-threads` feature build uses, plus a predicate that tests if
# that toolchain is fully installed on the current host.
#
# `scripts/build-wasm.sh` is the canonical user-facing entrypoint for
# the threaded build; it consumes `WASM_THREADED_TOOLCHAIN` and its
# own (more verbose) prereq logic. `scripts/check.sh` consumes
# `wasm_threaded_build_available` to decide whether the verification
# gate should produce a threaded `pkg/` (when nightly is around) or
# fall back to the default single-threaded `wasm-pack build`. Without
# that dispatch, the gate would silently overwrite `pkg/` with the
# single-threaded bundle and the browser would then load that on the
# next dev session — surprising the dev who just ran `build-wasm.sh`.

# Pinned nightly. Bump explicitly; see `scripts/build-wasm.sh`'s header
# for what verifying a bump entails.
WASM_THREADED_TOOLCHAIN="nightly-2026-04-15"

# Returns 0 (success) if the threaded WASM build can run on this host:
#   - rustup is on PATH
#   - the pinned nightly toolchain is installed
#   - the `rust-src` component is present on that toolchain
# Returns 1 otherwise. Silent on stderr — callers print their own
# user-facing message.
wasm_threaded_build_available() {
    command -v rustup >/dev/null 2>&1 || return 1
    rustup toolchain list 2>/dev/null \
        | grep -q "^${WASM_THREADED_TOOLCHAIN}" \
        || return 1
    rustup component list \
            --toolchain "${WASM_THREADED_TOOLCHAIN}" --installed 2>/dev/null \
        | grep -q "^rust-src" \
        || return 1
    return 0
}

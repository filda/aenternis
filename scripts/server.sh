#!/usr/bin/env bash
# Aenternis — start the native dev backend (`aenternis-server`).
#
# Wraps `cargo run -p aenternis-server` so the right toolchain is
# picked on each host:
#   - Windows (Git Bash / MSYS2): force MSVC via `+stable-x86_64-pc-windows-msvc`.
#     `rust-toolchain.toml` resolves `channel = "stable"` to the host
#     gnu target, which pulls in `windows-sys` linking through
#     `dlltool.exe` (not on PATH in a typical Git Bash setup).
#   - Linux / macOS / CI: plain `cargo`, since the host toolchain is
#     already what the workspace expects.
#
# Usage (any extra args pass through to the binary):
#
#     bash scripts/server.sh
#     bash scripts/server.sh --host 0.0.0.0 --port 9000
#
# Run from the **sandbox clone** so the `target/` tree doesn't mix
# Linux and Windows artifacts (AGENTS.local.md). The mount is for
# Read/Edit/Write only.

set -u

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${REPO_ROOT}"

case "${OSTYPE:-}" in
    msys*|cygwin*|win32*) CARGO=(cargo "+stable-x86_64-pc-windows-msvc") ;;
    *)                    CARGO=(cargo) ;;
esac

exec "${CARGO[@]}" run -p aenternis-server -- "$@"

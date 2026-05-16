# Aenternis Agent Notes

## Working Rules

- Read [README.md](README.md) and the relevant doc in `docs/` (start with `docs/aenternis.md`) before non-trivial work. The design corpus is dense and most "obvious" approaches were already considered and rejected — check before reinventing.
- Each prototype in `prototypes/` is a **self-contained laboratory experiment**, not a stepping stone toward a single product. Don't generalize across prototypes or refactor shared infra unless explicitly asked. The whole point of the prototype phase is throwaway exploration of one specific question per directory.
- Before changing a prototype, read its own README (often Czech) — it documents what the prototype is *for* and what is intentionally out of scope.
- **All production code in `src/` must have tests.** A bugfix is not complete without a test that would fail before the fix and pass after it. New behavior is not complete without a test that exercises it.
- **The verification gate is `./check`** (fast — TS typecheck + vitest with coverage + Rust fmt/clippy/test + WASM build). After algorithmic changes or before declaring a piece of work done, run **`./check --mutation`** — adds Stryker (JS) and `cargo mutants` (Rust) on top. All stages must pass:
  - Coverage thresholds (vitest): 95% lines / 95% functions / 90% branches / 95% statements over `src/`.
  - Mutation thresholds (Stryker): break at 70%, low at 80%, high at 90%. Aim for 100% — the codebase is small enough.
  - Surviving mutants mean a missing assertion. Add one; do not weaken the threshold.
- For trivial edits (typo, comment, single-line config tweak) `npm run test` (no mutation, no coverage) is an acceptable explicit shortcut. State the shortcut when you take it.
- For browser / visual / UX work in prototypes, manual repro steps in chat are sufficient — see "Prototypes are exempt" below.
- **Prototypes are exempt from the test gate.** Files under `prototypes/**` are throwaway laboratory experiments. They are explicitly excluded from coverage and mutation runs (see `vitest.config.js` and `stryker.conf.json`). Don't write tests for prototype code; don't refactor prototype code in service of testability.
- For sim-logic changes (when they hit `src/`), a counting / conservation check (`isEnergyConserved`, `totalSlots`) over many ticks counts as a real test and should be added.
- The Rust workspace lives at the repository root (`Cargo.toml` + `crates/aenternis-core/`). The Rust verification gate is **`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`** — all three must pass after every non-trivial change to a Rust file. CI runs this automatically (see `.github/workflows/ci.yml`, job `rust`).
- Coverage and mutation testing on the Rust side will be added once the core has more than skeleton modules: `cargo llvm-cov` for coverage and `cargo mutants` for mutation testing (analogue of the JS Stryker pipeline). Same expectation: thresholds high, surviving mutants are missing assertions.
- When working in Rust, follow the same "all production code must have tests" rule. Tests can live next to the module (`#[cfg(test)] mod tests`) for unit work or under `tests/` for integration. The `aenternis-core` crate has a `tests/coord.rs` example to mirror.
- The production Rust + WASM implementation targets sparse 3D from day one (see `docs/plan.md`). Toroidal models live only as fixed-N reference implementations for the bit-identity harness against the JS prototypes.
- A second backend lives alongside WASM: **`aenternis-server`** (`crates/aenternis-server/`) is a native Rust binary that hosts a shared `SparseWorld` over WebSocket on `ws://127.0.0.1:8765/sim` (configurable). The viewer (`web/main.ts`) picks WASM (default) or native via `?backend=...` URL flag or a checkbox in the side panel. The wire format mirrors `src/protocol.ts`: JSON for control, binary frames for snapshot and cellDetail. See `docs/native-server.md` for the dev workflow and operational notes.

## Project Basics

- Aenternis is a 3D toroidal simulation. Every cell is a programmable micro-computer with energy, a memory of 32-bit slots, six directional rate / pointer registers, and a tiny VM with ~20 opcodes.
- Higher-level phenomena — entities, organisms, movement, reproduction, combat, communication — are meant to **emerge from the physics**, not be hard-coded. Rule of thumb: if a feature requires an exception in the cell update, it is the wrong feature.
- The current state is the **prototype phase**: 8 self-contained browser pages in `prototypes/`, each verifying one specific physics or programmer-interface question. None of these prototypes is the production target.
- Production target is **Rust + WASM**, served by the Vite dev server. Today's JavaScript prototypes are intentionally low-friction so design questions are answered cheaply.
- Dev server: `npm run dev` (Vite on port 5173). The dev server is required for prototype 8 Web Worker mode (Chrome blocks workers from `file://` null origin); other prototypes also work when opened directly via `file://`.

## Language

- Top-level repository documentation (`README.md`, `LICENSE`) is in **English**.
- Design / process documents in `docs/` (mechanics, VM, plan, questions, prototypes index) are in **Czech**. Filip writes them in Czech and wants them to stay that way — they are a long-running written record, not an outward-facing manual.
- Each `prototypes/*/README.md` is a Czech lab note. Match the existing language of the file when editing.
- Source code identifiers and code comments: **English** in production-facing code (the future Rust + WASM core). Prototype JavaScript may carry Czech comments where the surrounding file already does — match the file.
- Chat with Filip is in **Czech**. New Czech docs should match the tone of the existing ones: technical, prose-heavy, no excessive bulleting.

## Cleanup pass

After non-trivial work, before declaring done:

1. **`./check --mutation`** — full gate (TS + Rust + WASM + Stryker + cargo mutants). All stages must pass; surviving mutants mean a missing assertion.
2. Re-read the diff as if it were someone else's PR. Look for dead code, stale comments, leaked concerns from earlier iterations, TODO leftovers.
3. Coverage report (`reports/coverage/index.html`) — check the touched area and cover meaningful gaps. Mutation report (`reports/mutation/index.html`) — chase any survivors with a new assertion, do not relax the threshold.
4. If the implementation revealed duplication or awkward abstractions, address them while context is fresh — but only **inside the prototype you touched** when working in prototypes, or inside the changed module when working in `src/`. Cross-cutting refactors are out of scope unless asked.
5. Update the prototype README, `PROTOTYPY.md`, or `PLAN.md` if behavior or scope changed. Designs that aren't written down don't survive the next session.

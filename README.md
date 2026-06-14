# Aenternis

<!-- CI badge: replace OWNER/REPO with the GitHub path once the repo is published.
     Workflow file lives at .github/workflows/ci.yml and the badge resolves automatically:
     ![CI](https://github.com/OWNER/REPO/actions/workflows/ci.yml/badge.svg) -->

A 3D simulation where every cell is a latent micro-computer with its own program, energy, and memory pointers. Higher-level phenomena — entities, organisms, movement, reproduction, combat, communication — emerge from the physics of energy flow and programmable content.

The model shifts away from "entities living in space" toward "space is made of micro-entities". The boundary between an empty cell and an active entity is a spectrum of organization, energy, and program coherence — not a binary flag.

> An entity is not an object in space, but the continuity of a program in an energetic-informational field.

## Documentation

Start with **[docs/aenternis.md](docs/aenternis.md)** — the design core, vocabulary, and key invariants. From there:

- **[docs/mechanics.md](docs/mechanics.md)** — physics in detail (energy, diffusion, pointers, dominance, collision, combat, communication)
- **[docs/vm.md](docs/vm.md)** — virtual machine specification and instruction set
- **[docs/prototypes.md](docs/prototypes.md)** — the laboratory prototypes and what each one verified
- **[docs/plan.md](docs/plan.md)** — current implementation status and the road ahead
- **[docs/questions.md](docs/questions.md)** — open questions and resolved decisions

## Status

Past the prototype phase. Twelve laboratory web prototypes live in `prototypes/` (each verifying a layer of physics or programmer interface), but the production engine has moved to a **Rust + WASM core** with a sparse, unbounded world. Implemented there: a 31-opcode VM with a 100 %-density decode fold, dominance / intrusion (collision as soft mixing), gravity / pressure, density-coupled mutation, procedural macro-genesis, and a Three.js viewer with an inspector and war-paint / lineage color modes. Still on the backlog: the UI lineage tracker + manual tag, and the additional sensor opcodes — see [docs/plan.md](docs/plan.md).

The lab prototypes are intentionally low-friction JavaScript so design questions can be answered cheaply; the production code lives in `crates/` (Rust core + WASM) and `src/` + `web/` (the TypeScript viewer).

## Setup

### Rust

Install Rust via [rustup](https://rustup.rs/):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows use the installer from <https://rustup.rs/> instead.

The `rust-toolchain.toml` at the repo root pins the toolchain automatically — `rustup` will download the right stable channel and add `clippy` and `rustfmt` on first use. The minimum supported version is **Rust 1.78**.

#### System C toolchain (linker)

`rustup` does **not** install a C linker, but `cargo build` needs one (otherwise you'll see `error: linker 'cc' not found`). Install it via your OS package manager:

- **Debian / Ubuntu / WSL**: `sudo apt install build-essential pkg-config`
- **Fedora / RHEL**: `sudo dnf install gcc pkgconf-pkg-config`
- **Arch**: `sudo pacman -S base-devel`
- **macOS**: `xcode-select --install`
- **Windows**: install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the "Desktop development with C++" workload (the `rustup` installer will prompt you if missing)

Verify the installation:

```sh
cargo --version   # e.g. cargo 1.87.0 (stable)
cc --version      # any version is fine — cargo just needs *a* linker
```

### WASM target + wasm-pack (only needed for the WASM bundle)

The WASM build (used by the `web/` frontend and the GitHub Pages deployment) requires one extra target and the `wasm-pack` tool:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

> `wasm-pack` can also be installed without Cargo — see <https://rustwasm.github.io/wasm-pack/installer/> for OS-specific options.

Build everything (TS + WASM):

```sh
./build
```

`./build` runs a two-pass WASM build: a stable single-threaded pass (CI parity, catches stable-only regressions) followed by the threaded nightly pass that produces the canonical `crates/aenternis-wasm/pkg/` consumed by `web/worker.ts`. If only a quick single-threaded bundle is needed:

```sh
wasm-pack build crates/aenternis-wasm --target web
```

#### Multi-threaded WASM (optional)

For large worlds (≥ 100 k cells), the single-threaded WASM bundle becomes the frame-rate bottleneck. The `wasm-threads` feature flag enables a multi-threaded bundle via `wasm-bindgen-rayon`, which spawns a pthread-over-Web-Workers pool and parallelizes the tick step.

This path requires a **pinned nightly Rust toolchain** because `-Z build-std` (rebuilding `std` with the wasm32 atomics target-feature enabled) is nightly-only. The pinned date lives in `scripts/_wasm-threaded-toolchain.sh`.

One-time setup:

```sh
rustup toolchain install nightly-2026-04-15 \
    --component rust-src \
    --target wasm32-unknown-unknown
```

Build:

```sh
./build
```

The threaded pass runs automatically when the pinned nightly is installed; without it `./build` falls back to the single-threaded bundle and prints a warning. JS callers must `await initThreadPool(navigator.hardwareConcurrency)` after `await init()` to actually spawn the worker pool. The host page must be `crossOriginIsolated`; `web/coi-serviceworker.js` installs a Service Worker that adds the COOP / COEP response headers via fetch interception, which works on hosts that can't set them natively (GitHub Pages, the Vite dev server before its config is taught about them).

### Node.js

Node.js **20+** is required for the dev server, prototypes, and JavaScript tests. Install it from <https://nodejs.org/> or via a version manager such as [nvm](https://github.com/nvm-sh/nvm) or [fnm](https://github.com/Schniz/fnm).

## Running

```
npm install
./run                # vite dev server at http://localhost:5173/
./run --server       # vite + aenternis-server (native backend) in parallel
npm run dev:p8       # vite, opening prototype 8 directly
```

The Vite dev server is required for the Web Worker mode in prototype 8 (Chrome blocks workers from `file://` null origin). Each prototype is otherwise a self-contained static page and can also be opened directly via `file://` if you don't need workers.

### Backends

The 3D viewer at `web/` runs against either of two interchangeable backends:

- **WASM Web Worker** (default) — `wasm-pack` builds `aenternis-wasm` into a `.wasm` bundle and the viewer runs the simulation inside a Web Worker. Self-contained, deployable to GitHub Pages.
- **Native dev backend** — `cargo run -p aenternis-server` starts a Rust binary that hosts a *shared* `SparseWorld` over WebSocket on `ws://127.0.0.1:8765/sim`. Viewer connects to it when launched with `?backend=native` (or via the "Backend" panel in the side hud). Multi-tab clients see the same world; reset/pause from any tab applies globally. Faster ticks (rayon, native LLVM) and quicker rebuild loop than WASM, but dev-only — no auth, no deployment story.

See [docs/native-server.md](docs/native-server.md) for the full dev workflow.

## Tests

### JavaScript

Production code under `src/` is covered by [Vitest](https://vitest.dev/) and mutation-tested by [Stryker](https://stryker-mutator.io/). Lab prototypes under `prototypes/**` are intentionally exempt.

```
npm run test          # vitest, single run
npm run test:watch    # vitest, watch mode
npm run test:cov      # vitest with v8 coverage (95% lines / 90% branches)
npm run test:mutation # Stryker mutation testing (break threshold 70 %)
./check               # full gate: TS + Rust + WASM (no mutation, ~2 min)
./check --mutation    # full gate + Stryker + cargo mutants (~30 min)
```

`./check` is what CI runs on every push and pull request (see `.github/workflows/ci.yml`). Reports land in `reports/coverage/` and `reports/mutation/`.

### Rust

The workspace is tested with `cargo test`, plus [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) for coverage and [cargo-mutants](https://mutants.rs/) for mutation testing.

```sh
cargo test --workspace --all-targets       # unit + integration tests
cargo llvm-cov --workspace --html          # coverage → reports/rust-coverage/html/
cargo mutants --workspace                  # mutation → reports/rust-mutation/
```

`cargo fmt`, `cargo clippy`, and `cargo test` run in CI on every push.

**Mutation testing runs locally only** — a full workspace run takes ~13 minutes, too slow to gate every PR on. Run it after meaningful logic changes (new opcodes, new physics phases, RNG tweaks). Current baseline is **0 missed mutants**; any `MISSED` line in your run is a real test-coverage gap.

`.cargo/mutants.toml` lists 18 mutations classified as **equivalent** — their effect is mathematically indistinguishable from native code (boundary cases at probability 0, BTreeMap iteration ordering, no-op assignments at equality). Each entry is documented with reasoning; remove an entry only when new code makes that mutation observable.

Install the Rust test tools once per machine:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov cargo-mutants
```

## Prototypes

Each prototype is a self-contained static page — open via the dev server (or `file://` directly).

- [`prototypes/01-diffusion/`](prototypes/01-diffusion/) — energy diffusion in a 3D torus
- [`prototypes/02-memory/`](prototypes/02-memory/) — proto-entity as energetic memory (concept abandoned, kept as historical record)
- [`prototypes/03-vm/`](prototypes/03-vm/) — minimal virtual machine
- [`prototypes/04-ports/`](prototypes/04-ports/) — directional ports, ignition, energy suction (drain since abandoned)
- [`prototypes/05-pointers/`](prototypes/05-pointers/) — 3D field of micro-entities, pointer-driven emission
- [`prototypes/06-cooperation/`](prototypes/06-cooperation/) — 2D cooperation, full 20-opcode VM, A/B inspector
- [`prototypes/07-perf-3d/`](prototypes/07-perf-3d/) — 3D performance test
- [`prototypes/08-viewer-3d/`](prototypes/08-viewer-3d/) — 3D viewer (Three.js, instanced rendering, optional Web Worker)
- [`prototypes/09-sparse-world/`](prototypes/09-sparse-world/) — sparse 2D world, big bang as initial condition, `cells ≤ energy`
- [`prototypes/09b-sparse-world-3d/`](prototypes/09b-sparse-world-3d/) — the same sparse world in 3D (`DIRS = 4 → 6`), proving the mechanics are dimension-agnostic
- [`prototypes/10-render-tuner/`](prototypes/10-render-tuner/) — tournament-style chooser for the viewer's render parameters (visual tool, not physics)
- [`prototypes/11-gravity/`](prototypes/11-gravity/) — gravity + pressure on diffusion (reference validation for the core mechanic)

The README in each prototype folder is in Czech — they are kept as historical lab notes.

## License

MIT — see [LICENSE](LICENSE).

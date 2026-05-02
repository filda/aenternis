# Aenternis

<!-- CI badge: replace OWNER/REPO with the GitHub path once the repo is published.
     Workflow file lives at .github/workflows/ci.yml and the badge resolves automatically:
     ![CI](https://github.com/OWNER/REPO/actions/workflows/ci.yml/badge.svg) -->

A 3D toroidal simulation where every cell is a latent micro-computer with its own program, energy, and memory pointers. Higher-level phenomena — entities, organisms, movement, reproduction, combat, communication — emerge from the physics of energy flow and programmable content.

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

Prototype phase. Eight laboratory web prototypes exist in `prototypes/`, each verifying a specific layer of physics or programmer interface. The VM has 20 opcodes in its latest 2D variant. The dominance / intrusion mechanic, the UI lineage tracker, and the additional sensors are designed but not yet implemented — see [docs/plan.md](docs/plan.md).

The eventual production target is Rust + WASM. Today's prototypes are intentionally low-friction JavaScript so design questions can be answered cheaply.

## Running

```
npm install
npm run dev          # http://localhost:5173/ — landing page with prototype index
npm run dev:p8       # opens prototype 8 directly
```

The Vite dev server is required for the Web Worker mode in prototype 8 (Chrome blocks workers from `file://` null origin). Each prototype is otherwise a self-contained static page and can also be opened directly via `file://` if you don't need workers.

## Tests

Production code under `src/` is covered by [Vitest](https://vitest.dev/) and mutation-tested by [Stryker](https://stryker-mutator.io/). Lab prototypes under `prototypes/**` are intentionally exempt.

```
npm run test          # vitest, single run
npm run test:watch    # vitest, watch mode
npm run test:cov      # vitest with v8 coverage (95% lines / 90% branches)
npm run test:mutation # Stryker mutation testing (break threshold 70 %)
npm run check         # test:cov && test:mutation — the verification gate
```

`npm run check` is what CI runs on every push and pull request (see `.github/workflows/ci.yml`). Reports land in `reports/coverage/` and `reports/mutation/`.

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

The README in each prototype folder is in Czech — they are kept as historical lab notes.

## License

MIT — see [LICENSE](LICENSE).

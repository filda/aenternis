# Aenternis — plan and implementation status

Last updated: 2026-05-03 (Rust core + VM done; toroid harness deferred; WASM bindings next)

This document summarizes where we are and what comes next. Decisions about mechanics live in `mechanics.md`, questions and agreements in `questions.md`, prototypes in `prototypes.md`.

## Current status

### Done in implementation

- 9 laboratory prototypes (`prototypes/01-diffusion` through `09-sparse-world`), each verifying a specific layer of physics
- The 2D variant in prototype 6 as a platform for cooperation and collision experiments
- The 3D variant in prototype 5 as a baseline for emergent reproduction
- A 3D performance test (prototype 7) and a 3D viewer (prototype 8, including a Web Worker mode)
- Sparse world (prototype 9, 2D): big bang from a single cell, `world.size() ≤ E_total` invariant, alloc-on-write, GC of `E = 0` cells, tick-based RNG. Bit-equivalent to a port of prototype 6's toroid for 1000+ ticks while it stays inside the toroid window.
- **Rust core skeleton** (`crates/aenternis-core/`): `Coord`, `Direction`, `Rng` (PCG-XSH-RR-64/32 with splittable per-cell-per-tick streams), `Cell` (memory + 6 pointers + 6 rates + active outflow + override flags + PC + UI tags), `proportional_clamp`, `SparseWorld` (`BTreeMap<Coord, Cell>` for deterministic iteration, big bang, GC, neighbor lookup). 92 unit / integration tests, all green; no tick logic yet.
- Slot model (32-bit unsigned integer, opcode = low byte)
- VM with 20 opcodes (nop, set, copy, add, sub, inc, dec, jmp, jz, setp, getp, port, senergy, jne, je, ldi, sti, setpv, sid, paint)
- Passive emission with pointer layout from the end of memory
- Active `port` — active outflow on top of passive
- Sub-tick reflow: pointer layout reacts to combined_rate within the current tick
- Programmer override of pointers (ephemeral, per tick)
- Stochastic floor for flow (no freezing), proportional clamping (no checkerboard) — Largest-Remainder apportionment with Fisher-Yates tie-break, statistically isotropic across `Direction::ALL` (2026-05-13)
- Per-cell tickBudget for CPU stepping in the inspector
- K = 1 as the default (compute = energy conserved)
- Dual A/B inspector + communication trace A ⇄ B

### Done in design (decided, awaiting implementation)

- **Dominance / intrusion mechanic** (collision as soft mixing)
- **Identity / lineage tracker** in the UI (Hamming-distance match)
- **HSV visualization** combining appearance hue + energy brightness
- **Sensors `sinflow`, `sself`, `srate`** — implementation debt

### Open (needs further discussion or experiment)

See `questions.md`. Notably:

- Calibration of `move_threshold` in the dominance formula
- Order of inflow application across multiple directions with high dominance
- Multi-hop sense (whether at all)
- Performance refactor for larger worlds
- Rust + WASM as the production platform

## Implementation roadmap (Rust + WASM)

The skeleton crate is in place; from here the work proceeds in narrow phases that each end on a passing `./check` (= TS typecheck + vitest + `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` + WASM build).

### Phase 0 — Skeleton ✓

Workspace, lints, CI job, helper script for the verification gate, and four foundation modules: `Coord`/`Direction`, `Rng`, `Cell` + `proportional_clamp`, `SparseWorld`. **Done 2026-05-03.**

### Phase 1 — Diffusion-only tick ✓

End-to-end tick cycle without the CPU phase. `tick::compute_natural_rates`, `tick::lay_out_pointers`, `tick::collect_outflow`, `tick::apply_outflow`, `tick::end_of_tick`, plus the `tick::initialize` helper and the orchestrating `tick::step_diffusion(world, coeff)`. `SparseWorld::get_or_alloc` provides the alloc-on-write entry point used by inflow into void neighbors.

Verified by 132 tests, including:

- **conservation** of `world.total_energy()` across 50 ticks of `step_diffusion`
- `world.len() ≤ total_energy` invariant maintained across 20 ticks
- **determinism** — two independently-seeded simulations produce byte-identical state for every cell after 30 ticks
- big bang from origin expands outward (orthogonal neighbors are alloc-on-written in the first tick)
- empty cells produced during outflow are removed by `gc_empty` before the next tick starts

The behavioral semantics match prototype 9: the only pieces still missing for parity are the CPU phase, `active_outflow`-driven reflow, and dominance / intrusion. **Done 2026-05-03.**

### Phase 2 — VM execution ✓

20-opcode VM wired in as the CPU phase. Per-cell `floor(energy / k)` instructions executed against a snapshot of neighbor energies (introspection invariant enforced by type signature — `execute_instruction` has no access to the world). Programs may override pointers via `setp`/`setpv` and accumulate active outflow via `port`; the sub-tick reflow re-runs `lay_out_pointers` with the resulting combined rates honoring the override flags.

Verified by 57 additional tests (189 total), including:

- **conservation** holds with VM running over 30 ticks (random program from big-bang noise) — no opcode leaks energy
- **determinism** with VM running — two seeded simulations produce byte-identical state after 20 ticks
- `step(world, coeff, u32::MAX)` ≡ `step_diffusion(world, coeff)` (budget = 0, no instructions)
- per-opcode behavior matches `docs/vm.md` (set, copy, add, sub, inc, dec, jmp, jz, jne, je, setp, getp, port, senergy, ldi, sti, setpv, sid, paint, plus nop and unknown-opcode default)
- modular addressing, PC wrap, direction-modulo for `d` operand
- two adjacent cells running `senergy` each see the other's energy correctly through the per-cell snapshot

`tick::step(world, coeff, k)` is now the production tick. `tick::step_diffusion` remains for tests that want a CPU-less baseline. **Done 2026-05-03.**

### Phase 3 — WASM bindings ✓

`crates/aenternis-wasm/` cdylib wraps the core `SparseWorld` as an owned-handle JS API via `wasm-bindgen`. Build via `wasm-pack build crates/aenternis-wasm --target web` (now also a step in `./check`, so the verification gate enforces a working WASM bundle).

API surface:

- `new(seed, energy)`, `step(coeff, k)` — constructor + tick
- `total_energy()`, `cell_count()`, `tick()` — scalar stats
- `cells_snapshot()` — flat `Uint32Array`, 6 fields per cell `(x, y, z, energy, origin_tag, appearance)`, deterministic canonical iteration order
- `snapshot_stride` getter (= 6) so JS doesn't hard-code the layout

Animated smoke-test page lives at the repo root (`index.html` + `web/main.js`): `requestAnimationFrame` loop, 2D xy projection on a canvas, heat-ramp coloring of cells, HUD with tick / cell count / total energy / FPS, and Pause / Reset / config controls. Run via `npm run dev` (which opens `/` in the browser; the WASM bundle must already be built — `./check` and `./build` both rebuild it as a side effect).

**Done 2026-05-03.**

### Phase 4 — Frontend integration ✓ (4a–4d)

Production frontend in `web/` (`index.html` + `main.js` + `worker.js`):

- **4a — Three.js render swap.** InstancedMesh of low-poly spheres, dynamic capacity (powers of two), heat ramp from energy, OrbitControls for camera. Auto-fit to world bbox on first frame after Reset.
- **4b — WSAD camera + tracker + trail.** FPS-style WSAD/Q/E/Shift movement layered on top of OrbitControls (speed scales with target distance). Max-energy cell highlight (pulsing wireframe box) plus fading trail of last N positions.
- **4c — Web Worker.** Sim moves to a dedicated worker thread; main thread only renders. Snapshots flow back as transferable `Uint32Array`. HUD shows FPS (render thread), ms/tick (Rust step duration), and ticks/s (overall throughput). Decoupled — render holds 60 FPS even when sim is heavy.
- **4d — Inspector panel.** `World::cell_inspect(x, y, z)` exposes a flat 28-prefix `Uint32Array` (pc, energy, origin_tag, appearance, four 6-arrays for pointers / rates / active_outflow / inflow) plus the variable-length memory tail. Three.js raycaster on click → `inspect` worker message → side panel rendering with live auto-refresh every 5 frames while the world runs.

**Done 2026-05-04.**

### Phase 5 — Dominance / intrusion ✓

Collision-as-soft-mixing implemented in `tick::apply_outflow`. Three-phase pipeline replaces the old append-at-end logic:

1. Snapshot pre-step energies + per-source total outflow.
2. Shrink each source by its total outflow.
3. Per target, build inflow entries with `dominance = clamp(1 - r/move_threshold, 0, 1)` where `r = target_E_post / max(attacker_E_post_burn, 1)`. Sort by dominance descending (tie-break by canonical direction). Insert each at `write_start = memSize - intrusion_depth`. Origin-tag inheritance fires when top dominance ≥ 0.5.

PC stays numerically the same across the insert (body-snatch vs. continuity per `pc_old < write_start`, exactly as `docs/mechanics.md` point 15.3 specifies).

`SparseWorld::move_threshold: f32` (default 2.0, public field) tunes how easily strong attackers take over. 9 new tests cover the dominance arithmetic, intrusion-depth math, sort order with multiple inflows, origin-tag inheritance threshold, and conservation. **Done 2026-05-04.**

**Visual observable:** before phase 5 the WASM 3D viewer rendered a uniform "potato"; after phase 5 it shows the same wisps and local concentrations as JS prototype 9. The fix wasn't iteration order, it was a missing physical mechanic.

### Phase 6 — Sensors ✓

`Sinflow` (0x14), `Sself` (0x15), `Srate` (0x16) opcodes wired through `Cell::inflow`, `vm::execute_instruction`, and the inflow-tracking pass in `tick::apply_outflow`. Programs can now observe (a) how many slots arrived from each face in the last tick, (b) their own current energy / memSize, (c) their own combined per-direction rate. Total opcode count: 23 of 256 (= 9 % density). **Done 2026-05-04.**

### Phase 7 — Program injection ✓

`SparseWorld::big_bang_with_program(seed, energy, &[u32])` writes a programmer-supplied prefix into the origin cell's memory; the rest of the slots come from the deterministic RNG. RNG is **not** advanced for program-covered slots, so the suffix from `(seed, energy)` is identical regardless of the program supplied — matches prototype 9's `bigBang(eTotal, programSlots)`.

WASM exposes this as `World.newWithProgram(seed, energy, Uint32Array)`. The frontend's config panel has a textarea (`Program v centrální buňce`) accepting one slot per line, decimal or `0x`-hex, with `;`-comments. On Reset the parser produces a `Uint32Array` and posts it to the worker.

This closes the prototype-9 parity gap on the initial-state semantics: pure RNG noise (random emergence) vs deterministic seeded program (intentional experiments) is now a textarea-level choice rather than a code-level one. Mnemonic assembler (parsing `set 5, 42` etc. into slots) is still pending — see "Later". **Done 2026-05-04.**

### Later
- **Mnemonic assembler / disassembler** — phase 7 lands raw u32 program injection; the next step is parsing `set 5, 42` / `port 0, 10` / `jmp 0` mnemonics into slots, plus rendering the inspector's memory dump as a disassembly with PC marker. Pairs naturally with preset programs (`burner`, `repli`, `orbiter`) that prototype 9 had.
- **Lineage tracker** + manual tag + war paint as a UI overlay.
- **Z80-density opcodes**: bitwise, arithmetic, conditional jumps, stack. Goal: ~60 % meaningful opcodes for emergent appearance from random noise.
- **Persistence**: `bincode` save / load with an explicit version byte in the header.
- **Aging counter** as a debug metric (open: per-slot vs aggregated, see `questions.md`).
- **Reflection mechanism** if cap-exceeding inflows turn out to be a problem in 3D.
- **Performance**: rayon `par_iter` over cells, eventually `SharedArrayBuffer` + WASM threads for off-main parallelism. Only after profiling shows it's needed.
- **Optional: toroid reference + bit-identity harness in Rust**. Originally listed as the first production milestone (a port of prototype 5's toroid as a side-by-side baseline for sparse). Demoted to optional because (a) prototype 9 already verified the sparse-vs-toroid equivalence in 2D JS, (b) the 2D-to-3D step is a config change (`DIRS = 4 → 6`) with no logic change, and (c) the Rust sparse engine already has strong invariant coverage (conservation, determinism, world-size bound, per-opcode behavior). Revisit only if a concrete bug surfaces that this harness would have caught.

### Out of scope (for now)

- **JS-side performance refactor.** With sparse 3D moving directly to Rust + WASM, the JS toroid prototypes (5–8) stay as historical lab experiments at their current ceiling (N ≈ 32–48 smooth, 64 usable offline).
- **Prototype 10 (self-encapsulation, 2D sparse)** — paused; the same physical question can be studied directly in the 3D Rust + WASM core once dominance lands. If Rust dominance proves hard to debug, a JS prototype 10 may come back as a sandbox.

## Milestone history

- **2026-04-28**: project established, first design document drafted (`aenternis.md`)
- **2026-04-29**: prototypes 1-4 (diffusion, memory, VM, ports)
- **2026-04-30**: prototype 5 (3D field of micro-entities, slot model, pointers), prototype 6 (2D cooperation), 18 opcodes
- **2026-05-01**: consolidation discussion. Documentation refactored — split into `aenternis.md` (core), `mechanics.md` (detail), `questions.md` (questions), `vm.md` (spec), `prototypes.md` (laboratories).
- **2026-05-02**: prototype 7 (3D performance test) and prototype 8 (3D viewer with WSAD camera, instanced rendering, Web Worker mode); `sid` and `paint` opcodes implemented (VM at 20 opcodes).
- **2026-05-03**: prototype 9 (sparse world, 2D) — `Map<bigint, Cell>` replacing the toroidal grid, big bang as initial condition, alloc-on-write + GC, tick-based RNG. Headless conservation test + bit-identity comparison harness against a port of prototype 6.
- **2026-05-03 (later)**: Rust + WASM implementation kicked off. Cargo workspace (`crates/aenternis-core/`), CI job (`cargo fmt --check + clippy -D warnings + test`), `scripts/check.sh` helper for the local sandbox loop. Phase 0 skeleton landed: `Coord` + `Direction`, deterministic PCG `Rng` with splittable per-cell-per-tick streams, `Cell` with full pointer/rate/override surface + `proportional_clamp`, `SparseWorld` over `BTreeMap<Coord, Cell>` with big bang and `gc_empty`. 92 tests, all green. No tick logic yet — that's phase 1.
- **2026-05-03 (evening)**: Phase 1 (diffusion-only tick) and Phase 2 (full VM execution) landed in a single sandbox-iteration loop. 189 tests, all green. The Rust core now matches prototype 9's physics in 3D + a working 20-opcode CPU phase. Energy conservation verified over 30 ticks of `step` with the VM actively running random noise as a program. Production sparse 3D engine at functional parity with the JS prototypes (modulo dominance / lineage tracker / sensor expansion, all on the post-bit-identity backlog).
- **2026-05-13**: directional-bias fix in `combined_clamped` — the proportional-clamp leftover used to land entirely in `Direction::ALL[0]` (`Xp`), producing a visible asymmetry in long-running 3D viewer runs (lalok toward `+X` after ~3000 ticks). Replaced with **Largest-Remainder apportionment + Fisher-Yates tie-break** (centralized in `crates/aenternis-core/src/apportion.rs`); the leftover now distributes uniformly across faces, deterministic in `(world_seed, tick, coord, domain)`. Five contract tests (`tests/tick_combined_clamped_contracts.rs`) lock in conservation, non-exceedance, determinism, the `total ≤ cap` fast path, and statistical isotropy under symmetric input. Same algorithm is shared with `proportional_clamp` in `cell.rs`. Bit-parity vs JS laboratory prototype 9-B is consciously released by this change — the Rust core is now its own stream-stability anchor (`tests/rng.rs` is the frozen reference); the historical `dump_state_for_diff.rs` harness stays as `#[ignore]` for forensic value but is no longer expected to match the JS dump.
- **2026-05-15**: multi-threaded WASM via **`wasm-bindgen-rayon`**. The `wasm-threads` cargo feature on `aenternis-wasm` (forwarded from `aenternis-core`) pulls in rayon's pthread-over-Web-Workers bridge; the existing `par_or_seq_iter_mut!` macro now dispatches to the rayon parallel branch on wasm32 too, behind that feature flag. Build requires a pinned nightly Rust toolchain (`-Z build-std` is nightly-only, needed to rebuild `std` with atomics-enabled wasm32 features) — wrapped in `scripts/build-wasm.sh` with all required RUSTFLAGS (`+atomics,+bulk-memory`, `--shared-memory`, `--max-memory=1073741824`, `--import-memory`, plus four `__wasm_init_tls`-family symbol exports). `scripts/check.sh` auto-dispatches to the threaded build when the toolchain is installed, so the gate's `pkg/` matches what `web/worker.ts` initializes at runtime. Host page must be `crossOriginIsolated`: Vite dev sets COOP/COEP headers in `vite.config.ts`; GitHub Pages production relies on `web/coi-serviceworker.js` (vendored, MIT) which installs a Service Worker that rewrites every fetch response with the headers. JS-side runtime detection in `worker.ts`: if `crossOriginIsolated` and the WASM bundle exports `initThreadPool`, calls it with `navigator.hardwareConcurrency`; otherwise falls back to single-threaded WASM. Measured speedup on a 12-core dev machine at 1 M energy: tick step time drops ~2× across the realistic cell-count range (124 ms → 66 ms at 100 k cells, 569 ms → 269 ms at 240 k cells). Sub-linear vs core count is Amdahl-bounded — `cellsSnapshot` serialization, parts of `apply_outflow` (phases 1+2 are sequential), Web Worker synchronization overhead, and SharedArrayBuffer memory bandwidth all contribute a roughly 50 % sequential fraction. Bit-parity multi-thread test (`bit_parity_rayon_parallel_path` in `tests/apply_outflow_bit_parity.rs`) locks in deterministic output through rayon work-stealing on a 22³ = 10 648-cell dense grid that immediately exceeds the `PAR_THRESHOLD` (8 192), so any future change that introduces a race across cells trips the test.

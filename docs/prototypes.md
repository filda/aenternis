# Aenternis — prototypes

Last updated: 2026-05-03 (prototype 9 implemented; 3D sparse prototype dropped)

A series of laboratory web prototypes lives in `prototypes/`. Each prototype is meant to verify a specific layer of physics or programmer interface. They are not games; they are experiments.

Common traits of all prototypes:

- a static HTML / CSS / JS application, openable in a browser without any build step
- visualization of the field + simulation controls + parameters
- an inspector for a detailed view of specific cells

The prototype READMEs themselves remain in Czech as historical lab notes.

## Prototype 1: energy diffusion in a 3D torus

`prototypes/01-diffusion/` — 2026-04-29

A 3D toroidal grid, a single starting energy, diffusion across 6 orthogonal directions. Verified that local energy flow driven by potential differences produces stable dispersion.

**Question:** does energy behave stably, legibly, and physically credibly when it flows by potential difference? **Answered: yes.**

## Prototype 2: a proto-entity as energetic memory

`prototypes/02-memory/` — 2026-04-29

A single entity with energy + memory. Aging of memory cells. Probabilistic mutation by age. Tested the rule `$a = $a` as a memory refresh.

**Question:** is the rule "memory ages, writing refreshes it" usable as a basis for stability and mutation?

**Current status: concept abandoned** (consolidation 2026-04-30). Diffusion as mixing supplies mutation more naturally, and aging conflicted with the membrane model. The prototype remains as a historical record of the design.

## Prototype 3: a minimal virtual machine

`prototypes/03-vm/` — 2026-04-29

8-bit address space, Program Counter, stack, shared memory for program and data. Basic instructions. Tested the VM as the basis of a programmable entity.

**Question:** is the VM simple enough to be understandable and at the same time rich enough that interesting failures emerge from it?

**Status:** the VM concept evolved in later prototypes. 8-bit bytes shifted to 32-bit slots (with the opcode in the low byte), and the instruction set keeps growing.

## Prototype 4: ports, rocket movement, and energy suction

`prototypes/04-ports/` — 2026-04-29

3D world with diffusion + a single manually controlled entity. Six directional ports. Writing to a port = strong ignition + movement impulse + emission of energy. Active suction of energy from the surroundings.

**Question:** can the entity be controlled like a rocket without the movement feeling like an artificial instruction?

**Status:** the "ignition = energy + impulse + movement" concept was split apart in later prototypes. Movement shifted to metempsychosis (continuity of program, not object). Energy suction / drain was abandoned (consolidation 2026-05-01, point 4). Active port writes survive only as "addition to outflow", without the magical impulse.

## Prototype 5: a field of micro-entities and diffusion as mixing (3D)

`prototypes/05-pointers/` — 2026-04-30

A 3D toroidal field where **every cell is a latent micro-entity**. Slot model (32-bit). VM with 12 opcodes (`setp` / `getp` / `port` added). Passive emission driven by six directional pointers. Sub-tick reflow with combined_rate. Diffusion as mixing, with membrane and core emerging.

**Question:** can passive emission driven by pointers serve as the sole mechanism of reproduction, infection, and mutation?

**Status: successfully verified.** Observations:

- the two-layer structure (core + membrane) emerges exactly as the theory predicted
- a self-replicating program propagates; a projectile via a strong `port` produces "fire hells" of emergent infection
- above flow coefficient 0.17, the expected checkerboard artifact of explicit 3D diffusion appears
- the programmer must write programs with a tight loop, otherwise the PC drifts off into data

## Prototype 6: cooperation of two entities (2D)

`prototypes/06-cooperation/` — 2026-04-30

A 2D toroidal variant focused on cooperation between two neighboring cells. Same slot model and VM as prototype 5, plus an extension of the instruction set (20 opcodes total) for reactive logic:

- `senergy d, a` — read-only sense of a neighbor's energy
- `jne a, t`, `je a, b, t` — conditional jumps
- `ldi a, b`, `sti a, b`, `setpv d, a` — indirect addressing
- `sid a`, `paint v` — call-sign and war paint (UI layer)
- per-inspector "CPU step" with tickBudget
- communication trace A ⇄ B

**Question:** can cooperation arise purely from program and ports, without the engine knowing the concept of "ally"?

**Status: active experimental platform.** Hand-written cooperative programs have not yet produced stable cooperation — in its current form the VM is too sparse for reactive protocols. The instruction set can be extended on demand (`sinflow`, `sself`, `srate`, bitwise operations, etc.).

## Prototype 7: 3D performance test

`prototypes/07-perf-3d/` — 2026-05-02

A pure performance experiment, no new physics. Measures how far a JS-only simulation of an Aenternis 3D field can go. Built on prototype 5 (3D variant with 12 opcodes, slot model, sub-tick reflow). Includes a "Benchmark 100 ticks" button that measures wall-clock time and computes ms/tick and cells/sec.

**Question:** what N (in N×N×N) does JS still handle smoothly, and how much would Rust + WASM realistically buy us?

**Initial measurements** (from notes during development):

| N    | Cells   | 100-tick run | ms / tick | cells / sec | FPS   |
|------|---------|--------------|-----------|-------------|-------|
| 8    | 512     | 40 ms        | 0.40      | 1.29 M      | 60    |
| 16   | 4 096   | 162 ms       | 1.61      | 2.54 M      | 60    |
| 32   | 32 768  | 2 095 ms     | 20.95     | 1.56 M      | 40    |
| 64   | 262 144 | 16 799 ms    | 167.99    | 1.56 M      | 5     |
| 128  | 2.1 M   | 73 714 ms    | 737.14    | 1.36 M      | 1.3   |

**Status:** confirms that pure JS taps out around N = 32–48 for smooth play, with N = 64 still usable for offline experiments. Rust + WASM is the path to 100³ realtime.

## Prototype 8: 3D viewer

`prototypes/08-viewer-3d/` — 2026-05-02

A purely visualization-focused prototype. Decides whether a 3D view of Aenternis has a future or whether 2D slices remain the practical debug tool.

Built on Three.js, using `InstancedMesh` for N³ voxels, OrbitControls + WSAD FPS-style camera, two visualization modes (energy / origin tag), a tracker that follows the highest-energy cell with a fading trail, and an optional Web Worker mode that runs the simulation on a background thread (with a Blob-URL fallback for `file://`).

**Question:** does a 3D view of the world give a player something a 2D slice cannot, and at what N does it stop being readable / playable?

**Status: experimental platform.** Confirms that instanced rendering + Web Worker is the right architectural direction; the perceptual / UX questions (camera, filters, what's actually playable) remain open.

## Prototype 9: sparse world (2D)

`prototypes/09-sparse-world/` — 2026-05-03

A 2D world without a fixed grid. The world's size is a consequence of total energy: a cell exists iff `E > 0`, and `world.size() ≤ E_total`. The big bang is the literal initial condition — one cell at (0, 0) holds all the energy and the world expands outward. Built on the same VM and physics as prototype 6 (20 opcodes, 4 directions), with `Map<bigint, Cell>` replacing the toroidal `Float32Array`, alloc-on-write semantics for emission into void, garbage collection of `E = 0` cells, and tick-based RNG that makes results independent of cell life-cycle.

Comes with two test harnesses (Node):

- `test-headless.js` — conservation + cap invariants over 200+ ticks for three scenarios
- `test-equivalence.js` — bit-identical match against a port of prototype 6's toroid (fixed-N), valid while sparse stays inside the toroid bbox

**Question:** can we replace the fixed toroidal grid with a sparse representation governed by the invariant `cells ≤ energy`, and preserve all the existing physics without exceptions?

**Status: successfully verified.** Observations:

- big bang and heat death emerge from physics alone — entropy gradient is real, not designed
- sparse and toroidal implementations are bit-identical for 1000+ ticks (pure_noise, counter) while sparse stays inside the toroid's window
- expansion front advances at ≈ 1 cell / tick / side in heat-death-like regimes, slower for compact replicators
- `world.size()` peaks around 97% of `E_total` for noise scenarios, matching the theoretical maximum

**Out of scope (carried forward):** 3D variant, full inspector, lineage tracker, history-trace overlay. See README in the prototype directory for the full lab notes.

## Prototype 9-B: sparse world (3D)

`prototypes/09b-sparse-world-3d/` — 2026-05-03

An exact copy of prototype 9 with a single change: `DIRS = 4 → 6`. The sparse world gains a `z` axis (directions `zp` / `zn`) — no new opcodes, no new rules, six neighbors instead of four. The big bang is one origin cell at `(0,0,0)`; the world's maximum diameter in cells is `O(∛E_total)` (vs `O(√E_total)` in 2D), so the expansion front advances more slowly per axis.

**Question:** do the sparse-world mechanics hold unchanged in 3D, or does an edge case appear that the plane hides?

**Status: verified.** Same conservation + cap invariants and the same toroid bit-identity (against a 6-direction port of the reference toroid; the sparse world itself is not a torus) as the 2D version. Confirms the sparse mechanics are dimension-agnostic, which cleared the 3D port to go straight into the Rust + WASM core rather than through a JS prototype.

## Prototype 10: render tuner

`prototypes/10-render-tuner/` — 2026-06

A tournament-style chooser for the production render block in `web/`. A static world is generated once (WASM, fixed `seed=1234` / 1 M energy / 250 ticks), then a 5×2 grid of previews each shows a different value of one parameter; clicking a tile fixes that value and advances to the next round (exposure → emissive → roughness → bloom strength/threshold/radius → fog → SSAO radius → voxel size → min luma). The final round emits a JSON of the chosen combination to paste into the slider defaults in the root `index.html`.

**Note:** not a physics experiment — a visual-calibration tool for the viewer. (This slot was originally pencilled in for a self-encapsulation prototype; that experiment moved directly into the Rust + WASM core instead, so slot 10 was reused for the render tuner.)

## Prototype 11: gravity and pressure

`prototypes/11-gravity/` — 2026-06

A standalone JS reimplementation of the core (like prototypes 01–09) that adds two competing forces to diffusion: long-range gravity and pressure. Dense grid, energy as `Float64`, everything is flux across faces: `drive(A→B) = coeff·(E_A−E_B) + (Π(E_A)−Π(E_B)) + grav·(M_B−M_A)`, proportionally clamped so flow stays conservative. Gravity uses a long-range `1/r` potential with cutoff `R`; pressure `Π(E) = pressure·eref·(E/eref)^γ` is steep in density and halts collapse. Boundary is switchable: torus (default, strict conservation — the clean choice for studying structure formation) or void (open universe, leaked energy is lost). Noise breaks symmetry so structure can grow from a symmetric start (Jeans instability).

**Question:** does introducing gravity / density lead to the *emergence of structure* (clusters), or does everything just dilute into the void?

**Status: physics verified.** This is the reference validation for the gravity / pressure / density-coupled-mutation mechanic that landed in the Rust core (`mechanics.md`, "Gravity and pressure"). The VM is intentionally *not* added here — it would duplicate `vm.rs` / the CPU phase; emergence is studied in the production core where the VM and `active_outflow` already live.

## Planned extensions / further prototypes

Of the priorities from the consolidation on 2026-05-01, most have since landed in the **Rust + WASM core** rather than in a JS prototype:

- **Dominance / intrusion mechanic** (collision as soft mixing of continuities) — ✅ implemented in the core (roadmap Phase 5).
- **Gravity / pressure / density-coupled mutation** — ✅ implemented in the core; physics first validated in prototype 11.
- **War paint** in the UI — ✅ done (viewer color modes: energy / appearance / lineage, `src/color.ts`). The **lineage tracker** (follow one entity across metempsychosis) and **manual tag** (click-to-tag) are still on the backlog.
- **Sensor expansion** (`sinflow`, `sself`, `srate`) — still on the backlog (the inflow-tracking data exists, the opcodes do not).

**Self-encapsulation** was pencilled in as a 2D-sparse prototype, but the experiment moved directly into the Rust + WASM core (where dominance already lives), so no JS prototype was built — slot 10 was reused for the render tuner. Self-encapsulation in the sparse model is literally "creating your own surrounding world from your own energy".

**No separate 3D sparse JS prototype was needed.** Prototype 9-B confirmed the sparse mechanics are dimension-agnostic — `DIRS = 4 → 6` is a configuration change, not a design change — so the production 3D port went straight into the Rust + WASM core.

Detail in `plan.md` and `questions.md`.

## The tone of experimentation

Every prototype is a narrow laboratory experiment, not a game. Visualization is a technical tool, not the final product. The aim: understand how a specific layer of physics behaves. When a prototype only "simulates something" without giving a concrete answer, it is poorly designed.

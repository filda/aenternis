# Aenternis — prototypes

Last updated: 2026-05-02

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

## Planned extensions / further prototypes

From the consolidation on 2026-05-01, several priorities remain unimplemented:

- **Dominance / intrusion mechanic** in the existing prototype 6 — collision as soft mixing of continuities
- **Lineage tracker + manual tag + war paint** in the prototype 6 UI
- **Sensor expansion** (`sinflow`, `sself`, `srate`)
- **Performance refactor** (shared TypedArray) for larger worlds
- **Real implementation** in Rust + WASM for production-grade stages

Detail in `plan.md` and `questions.md`.

## The tone of experimentation

Every prototype is a narrow laboratory experiment, not a game. Visualization is a technical tool, not the final product. The aim: understand how a specific layer of physics behaves. When a prototype only "simulates something" without giving a concrete answer, it is poorly designed.

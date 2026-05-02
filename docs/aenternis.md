# Aenternis

Aenternis is a long-running personal project sitting between simulation, programming game, and toy. The goal is a 3D toroidal space where every cell is a latent micro-computer with its own program, energy, and memory pointers. Higher-level phenomena (entities, organisms, movement, reproduction, combat, communication) emerge from the physics of energy flow and programmable content.

## The core picture

Aenternis shifts away from the model "entities living in space" toward the model **space is made of micro-entities**. Each cell is a latent micro-CPU with energy, memory, a Program Counter, and directional pointers. The difference between an "empty cell" and an "active entity" is not binary — it's a spectrum of organization, energy, and program coherence.

> An entity is not an object in space, but the continuity of a program in an energetic-informational field.

## Vocabulary

- **cell**: a physical location in the world grid. Has its coordinates and neighbors.
- **slot**: a single memory/energy unit inside a cell (a 32-bit value).
- **micro-entity**: every cell, because it has energy, memory, a PC, and directional pointers. This is not a choice — it's a fundamental property of the space.
- **active entity**: a cell with enough energy and a coherent enough program to do something visible.
- **organism**: a programmatically coordinated cluster of micro-entities maintaining a stable mutual state.
- **program continuity / consciousness**: a recognizable, ongoing program pattern. The identity of "the same program" across world ticks.
- **world tick**: the global simulation step. In one tick a CPU executes `floor(energy/K)` instructions, then diffusion, outflow, inflow, and state updates run.

## Design principles

### Physics, not exceptions

No rule should be unnecessarily artificial. Mechanics rest on the "physics" of the world: energy, emission, memory, gradients, dominance, local interactions. Higher-level instructions may exist as a convenience layer on top of physical primitives, never as exceptions to them.

Concretely:

- movement comes from ignition / emission and changes in gradients, not from a magical teleport
- reproduction comes from passing energy and information along, not from an abstract `spawn child` instruction
- combat comes from manipulating gradients and dominance, not from directly subtracting HP from a neighbor
- mutation comes from mixing content between neighbors, or from local energy losses

### Key invariants

- **Diagonal movement physically does not exist** — corner cells share no 2D interface for flow. Energy flows only across faces.
- **No velocity, no inertia** — every tick re-evaluates everything from the current potentials.
- **A neighbor's interior is untouchable** — cells can affect each other only through their interface (emission in / out), never by direct access to another cell's registers, memory, or PC.
- **Drain does not exist as a primitive** — a cell cannot directly steal energy from a neighbor. It can only modify its own outflow and thereby shift gradients.
- **Core and membrane are not structural** — they emerge from the dynamics of memory, not from explicit address rules.
- **Identity is interpretation, not state** — the engine does not maintain "who is the same entity as before." For UI/debug there is an optional origin tag, which does not influence physics.

## Documents

- **`aenternis.md`** (this document) — core, vocabulary, key invariants
- **[`mechanics.md`](mechanics.md)** — detailed physics (energy, diffusion, pointers, dominance, collision, combat, communication)
- **[`vm.md`](vm.md)** — virtual machine specification, instruction set, slot format
- **[`questions.md`](questions.md)** — open questions and resolved decisions
- **[`prototypes.md`](prototypes.md)** — series of laboratory prototypes and what each one verifies
- **[`plan.md`](plan.md)** — current implementation status and planned extensions

## Project status

Eight laboratory web prototypes live in `prototypes/` (`01-diffusion`, `02-memory`, `03-vm`, `04-ports`, `05-pointers`, `06-cooperation`, `07-perf-3d`, `08-viewer-3d`). Each prototype verifies a specific layer of physics or programmer interface. Details in `prototypes.md`.

The VM currently has 20 opcodes in the latest 2D variant (prototype 6), and 12 in the older 3D variant (prototype 5). Density of meaningful opcodes is ~8 % — sufficient for writing simple cooperative programs, sparse for emergence from pure noise. Instruction-set extensions will come when needed.

The next stages are **dominance + intrusion_depth** (collision as soft mixing of continuities), **lineage tracker / war paint** (a UI layer for the player), and **expansion of sensors** (`sinflow`, `sself`, `srate`).

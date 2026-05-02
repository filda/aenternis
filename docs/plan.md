# Aenternis — plan and implementation status

Last updated: 2026-05-02

This document summarizes where we are and what comes next. Decisions about mechanics live in `mechanics.md`, questions and agreements in `questions.md`, prototypes in `prototypes.md`.

## Current status

### Done in implementation

- 8 laboratory prototypes (`prototypes/01-diffusion` through `08-viewer-3d`), each verifying a specific layer of physics
- The 2D variant in prototype 6 as a platform for cooperation and collision experiments
- The 3D variant in prototype 5 as a baseline for emergent reproduction
- A 3D performance test (prototype 7) and a 3D viewer (prototype 8, including a Web Worker mode)
- Slot model (32-bit unsigned integer, opcode = low byte)
- VM with 20 opcodes (nop, set, copy, add, sub, inc, dec, jmp, jz, setp, getp, port, senergy, jne, je, ldi, sti, setpv, sid, paint)
- Passive emission with pointer layout from the end of memory
- Active `port` — active outflow on top of passive
- Sub-tick reflow: pointer layout reacts to combined_rate within the current tick
- Programmer override of pointers (ephemeral, per tick)
- Stochastic floor for flow (no freezing), proportional clamping (no checkerboard)
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

## Nearest work (priority)

1. **Dominance / intrusion in prototype 6** — implement `attacker_E_post_burn`, `dominance`, `intrusion_depth`, insertion in place of append. Start with default `move_threshold = 2.0`. PC rule = numerically stable (body snatch or continuity, depending on `pc_old < write_start`).
2. **Lineage tracker + manual tag + war paint in the inspector** — origin_tag field in the UI, visualization mode (the opcodes `sid` and `paint` are already implemented).
3. **Tier 1 debug metrics** — extend the inspector with `energy_before/after`, `natural_rate[d]`, `active_rate[d]`, `combined_rate[d]`, `inflow[d]`.
4. **Sensors** — implement `sinflow`, `sself`, `srate`.
5. **Cooperation experiments** — once the above is in place, write actual cooperative programs and observe what patterns arise.

## Later phases

- **Performance refactor**: shared `Uint32Array` for the whole world, allocation recycling, optionally a web worker. Goal: 64×64 running smoothly, 100×100 with a smile.
- **Instruction-set expansion to Z80 density**: bitwise operations, arithmetic, conditional jumps, stack. Goal: ~60 % meaningful opcodes.
- **Real implementation in Rust + WASM**: once the physics is verified enough and the VM stable. Today this is still the prototype phase.
- **Prototype 9 (self-encapsulation)** — once dominance is settled, this will be a concrete laboratory experiment with self-encapsulating programs.

## Milestone history

- **2026-04-28**: project established, first design document drafted (`aenternis.md`)
- **2026-04-29**: prototypes 1-4 (diffusion, memory, VM, ports)
- **2026-04-30**: prototype 5 (3D field of micro-entities, slot model, pointers), prototype 6 (2D cooperation), 18 opcodes
- **2026-05-01**: consolidation discussion. Documentation refactored — split into `aenternis.md` (core), `mechanics.md` (detail), `questions.md` (questions), `vm.md` (spec), `prototypes.md` (laboratories).
- **2026-05-02**: prototype 7 (3D performance test) and prototype 8 (3D viewer with WSAD camera, instanced rendering, Web Worker mode); `sid` and `paint` opcodes implemented (VM at 20 opcodes).

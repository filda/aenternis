# Aenternis — open questions and resolved decisions

Last updated: 2026-06-11 (synced to actual engine state — dominance implemented, sensor opcodes still pending)

This document collects questions about the world's mechanics — both active (waiting for a decision or experiment) and resolved (with the decision and a pointer to where it landed).

## Active open questions

### Calibration of dominance / intrusion (point 8)

The collision-as-soft-mixing-of-continuities mechanic (`mechanics.md`, "Collision as soft mixing of continuities") is **implemented** in the Rust core (`tick::apply_outflow`, roadmap Phase 5, 2026-05-04). What remains open is tuning, not implementation:

- Calibration of `move_threshold` (default 2.0, to be tuned experimentally)
- The `intrusion_depth(dominance)` function — currently linear `dominance * memSize`, possibly exponential
- Order of inflow application when multiple directions have differing dominance (proposal: sort, highest dominance first, into the lowest addresses)
- What happens to the target's pointers under metempsychosis (the original pointers are part of the cell's state, not its memory — do they stay?)

### Identity and tracking

- Implementing the lineage tracker in the UI (Hamming-distance match on low addresses, "follow this entity") — still open
- Manual cell tagging in the UI with visualization — still open
- Rule for dominance-propagation of the tag — **resolved & implemented**: the target inherits the attacker's `origin_tag` when top dominance ≥ 0.5 (Phase 5)
- Opcodes `sid` (read self-tag) and `paint` (set appearance) — **implemented** (0x12 / 0x13, 2026-05-02)

### Sensors and communication

- Implement `sinflow d a`, `sself a`, `srate d a` (point 12)
- Optional later: `speek d o a`, `shash d a`, `sdelta d a`
- Multi-hop sense (sight beyond distance 1) — not introduced; open question whether to add at all

### Performance and scaling

- Per-cell `Uint32Array` allocation creates GC pressure. Refactor to a shared TypedArray for the whole world?
- 64×64 = 4 K cells in 2D is fine; 1000×1000 = 1 M is problematic
- Eventual move to Rust + WASM for the real implementation

### Planned extensions for the real implementation

- **Aging as a metric** (not mutation): per-slot 32-bit counter, +1 per tick, reset on write. Used for debug ("where the stable core sits = high age", "what is being overwritten right now = low age"). Does not influence physics.
- **Reflection mechanism**: if inflow exceeds MAX_MEMORY, the surplus returns to the source as additional outflow. Preserves strict conservation.
- **Persistence**: save / load of the whole world. At 100³ that's ~2 MB dump, acceptable. Backwards compatibility not yet addressed.

### Initialization and emergence

- How to detect a viable loop automatically (movement, survival for some time, reproduction, combinations of the above)?

### Resolved (2026-05-02) on initialization

- **Big bang**: the initial entity at the center holds all the world's energy. The program is generated from a deterministic seed (random noise + an optional fixed header). Alternatively a "god program" — a starter program inserted by the player.
- **Energy conservation strict**: world_total = N³ always. A cap-based leakage must, in production, be replaced by the reflection mechanism.
- **Player UX**: in production the player gets one "free entity" they control. All other cells remain intact unless the player takes them over via dominance / metempsychosis. Debug mode lives only in development.
- **Determinism**: a seeded RNG (xorshift / PCG). Important for reproducibility.

### Opcode density and emergence

The current density of meaningful opcodes is ~8 % (20 / 256 in prototype 6). For emergent appearance from random noise, ~60 % (Z80 level) would be desirable. Planned instruction-set extensions: bitwise operations (and / or / xor / shl / shr / rol / ror), arithmetic (mul / div / mod), conditional jumps (jp / jn / jg / jl), stack (push / pop / call / ret), additional addressing modes.

## Resolved decisions

### From the consolidation on 2026-05-01

| Point | Question | Decision |
|-------|----------|----------|
| 1 | World ontology | A cell = a micro-entity. An "entity" = the continuity of a program, not an object. Vocabulary in `aenternis.md`. |
| 2 | Movement | Movement = metempsychosis of code. A face phenomenon between neighbors, not the translation of an object. `mechanics.md`. |
| 3 | Reproduction | Reproduction = a side effect of emission, not an instruction. Same physics as movement, different intent. `mechanics.md`. |
| 4 | Drain | Abandoned as a primitive. A cell cannot directly steal energy. `mechanics.md`, "Combat and conflict". |
| 5 | Ignition | An active contribution to outflow, not a special movement force. `combined_rate = natural + active`. `mechanics.md`. |
| 6 | Pointer layout | Layout reacts to combined_rate within the current tick (sub-tick reflow). Implemented in prototypes 5 and 6. |
| 7 | Ignition strength | Programmable, with real risk. Trade-off: stronger is not automatically better. `mechanics.md`. |
| 8 | Collision | Soft mixing of continuities through dominance / intrusion. **Implemented** (Rust core, Phase 5, 2026-05-04); `move_threshold` calibration still open. |
| 9 | Core / membrane | Emergent layers, not implemented rules. `mechanics.md`. |
| 10 | Diagonals | Physically impossible — no shared 2D interface between corners. `mechanics.md`. |
| 11 | Velocity / inertia | Do not exist. Every tick re-evaluates from scratch. `mechanics.md`. |
| 12 | Sensors | Read-only, only the interface and own state. No introspection of a neighbor. `vm.md`. |
| 13 | Communication / intrigue | Emergent from the rules + programmer creativity, no special instructions. `mechanics.md`. |
| 14 | Projectile | Not an object, but a pattern of traveling dominance. `mechanics.md`. |
| 15.3 | PC under metempsychosis | PC stays numerically the same. Body snatch or continuity preserved depending on PC vs. write_start. |
| 15.5 | Residual state of the source | A weakened version of itself ("exhaust"), no extra mechanism. |

### Before the consolidation (historical decisions)

| Question | Decision | Date |
|----------|----------|------|
| Reproduction via port-write with impulse | Abandoned. Reproduction is passive emission with pointers. | 2026-04-30 |
| Active port write produces a movement impulse | Abandoned. Active write is just a contribution to outflow, no impulse. | 2026-04-30 |
| Aging memory cells + probabilistic mutation | Abandoned. Diffusion itself is mixing = a source of entropy. | 2026-04-30 |
| Energy as 8-bit byte | Moved to 32-bit slot. 8-bit kept only for opcode. | 2026-04-30 |
| Energy = memory directly with K = energy / instruction count | Kept. K = 1 is mathematically distinguished (compute = energy conserved). | 2026-04-30 |
| Diffusion as energy alone, mutation through aging | Shifted: diffusion also carries information, mutation comes from mixing. | 2026-04-30 |
| Pointers as fixed start addresses | Shifted: pointers are laid out by the system from the end, the programmer can override (ephemeral). | 2026-04-30 |
| Movement only into an empty cell (E = 0) | Cancelled — empty cells don't exist. Movement is resolved through dominance / intrusion. | 2026-04-30 and 2026-05-01 |

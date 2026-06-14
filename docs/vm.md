# Aenternis — VM specification

Date: 2026-04-30

This document captures the current specification of the virtual machine (VM) used by every entity in Aenternis. A single source of truth for the instruction set, format, and semantics, separated from prototypes and design documents.

## Basic model

Every cell in the world is a latent micro-CPU. It has:

- **memory** as an array of **slots** (32-bit unsigned integers)
- **six pointers** in the 3D variant / four pointers in the 2D variant, one per direction
- a **Program Counter (PC)** — the index of the current slot
- **rate registers** — number of slots emitted in each direction per tick (managed by the system)
- **active outflow registers** — a temporary buffer for strong ignition from a `port` instruction, reset at the end of the tick

No other registers, no stack (for now). There is no memory-mapped I/O — I/O happens through system emission driven by pointers.

## Slot and opcode

A **slot** = 32-bit unsigned integer (0 to 2^32 - 1).

When a slot is interpreted as an instruction:

- **opcode** = `slot & 0xFF` (lowest byte)
- the upper bits of the slot are ignored during opcode decode — but they remain part of the slot value, so any instruction that reads this slot as data or address sees all 32 bits

Decoding is **total**: every one of the 256 possible low-byte values maps to a real opcode via the fold `opcode = (slot & 0xFF) mod COUNT`, where `COUNT` is the number of defined opcodes (currently 31). Opcodes occupy a contiguous `0x00..COUNT-1` range, so the modulo always lands on a valid instruction — there is no "unknown opcode → nop" case. This is the **opcode-density mechanism** (see "Opcode density" below): random noise always decodes to *something* executable. Bytes `< COUNT` are unchanged by the fold (`b mod COUNT == b`), so assembled programs keep their meaning as new opcodes are appended.

## Instruction set

| Opcode | Mnemonic     | Length  | Semantics |
|--------|--------------|---------|-----------|
| `0x00` | `nop`         | 1 slot  | does nothing |
| `0x01` | `set a v`     | 3 slots | `mem[a % memSize] = v` |
| `0x02` | `copy a b`    | 3 slots | `mem[a % memSize] = mem[b % memSize]` |
| `0x03` | `add a b`     | 3 slots | `mem[a] = (mem[a] + mem[b]) mod 2^32` |
| `0x04` | `sub a b`     | 3 slots | `mem[a] = (mem[a] - mem[b]) mod 2^32` |
| `0x05` | `inc a`       | 2 slots | `mem[a] = (mem[a] + 1) mod 2^32` |
| `0x06` | `dec a`       | 2 slots | `mem[a] = (mem[a] - 1) mod 2^32` |
| `0x07` | `jmp a`       | 2 slots | `PC = a % memSize` |
| `0x08` | `jz a t`      | 3 slots | `if mem[a % memSize] == 0: PC = t % memSize` |
| `0x09` | `setp d v`    | 3 slots | `pointers[d mod DIRS] = v % memSize` (ephemeral override) |
| `0x0A` | `getp d a`    | 3 slots | `mem[a % memSize] = pointers[d mod DIRS]` |
| `0x0B` | `port d i`    | 3 slots | `activeOutflow[d mod DIRS] += i` (strong outflow projectile) |
| `0x0C` | `senergy d a` | 3 slots | `mem[a % memSize] = energy of neighbor in direction d mod DIRS` (read-only sensor) |
| `0x0D` | `jne a t`     | 3 slots | `if mem[a % memSize] != 0: PC = t % memSize` |
| `0x0E` | `je a b t`    | 4 slots | `if mem[a % memSize] == mem[b % memSize]: PC = t % memSize` |
| `0x0F` | `ldi a b`     | 3 slots | `mem[a] = mem[mem[b]]` — load indirect (read from runtime address stored in b) |
| `0x10` | `sti a b`     | 3 slots | `mem[mem[a]] = mem[b]` — store indirect (write to runtime address stored in a) |
| `0x11` | `setpv d a`   | 3 slots | `pointers[d mod DIRS] = mem[a] % memSize` — `setp` with a runtime-computed value |
| `0x12` | `sid a`       | 2 slots | `mem[a] = own origin_tag` — call-sign instruction (UI layer) |
| `0x13` | `paint v`     | 2 slots | `appearance = v` — war paint (UI layer, does not affect physics) |
| `0x14` | `and a b`     | 3 slots | `mem[a] = mem[a] & mem[b]` |
| `0x15` | `or a b`      | 3 slots | `mem[a] = mem[a] \| mem[b]` |
| `0x16` | `xor a b`     | 3 slots | `mem[a] = mem[a] ^ mem[b]` |
| `0x17` | `not a`       | 2 slots | `mem[a] = ~mem[a]` (bitwise complement) |
| `0x18` | `shl a b`     | 3 slots | `mem[a] = mem[a] << (mem[b] mod 32)` |
| `0x19` | `shr a b`     | 3 slots | `mem[a] = mem[a] >> (mem[b] mod 32)` (logical / unsigned) |
| `0x1A` | `mul a b`     | 3 slots | `mem[a] = (mem[a] * mem[b]) mod 2^32` |
| `0x1B` | `div a b`     | 3 slots | `mem[a] = mem[b]==0 ? 0 : mem[a] / mem[b]` (unsigned) |
| `0x1C` | `mod a b`     | 3 slots | `mem[a] = mem[b]==0 ? 0 : mem[a] % mem[b]` (unsigned) |
| `0x1D` | `jp a t`      | 3 slots | `if (mem[a % memSize] as i32) > 0: PC = t % memSize` (signed) |
| `0x1E` | `jn a t`      | 3 slots | `if (mem[a % memSize] as i32) < 0: PC = t % memSize` (signed) |

DIRS = 6 in the 3D model, 4 in the 2D model. Opcodes `0x00`–`0x1E` are implemented in the Rust core (`Opcode::MAX = 0x1E`, `Opcode::COUNT = 31`). The bitwise / arithmetic / shift opcodes all mutate `mem[a]` in place, so they never change `memSize` and energy conservation is automatic. Division and modulo by zero yield `0` (the VM has no trap mechanism); shift amounts are taken `mod 32` (`wrapping_shl`/`wrapping_shr`) so they never panic.

Sensors (`senergy`) are **read-only** — a cell observes the surroundings, but cannot write into a neighbor. Influencing a neighbor happens exclusively via emission (passive radiation + active `port` write). All sensors currently work at distance 1 (immediate neighbor). Possible multi-hop sense (seeing further) is an open question for a later iteration.

### Introspection invariant

No sensor may allow reading a neighbor's internal state. **Forever forbidden:**

- reading another cell's PC
- reading another cell's pointers
- reading another cell's memory (direct access into a neighbor's memory)
- reading another cell's last instruction, pending instruction, registers, append/inflow buffer
- reading another cell's intent (anything internal)

**Allowed:**

- own state of the cell (energy, memSize, own pointers and rates)
- a neighbor's effects on the shared face (neighbor's energy, how much arrived from there)

> The interior of another entity is not directly observable. Only its manifestations on the interface exist.

This is a fundamental principle of Aenternis and no future instruction may break it. Communication, espionage, and defense can exist only through what a cell leaks out (radiation, appearance), never through direct access to its interior.

## Addressing

- **memory** is a slot array indexed from 0 to `memSize - 1`
- **modular addressing**: any `addr` is applied as `addr % memSize`. No memory-protection violations, no out-of-bounds errors.
- **PC wrap**: when the PC reaches or passes `memSize`, it wraps to 0 modularly
- **memory size = the cell's energy** (1 slot of energy = 1 slot of memory)

## Pointers

Six (or four) directional pointers:

- 3D: `xp`, `xn`, `yp`, `yn`, `zp`, `zn` (indices 0-5)
- 2D: `xp`, `xn`, `yp`, `yn` (indices 0-3)

A pointer is always a valid address in the current memory. There is no `NULL` state.

**Default layout** (computed by the system at the end of every tick):

```
zn_ptr = memSize - rate_zn
zp_ptr = zn_ptr - rate_zp
yn_ptr = zp_ptr - rate_yn
yp_ptr = yn_ptr - rate_yp
xn_ptr = yp_ptr - rate_xn
xp_ptr = xn_ptr - rate_xp = memSize - sum(rates)
```

Each direction therefore gets a sub-region of memory exactly the size that will be emitted next tick.

**Programmer override** (`setp` opcode): applies only to the current tick. At the end of the tick the system recomputes pointers back to the default. Sustained reproduction requires overriding every tick.

**Read-rate trick**: the programmer can derive the per-direction rate from the differences between adjacent pointers:

```
rate_xp = xn_ptr - xp_ptr
rate_xn = yp_ptr - xn_ptr
... and so on
```

This works only when no pointer has been overridden by the program.

## Emission

In every world tick, for every cell, for every direction `d`:

1. the system computes **rate_d** = number of slots to emit, as a function of the potential difference with the neighbor (`stochasticFloor((myE - nE) * coeff)`)
2. **active outflow** adds a bonus to rate_d from `port` instructions executed in this tick's CPU phase
3. the **combined rate** is proportionally clamped if the sum across all directions exceeds the current `memSize`
4. from source memory `rate_d` slots are copied starting at `pointers[d]` (modular) — **a copy, not a removal**
5. those slots are sent to the neighbor in direction `d`
6. source memory shrinks from the end by the total combined rate (energy decreases)

## Receiving

In every tick a cell:

1. receives slots from six (or four) neighbors
2. those slots are appended to the end of its memory in a fixed direction order (xp, xn, yp, yn, zp, zn)
3. memory grows accordingly (energy = memory). The engine imposes no `MAX_MEMORY` cap today; if cap-exceeding inflows ever become a problem, a reflection mechanism (surplus returned to the source) is the planned fix — see `plan.md` and `questions.md`.

This produces the natural two-layer structure: a **stable core** at low addresses (where the program lives) and a **mixing membrane** at high addresses (where content is exchanged with neighbors).

## CPU speed

In every world tick each cell executes:

```
instructions = floor(energy / K)
```

where `K` is a world constant. **K=1** is the mathematically distinguished choice — at that value the world's total compute is a conserved quantity (equal to total energy). For K > 1, diffusion loses compute to rounding.

A cell with zero energy does nothing. A cell with energy < K (for K > 1) is inert.

## Order within a world tick

1. **CPU phase**: each cell executes `floor(energy / K)` instructions. Programs may modify memory, pointers (`setp` / `setpv` — sets an "override" flag for that direction), or enqueue active outflow (`port`).
2. **Layout recomputation for this tick**: for each direction `d` where the programmer did not override the pointer, the pointer is re-positioned from the end of memory using `combined_rate[d] = natural[d] + active[d]`. For programmer overrides the pointer stays where the programmer set it; `combined_rate` only determines the length of the stream from that address.
3. **Outflow**: the combined rate (passive + active) from each cell is copied into its neighbors starting at the pointer for each direction.
4. **Inflow**: slots from neighbors are appended to the end of memory.
5. **Reset active outflow** and **reset override flags**.
6. **Layout recomputation for the next tick**: new natural rates from the new potentials, pointers laid out from the end of memory purely by natural rate (without an active component).

## Planned extensions

Specific agreed-upon instructions waiting for implementation (from the consolidation on 2026-05-01):

### Sensors (read-only, see introspection invariant)

`sinflow`, `sself`, and `srate` are **specified but not yet implemented** — the VM currently stops at `0x13` (`paint`). The inflow-tracking field `sinflow` would read already exists in the engine (`Cell::inflow`); only the three opcodes are missing.

Optional, later:

| Mnemonic       | Semantics |
|----------------|-----------|
| `speek d o a`  | mem[a] = a slot from the last received stream from direction d at offset o |
| `shash d a`    | mem[a] = signature / hash of the last received stream from direction d |
| `sdelta d a`   | mem[a] = change in flow / energy from direction d versus last tick |

### Identity and appearance (implemented 2026-05-02)

`sid` and `paint` are already implemented (see the table above). `origin_tag` and `appearance` are per-cell 32-bit fields. The engine does not consult them when computing rate / dominance; only the UI uses them for lineage tracking and visualization.

### Arithmetic and bitwise operations (for opcode density)

Implemented (2026-06-13): `and`, `or`, `xor`, `not`, `shl`, `shr`, `mul`, `div`, `mod` (see the table above). Still on-demand, not yet added: `neg`, `rol`, `ror` — low marginal value over the set above; the decode fold absorbs them whenever they land.

### Conditional jumps

Implemented (2026-06-13): `jp` (sign-test `> 0`), `jn` (sign-test `< 0`) — both interpret `mem[a]` as `i32`. Still planned: two-operand signed comparisons `jg` / `jl` (greater / less between `mem[a]` and `mem[b]`); for now `sub` + `jn`/`jp` covers the same ground at lower cost (3-slot vs 4-slot instructions).

### Stack

Planned: `push`, `pop`, `call`, `ret`. These need a stack pointer (SP) as another register. Open question: where the stack lives in memory, how many slots it has, and how it behaves under shrinkage.

### Multi-hop sense

Open question whether to add it at all. For now all sensors work at distance 1.

## Opcode density

Density of meaningful opcodes is now **structurally 100 %**: the decode fold `(slot & 0xFF) mod COUNT` maps every byte onto a real opcode, so random noise always executes *something* rather than mostly `nop` (the old `20 / 256 = 7.8 %` regime, where ~92 % of bytes were inert). Crucially this is **decoupled from the opcode count** — adding or removing opcodes changes `COUNT` but density stays 100 %, so there is no pressure to pad the instruction set just to hit a number. Instructions are added on actual need.

**Append-only maintenance rule.** The fold is only safe if opcodes occupy a contiguous `0x00..COUNT-1` range and new opcodes are *only appended* (next free discriminant) — never renumbered or inserted in the middle. An assembled program uses bytes `< COUNT_old`; after appending, `COUNT_new > COUNT_old`, so every such byte is still `< COUNT_new` ⇒ `b mod COUNT_new == b` ⇒ it decodes to the same opcode. Renumbering would break every existing program. (Enforced by comment in `vm.rs` and `asm.ts`.) The mild non-uniformity of the fold (`256 = 8·31 + 8`, so opcodes `0x00`–`0x07` are drawn slightly more often from pure noise) is a known, documented tuning lever, not a defect.

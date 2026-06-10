# Aenternis — mechanics

Detailed description of physics and rules. See `aenternis.md` for the overview, `vm.md` for the virtual machine specification, `questions.md` for open items.

## Energy, slot, and memory

In Aenternis there is a **finite amount of energy** spread across the grid of cells. Each unit of energy corresponds to one **slot** in that cell's memory:

- a slot is a 32-bit unsigned integer (0 to 2^32 - 1)
- a cell's memory size = number of slots = the cell's energy
- when energy decreases, memory shrinks **from the end** (highest addresses)
- when energy grows, new slots are appended at the end

Energy is never created or destroyed — it is conserved across all cells. Diffusion only moves it around.

### What lives in memory

A slot can be interpreted in two ways:

- **as an instruction**, when the Program Counter lands on it. The opcode is the lowest byte of the slot (`slot & 0xFF`); the upper bits are ignored during decode, or used as embedded data depending on the instruction.
- **as data**, when another instruction accesses it. Program and data share the same address space (Von Neumann model). Self-modifying code is the natural default.

Details of the VM are in `vm.md`.

## Diffusion and emission

In every world tick each cell moves energy and information to its neighbors (4 in 2D, 6 orthogonal directions in 3D). The flow is a **local face phenomenon** — energy crosses only the face shared with a neighbor, never an edge or a vertex.

### Rate per direction

For each direction `d` in a given tick:

```
natural_rate[d] = stochasticFloor((E_self - E_neighbor[d]) * coeff)
active_rate[d]  = active outflow from `port` instructions in the CPU phase
combined_rate[d] = natural_rate[d] + active_rate[d]
```

`coeff` is a world constant (typically 0.15-0.30 in 2D for stability). `stochasticFloor` rounds with probability equal to the fractional part — so even small gradients occasionally transmit (no freezing).

If `sum(combined_rate)` exceeds the cell's current memory size, all rates are scaled down proportionally (clamp).

### Pointer layout

Each cell has 4 / 6 directional pointers (`xp_ptr`, `xn_ptr`, ...). The system lays them out from the end of memory downward, by combined_rate:

```
zn_ptr = memSize - rate_zn
zp_ptr = zn_ptr - rate_zp
yn_ptr = zp_ptr - rate_yn
yp_ptr = yn_ptr - rate_yp
xn_ptr = yp_ptr - rate_xn
xp_ptr = xn_ptr - rate_xp = memSize - sum(combined_rate)
```

The pointers partition memory into sub-regions: lowest for xp, highest for zn. Each direction therefore gets a memory segment exactly the size that will be emitted next tick.

**The programmer can override a pointer** via `setp d, v` or `setpv d, a`. The override is ephemeral — it applies only to the current tick. After the tick ends, the layout is recomputed back to the default.

### Sub-tick reflow with combined_rate

After the CPU phase the engine recomputes pointers for non-overridden directions, this time with `combined_rate` (= natural + active ignition from `port`). Stronger ignition therefore automatically pushes the pointer deeper into memory, so the outgoing stream starts from a deeper location.

For directions where the programmer overrode the pointer via `setp` / `setpv`, the layout stays where the programmer set it; combined_rate only determines the length of the stream from that starting address.

### Outflow

For each direction d:

1. the system copies `combined_rate[d]` slots from source memory starting at `pointers[d]` (modular)
2. those slots are appended to the destination
3. the source memory is shrunk from the end by `sum(combined_rate)` total (energy decreases)

**Slots are copied, not removed at the pointer addresses.** Energy / memory always shrinks from the end, regardless of where the copy was taken from. The principle "energy = memory" holds, and information is read-only copyable from the pointer position.

### Inflow and membrane

The destination cell receives slots from all neighbors and **appends them to the end of its memory** in a fixed direction order (xp, xn, yp, yn, [zp, zn]).

This naturally produces two layers:

- a **stable core** at low addresses — long-unmodified content where program and identity live
- a **mixing membrane** at high addresses — the contact zone with neighbors, the space of diffusion, mutation, and infection

**Important: core and membrane are not implemented rules.** The engine does not know constants like "0x00-0x3f = core". The layers emerge purely from the dynamics:

- memory shrinks from the end (high addresses disappear first)
- incoming content is appended at the end (high addresses get overwritten)
- the system reads the default layout starting from the end

The programmer is responsible for placing critical code wisely. The engine will not save it.

## Energy as CPU speed

Energy in Aenternis plays three roles in a single quantity:

- **memory capacity** (how many slots the cell can hold)
- **lifetime** (how fast it dies under starvation)
- **CPU speed** (how many instructions it executes per world tick)

The working relation:

```
instructions_per_tick = floor(energy / K)
```

`K` is a world constant. **K=1 is the mathematically distinguished choice** — at that value the world's total compute is a conserved quantity (= sum of energies). For K > 1, diffusion loses compute to rounding.

A cell with zero energy does nothing. A cell with energy < K is inert (for K > 1).

CPU speed and emission **do not compete**. Both draw on energy, but through different mechanisms: CPU through internal state (energy / K), radiation through the boundary with the surroundings (the potential difference). An interior cell in a dense organism has high energy and a small potential difference with its neighbors at the same time — it ticks fast and emits little.

## Order of events within a world tick

1. **CPU phase**: each cell executes `floor(energy / K)` instructions. Programs may modify memory, pointers (`setp` / `setpv` — sets an override flag), or enqueue active outflow (`port`).
2. **Sub-tick reflow**: for non-overridden directions, the pointer is re-positioned from the end of memory using `combined_rate[d]`.
3. **Outflow**: combined rate from each cell is copied into its neighbors starting at the pointer.
4. **Inflow**: slots from neighbors are appended to the end of memory.
5. **Reset active outflow** and override flags.
6. **Layout for the next tick**: new natural rates computed from the new potentials.

## Producing offspring (reproduction)

Reproduction in Aenternis **is not an event or an instruction**. It is the **default behavior of the entire world**:

> Every cell, in every tick, emits energy with informational content into all of its neighbors. The neighbors absorb it. Every cell is therefore continuously becoming an "offspring" of its neighbors, and every neighbor is in turn an offspring of this cell.

Programmability is just **modification** of this constant flow. Through pointers and active `port` writes the programmer says what specific content is sent in a given direction. Targeted reproduction, infection, projectile, and broadcast are all just choices of which fragment of memory gets routed into the default flow.

### Reproduction vs. movement

They share the same physics, but differ in intensity, intent, and consequence:

- **reproduction** is a permanent side-effect of existence. Every cell radiates constantly and thereby "feeds" its neighbors with its own content. It has no goal.
- **movement** is a decision about dominance. A directed strong flow through `port` that breaks the existing continuity in the neighboring cell and asserts a new one.

Manifestations by flow strength:

- **reproduction**: the neighbor receives a coherent program for long enough to gradually "boot" as a copy of the source
- **infection**: an active neighbor receives foreign content, which mixes with its core
- **projectile**: a short, very strong directed flow, not sustainable in time, but with a large momentary effect
- **consciousness movement**: an emergent consequence of changes in stability, gradients, and dominance across a shared face

The engine does not need to (and does not) distinguish these phenomena. It only implements energy/information flow plus mixing rules. The player / debugger interprets the result.

## Movement as metempsychosis

Movement in Aenternis is not a transfer of mass from one cell to another. Mass and local energy stay put in the field, and there is no truly empty space. Movement is **transfer of dominance of continuity** across a shared face.

> Movement is metempsychosis of code: a dominant program continuity asserts itself in the neighboring cell, while the original cell remains as a trace, exhaust, discarded membrane, or fragment.

Consequences:

- the original cell does not vanish after "movement" — it still exists, holding what was left in it
- the destination cell was not empty; its content was mixed or suppressed
- movement is a local face phenomenon between two neighbors, not a geometric translation
- diagonal movement does not arise directly — continuity can only assert itself across orthogonal faces
- multi-direction ignition is splitting or broadcast, not diagonal motion
- apparent diagonal movement can arise as a sequence of orthogonal steps

### Why diagonal is impossible

It is not an arbitrary rule, it is **the geometry of the shared interface**:

- **face** (orthogonal neighbors): a shared 2D area through which energy and information flow
- **edge** in 3D (= corner neighbor in 2D): no shared 2D interface, just a 1D line or a point
- **vertex** in 3D: even less, just a single point

Energy flows **across an area**, not across an edge or a point. A diagonal cell therefore has no physical channel through which to receive flow from the source.

A metaphor: it's like trying to push from outside into a mouse hole. No matter how hard you push, you won't get through, because **the physical interface for passage doesn't exist**.

### No velocity, no inertia

The engine maintains no velocity vector, no momentum register, no memory of motion from the previous tick.

> Every transfer of continuity must be re-fought in the current tick through local outflow, inflow, gradients, and dominance.

Apparent inertia is generated only indirectly: a program remembers a direction and repeats ignition; exhaust from past ticks changed the energy of the surroundings; gradients and traces affect future flow; a projectile is a traveling front of dominance, not an object with velocity.

## Active port write (ignition)

Active write to a port (`port d, intensity`) **is not a special movement force**. It is purely an active increase of outflow in the given direction. Sub-tick reflow already places combined_rate into the layout (see above).

Consequences of active ignition:

- immediate energy loss (the source drops)
- shrinking of memory from the end
- the outgoing stream length = `combined_rate[d]`
- risk of damaging your own program (a strong ignition tears off your core, too)
- a stronger trace / exhaust / projectile in the destination
- changed gradients (the weakened source affects future flow)
- an indirect chance of transferring continuity

### The ignition strength trade-off

A stronger ignition is not automatically better. Working formulation:

```
burn         = active_rate[d]
E_after      = E_before - sum_of_combined_rates
target_ratio = E_target / max(E_after, 1)
```

A weak ignition is not enough (the target resists). A reasonable ignition helps (continuity transfers). A too-strong ignition destroys the stability the source needs to continue its consciousness.

The programmer therefore gets a real tactical decision — every tick, choose how much of your continuity you risk in order to assert yourself in a neighbor. The engine prescribes no "optimal" ignition.

## Combat and conflict

Combat in Aenternis does not stand on any "harm a neighbor" instruction. The engine offers no such primitive.

> A cell cannot directly steal energy from a neighbor. It can only modify its own outflow, thereby shift the local gradients, and consequently receive whatever energy the world's physics happens to send its way.

Combat shifts from direct damage to:

- **continuity disruption**: a strong ignition into a neighbor can overwrite that neighbor's memory with content that overpowers its program
- **overload**: the target receives in one tick more content than it can stably hold
- **contamination**: emitted content carries instructions that settle in the target at high addresses. Over time they may end up executed — through mutation, energy decline, or an intentional jump
- **pointer hijack**: if the attacker can force values into the victim that the victim interprets as a modification of its own pointers, the victim starts emitting something other than it intended
- **gradient manipulation**: by lowering its own potential (radiating its energy out) a cell creates an "energetic vacuum" into which the world's physics may bring energy from its neighbors

Drain as a standalone instruction **does not exist**. This was a deliberate decision (consolidation 2026-05-01, point 4): a cell cannot directly steal energy, only modify its own outflow.

## Collision as soft mixing of continuities

Collision is not a binary "moved / didn't move." A softer model:

> Collision is the degree of mixing of two continuities.

Per-direction, on inflow receipt:

```
attacker_E_post_burn = E_neighbor - sum_combined_rates_neighbor
r              = E_target / max(attacker_E_post_burn, 1)
dominance      = clamp(1 - r / move_threshold, 0, 1)
intrusion_depth = dominance * memSize_after_outflow
write_start    = max(0, memSize_after_outflow - intrusion_depth)
```

Incoming bytes are **inserted** at position `write_start`; existing content from that position upward shifts further up. This preserves `energy = memSize` and pushes the target's bytes into higher addresses.

Consequences by dominance:

- **dominance ≈ 0**: write_start = memSize, insertion at the very end (= membrane)
- **dominance ≈ 0.5**: write_start in the middle (= jammed continuity)
- **dominance ≈ 1.0**: write_start = 0, full metempsychosis — the attacker's program takes over the lowest addresses

### PC and metempsychosis

The PC stays **numerically** the same. If `pc_old < write_start`, the program continues (the core is protected). If `pc_old >= write_start`, the PC now points into the attacker's memory → execution shifts to the attacker's code = body snatch.

This gives the programmer a concrete strategy: keep the PC deep in the core to survive strong attacks.

### Residual state of the source cell

The source remains as a **weakened version of itself**. Memory shrunk from the end, energy lower, low-address content preserved, PC intact. After an extreme ignition the source is very weak, possibly below the CPU execution threshold, but it still exists. The state is "exhaust" after metempsychosis.

### Order of inflows from multiple directions

When several inflows arrive at once with different dominance:

1. sort inflows by dominance (highest first)
2. highest dominance goes into the lowest addresses (deepest intrusion)
3. weaker inflows stack on top of those
4. the weakest stays at the surface

**Status:** implemented in the Rust core (`tick::apply_outflow`, roadmap Phase 5, 2026-05-04). `move_threshold` is a public field on `SparseWorld`, default `2.0`. Calibration of `move_threshold` and the exact `intrusion_depth(dominance)` curve remain open tuning questions — see `questions.md` and `plan.md`.

## Communication and intrigue

Aenternis has no special instructions for communication, espionage, encryption, or defense. These layers emerge from two rules:

1. **Introspection invariant**: a cell cannot directly read the inside of its neighbors (no foreign PC, memory, or pointer)
2. **Continuous emission**: every cell is constantly sending something out

A natural layer of intrigue follows:

- **Knowing your neighbor** is grounded only in what leaks out. The neighbor sends bytes into you — from those you can deduce its program. But the neighbor decides what it sends.
- **Hiding** = controlling what is emitted. The programmer points a pointer at a neutral region — outside observers see only noise.
- **Deception** = broadcasting a false surface. The programmer points a pointer at a "decoy" memory region.
- **Immunity** = controlling incoming streams. Periodic rewriting of the membrane with known content, critical code in deep core.
- **Pointer hijack**: the attacker forces content on the victim that the victim interprets as a modification of its own pointers.

The engine knows none of these layers. They all follow from the basic rules plus programmer creativity.

## Density, stability, and the initial entity

There likely needs to be a density or stability bound. Not a hard "no more fits" cap, but at least a rule that the higher the local density, the stronger the emission, pressure, instability, or probability of change.

Principle:

- **low density**: energy disperses slowly, structures are weak
- **medium density**: an entity is stable and computationally capable
- **high density**: a lot of memory and possibilities, but emission and instability rise
- **extreme density**: rapid spilling, mutation, fragmentation, forced emission

An attack by adding energy can therefore work as **overload**: the opponent gets more energy than its program can handle. If it cannot quickly emit, store, divide, or use the surplus, the overload damages its stability.

### World initialization (big bang)

The question of how to initialize the program of the initial cells largely resolved itself once we decided "energy carries exactly the information it contains." The big bang releases energy with arbitrary noise. Diffusion spreads it across space, and information along with it. Every cell is therefore continuously contaminated by its neighbors. Somewhere a coherent fragment accidentally accumulates, a cell with enough energy starts executing it (CPU rate grows with energy), and life arises emergently.

The question reduces from "how to initialize" to "how to make sure this phase doesn't last 500 million ticks" — that one stays open.

In the sparse production engine the big bang is literally **one origin cell holding all the world's energy**; the world then expands outward by alloc-on-write as energy diffuses into the void. There is no `MAX_MEMORY` / 4096-slot cap and no 8-bit address limit — addresses are 32-bit slot values taken modulo the cell's current `memSize` (`vm.md`, "Addressing"), so a single cell can legitimately hold the whole world at tick 0. (An earlier design assumed a per-cell slot cap forcing a "dense region of saturated cells, not a singularity"; the engine does not impose that cap, so the singular origin cell is the actual initial condition — see `questions.md`, "Resolved on initialization".)

## Self-encapsulation and multi-cellular organisms

A higher-level structure of multiple entities can arise if the entities coordinate programmatically (synchronous movement, exchange of energy / messages across shared faces). The engine has no notion of "ally" — cooperation arises only as a consequence of programs.

### The key physical advantage of a collective

The interior contact face between two cooperating entities **need not mean energy loss into open space**. When two entities sit side by side with similar energies, what one emits toward the other, the other absorbs. Energy that would otherwise leak into sparse space gets captured.

Consequences:

- a single entity radiates in all 6 directions (maximum loss)
- two adjacent synchronous entities reduce the effective loss on the shared face
- a larger cluster has a smaller surface relative to its volume
- internal communication via ports can serve as a nervous / data system
- coordinated movement allows behaving as a larger organism

This gives a physical motivation for multi-cellularity. An organism is not defined by a special rule, but as a **successful strategy**: build an environment around yourself in which less energy is lost and the program is better preserved.

### Self-encapsulation

An entity that can surround itself on all sides has a higher chance of survival. The key: the entity can **make those neighbors itself** through reproduction.

A possible scenario:

1. an entity gathers enough energy
2. it creates an offspring in a neighboring cell (via setp + wait)
3. the offspring stays nearby and programmatically holds position
4. parent and child exchange energy or messages across the shared face
5. gradually a shell or cluster forms
6. interior entities are more stable than isolated ones in sparse space

In the extreme, the entity creates neighbors in all directions and becomes the core of a small structure. More stable, but in need of coordination, communication, and role distribution.

## Mutation

Mutation comes from **the very physics of mixing**. The membrane is rewritten in every tick by content from neighbors, so content is constantly shifting in and out. No special aging rule is needed.

Self-modifying code is another source of change — a program may deliberately rewrite its own code, or unintentionally overwrite itself (especially with active ignition or mis-aimed addresses).

The aging of memory cells from prototype 2 was abandoned (consolidation 2026-05-01, point 9): diffusion as mixing supplies mutation on its own. If it ever turns out in simulation that a fully isolated entity in a perfectly equilibrated region stops evolving, aging may return as a backup source of entropy for stagnant regions. For now, on hold.

## Identity and war paint (UI layer)

The identity of a program continuity is **not gameplay**. The engine does not know it. For the player there are optional UI tools:

- **lineage tracker**: the UI looks for the cell with the best match on a low-address signature against a captured snapshot — tracking "where my entity moved" across metempsychosis
- **manual tag**: the player assigns a tag / color to a specific cell and follows its lineage over time
- **call-sign** (optional opcode `sid a`): the program can read its own origin tag and react (typically for "who am I? have I been attacked?")
- **war paint** (optional opcode `paint v`): the program sets its visual appearance, combined in the UI with energy intensity (HSV mapping). Purely aesthetic, does not affect physics.

These tools sit **outside game mechanics**. The programmer may use them, but their presence or absence doesn't change the physics of combat, dominance, or emission.

**Status:** `sid` and `paint` are implemented; lineage tracker and the UI tag layer are designed and waiting for implementation in further iterations. See `plan.md`.

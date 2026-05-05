// Aenternis simulation worker.
//
// Owns the WASM `World` instance and runs the per-tick step loop in
// the background. Sends a snapshot back to the main thread after every
// tick via `postMessage`, transferring the underlying ArrayBuffer
// (zero-copy — the worker doesn't keep a handle to it after sending).
//
// Protocol:
//
//   main → worker:
//     { type: "init", seed, energy, coeff, k, moveThreshold, rngKind,
//       legacyTickOffset, program }
//     { type: "config", coeff, k, moveThreshold, legacyTickOffset }
//     { type: "running", running }
//     { type: "inspect", x, y, z }
//
//   worker → main:
//     { type: "ready" }                                — after WASM init
//     { type: "snapshot", tick, cellCount, totalEnergy, snap, stride }
//                                                     — every tick
//
// `rngKind` is "pcg" (default) or "xorshift32"; the worker translates to
// the u8 the WASM bridge expects (0 / 1).
//
// `legacyTickOffset` is a boolean — when true, the world's
// `compute_natural_rates` keys its per-cell-tick RNG with `tick - 1` to
// match JS prototype 9-B's "compute layout pre-increment" quirk.

import init, { World } from "/crates/aenternis-wasm/pkg/aenternis_wasm.js";

await init();
postMessage({ type: "ready" });

let world = null;
let running = false;
let coeff = 0.20;
let k = 1;
let moveThreshold = 2.0;
let legacyTickOffset = false;

self.onmessage = (ev) => {
  const msg = ev.data;
  if (msg.type === "init") {
    if (world) world.free();
    // RNG backend: 0 = PCG (Aenternis default), 1 = xorshift32 (matches
    // JS prototype 9-B). The string form on the wire keeps the protocol
    // self-describing; translate to the u8 that the WASM bridge wants.
    const rngKindU8 = msg.rngKind === "xorshift32" ? 1 : 0;
    const programArr = msg.program && msg.program.length > 0
      ? (msg.program instanceof Uint32Array ? msg.program : new Uint32Array(msg.program))
      : new Uint32Array(0);
    world = World.newWithProgramAndKind(msg.seed, msg.energy, programArr, rngKindU8);
    coeff = msg.coeff;
    k = msg.k;
    moveThreshold = msg.moveThreshold ?? 2.0;
    world.setMoveThreshold(moveThreshold);
    legacyTickOffset = !!msg.legacyTickOffset;
    world.setLegacyTickOffset(legacyTickOffset);
    running = true;
    sendSnapshot(); // initial state, before any tick has run
    schedule();
  } else if (msg.type === "config") {
    coeff = msg.coeff;
    k = msg.k;
    if (typeof msg.moveThreshold === "number") {
      moveThreshold = msg.moveThreshold;
      if (world) world.setMoveThreshold(moveThreshold);
    }
    if (typeof msg.legacyTickOffset === "boolean") {
      legacyTickOffset = msg.legacyTickOffset;
      if (world) world.setLegacyTickOffset(legacyTickOffset);
    }
  } else if (msg.type === "running") {
    const wasRunning = running;
    running = msg.running;
    if (running && !wasRunning) schedule();
  } else if (msg.type === "inspect") {
    sendCellDetail(msg.x, msg.y, msg.z);
  }
};

function sendCellDetail(x, y, z) {
  if (!world) return;
  const data = world.cellInspect(x, y, z);
  postMessage(
    {
      type: "cellDetail",
      x, y, z,
      tick: world.tick(),
      data,
      prefix: world.inspectPrefix,
    },
    [data.buffer],
  );
}

function schedule() {
  // setTimeout(0) yields to the message loop between ticks so config /
  // pause messages still get through promptly. Use a microtask via
  // Promise.resolve() if you want max throughput at the cost of
  // message-handling latency.
  setTimeout(loop, 0);
}

let lastMsPerTick = 0;

function loop() {
  if (!world || !running) return;
  const t0 = performance.now();
  world.step(coeff, k);
  lastMsPerTick = performance.now() - t0;
  sendSnapshot();
  schedule();
}

function sendSnapshot() {
  if (!world) return;
  const snap = world.cellsSnapshot();
  postMessage(
    {
      type: "snapshot",
      tick: world.tick(),
      cellCount: world.cellCount(),
      totalEnergy: world.totalEnergy(),
      msPerTick: lastMsPerTick,
      snap,
      stride: world.snapshotStride,
    },
    [snap.buffer], // transferable — zero-copy ownership handoff
  );
}

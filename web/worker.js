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
//     { type: "init", seed, energy, coeff, k }
//     { type: "config", coeff, k }
//     { type: "running", running }
//
//   worker → main:
//     { type: "ready" }                                — after WASM init
//     { type: "snapshot", tick, cellCount, totalEnergy, snap, stride }
//                                                     — every tick

import init, { World } from "/crates/aenternis-wasm/pkg/aenternis_wasm.js";

await init();
postMessage({ type: "ready" });

let world = null;
let running = false;
let coeff = 0.20;
let k = 1;

self.onmessage = (ev) => {
  const msg = ev.data;
  if (msg.type === "init") {
    if (world) world.free();
    if (msg.program && msg.program.length > 0) {
      // wasm-bindgen `Vec<u32>` arg: pass a Uint32Array (or array of
      // numbers). We accept either for ergonomic JS.
      const programArr = msg.program instanceof Uint32Array
        ? msg.program
        : new Uint32Array(msg.program);
      world = World.newWithProgram(msg.seed, msg.energy, programArr);
    } else {
      world = new World(msg.seed, msg.energy);
    }
    coeff = msg.coeff;
    k = msg.k;
    running = true;
    sendSnapshot(); // initial state, before any tick has run
    schedule();
  } else if (msg.type === "config") {
    coeff = msg.coeff;
    k = msg.k;
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

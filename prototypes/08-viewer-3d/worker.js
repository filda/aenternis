"use strict";

// ===== Aenternis simulation worker =====
// Stejná logika jako v main.js, ale běží v Web Workeru.
// Komunikace s main threadem přes postMessage.
//
// Worker -> Main:
//   { type: 'origins', N, tags: Uint32Array }   (jednorazově po reset)
//   { type: 'state', step, energies: Uint32Array, msPerTick }   (po každém kroku)
//
// Main -> Worker:
//   { type: 'reset', N, scenario, coeff }
//   { type: 'run', running: bool }
//   { type: 'step' }
//   { type: 'setCoeff', coeff }

const DIRS = 6;
const OPPOSITE = [1, 0, 3, 2, 5, 4];
const DIR_OFFSET = [
  [+1, 0, 0], [-1, 0, 0],
  [0, +1, 0], [0, -1, 0],
  [0, 0, +1], [0, 0, -1],
];
const LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0];
const MAX_MEMORY = 16777216;

const sim = {
  N: 32,
  step: 0,
  cells: [],
  coeff: 0.15,
  running: false,
};

let lastMsPerTick = 0;

function idx(x, y, z) { return x + y * sim.N + z * sim.N * sim.N; }

function flatNeighborIdx(i, d) {
  const N = sim.N;
  const N2 = N * N;
  const x = i % N;
  const y = Math.floor(i / N) % N;
  const z = Math.floor(i / N2);
  const [dx, dy, dz] = DIR_OFFSET[d];
  return ((x + dx + N) % N) + ((y + dy + N) % N) * N + ((z + dz + N) % N) * N2;
}

function stochasticFloor(v) {
  if (v <= 0) return 0;
  const w = Math.floor(v);
  return w + (Math.random() < (v - w) ? 1 : 0);
}

function makeCell() {
  return {
    energy: 0,
    memory: new Uint32Array(0),
    pointers: [0,0,0,0,0,0],
    rates: [0,0,0,0,0,0],
    originTag: Math.floor(Math.random() * 0x100000000) >>> 0,
  };
}

function randomSlots(n) {
  const a = new Uint32Array(n);
  for (let i = 0; i < n; i++) a[i] = Math.floor(Math.random() * 0x100000000) >>> 0;
  return a;
}

function resetWorld() {
  const N = sim.N;
  sim.cells = new Array(N*N*N);
  for (let i = 0; i < sim.cells.length; i++) sim.cells[i] = makeCell();
  sim.step = 0;
}

function initScenario(scenario) {
  const N = sim.N;
  const c = Math.floor(N / 2);
  if (scenario === "point") {
    const totalE = N * N * N;
    sim.cells[idx(c, c, c)].energy = totalE;
    sim.cells[idx(c, c, c)].memory = randomSlots(totalE);
  } else if (scenario === "ball") {
    const r = Math.max(2, Math.floor(N / 6));
    const maxE = 64;
    for (let z = 0; z < N; z++)
      for (let y = 0; y < N; y++)
        for (let x = 0; x < N; x++) {
          const dx = x-c, dy = y-c, dz = z-c;
          const dist = Math.sqrt(dx*dx + dy*dy + dz*dz);
          if (dist <= r) {
            const e = Math.floor(maxE * (1 - dist/(r+1)));
            if (e > 0) {
              sim.cells[idx(x,y,z)].energy = e;
              sim.cells[idx(x,y,z)].memory = randomSlots(e);
            }
          }
        }
  } else if (scenario === "random") {
    for (let i = 0; i < sim.cells.length; i++) {
      const e = Math.floor(Math.random() * 3);
      if (e > 0) {
        sim.cells[i].energy = e;
        sim.cells[i].memory = randomSlots(e);
      }
    }
  }
  recomputeLayouts();
}

function recomputeLayouts() {
  const N = sim.N;
  const total = N*N*N;
  const cells = sim.cells;
  for (let i = 0; i < total; i++) {
    const cell = cells[i];
    const myE = cell.energy;
    let totalRate = 0;
    for (let d = 0; d < DIRS; d++) {
      const nE = cells[flatNeighborIdx(i, d)].energy;
      cell.rates[d] = (myE - nE > 0) ? stochasticFloor((myE - nE) * sim.coeff) : 0;
      totalRate += cell.rates[d];
    }
    const memSize = cell.memory.length;
    if (totalRate > memSize && totalRate > 0) {
      const scale = memSize / totalRate;
      let newTotal = 0;
      for (let d = 0; d < DIRS; d++) {
        cell.rates[d] = Math.floor(cell.rates[d] * scale);
        newTotal += cell.rates[d];
      }
      let leftover = memSize - newTotal;
      while (leftover > 0) {
        let added = false;
        for (let d = 0; d < DIRS; d++) {
          if (cell.rates[d] > 0 && leftover > 0) {
            cell.rates[d] += 1;
            leftover -= 1;
            added = true;
            break;
          }
        }
        if (!added) break;
      }
    }
    let cursor = cell.memory.length;
    for (const d of LAYOUT_ORDER_FROM_END) {
      cursor -= cell.rates[d];
      cell.pointers[d] = Math.max(0, cursor);
    }
  }
}

function step() {
  const N = sim.N;
  const total = N*N*N;
  const cells = sim.cells;

  // Phase 1: Outflow snapshot
  const outflows = new Array(total);
  for (let i = 0; i < total; i++) {
    const cell = cells[i];
    const out = new Array(DIRS);
    const memSize = cell.memory.length;
    for (let d = 0; d < DIRS; d++) {
      const rate = cell.rates[d];
      const ptr = cell.pointers[d];
      const slots = new Uint32Array(rate);
      if (memSize > 0) {
        for (let k = 0; k < rate; k++) slots[k] = cell.memory[(ptr+k) % memSize];
      }
      out[d] = slots;
    }
    outflows[i] = out;
  }

  // Phase 2: Apply
  for (let i = 0; i < total; i++) {
    const cell = cells[i];
    let totalOut = 0;
    for (let d = 0; d < DIRS; d++) totalOut += cell.rates[d];

    let totalIn = 0;
    const inflows = new Array(DIRS);
    for (let d = 0; d < DIRS; d++) {
      const inSlots = outflows[flatNeighborIdx(i, d)][OPPOSITE[d]];
      inflows[d] = inSlots;
      totalIn += inSlots.length;
    }

    const oldMem = cell.memory;
    const sizeAfterOut = Math.max(0, oldMem.length - totalOut);
    let newSize = sizeAfterOut + totalIn;
    if (newSize > MAX_MEMORY) newSize = MAX_MEMORY;
    const newMem = new Uint32Array(newSize);
    const preserveLen = Math.min(sizeAfterOut, newSize);
    for (let k = 0; k < preserveLen; k++) newMem[k] = oldMem[k];
    let pos = preserveLen;
    for (let d = 0; d < DIRS && pos < newSize; d++) {
      const inS = inflows[d];
      for (let k = 0; k < inS.length && pos < newSize; k++) newMem[pos++] = inS[k];
    }
    cell.memory = newMem;
    cell.energy = newSize;
  }

  recomputeLayouts();
  sim.step += 1;
}

function postState() {
  const total = sim.N * sim.N * sim.N;
  const energies = new Uint32Array(total);
  for (let i = 0; i < total; i++) energies[i] = sim.cells[i].energy;
  postMessage({
    type: 'state',
    step: sim.step,
    energies: energies,
    msPerTick: lastMsPerTick,
  }, [energies.buffer]);
}

function postOrigins() {
  const total = sim.N * sim.N * sim.N;
  const tags = new Uint32Array(total);
  for (let i = 0; i < total; i++) tags[i] = sim.cells[i].originTag;
  postMessage({
    type: 'origins',
    N: sim.N,
    tags: tags,
  }, [tags.buffer]);
}

function loop() {
  if (!sim.running) return;
  const t0 = (typeof performance !== 'undefined') ? performance.now() : Date.now();
  step();
  const t1 = (typeof performance !== 'undefined') ? performance.now() : Date.now();
  lastMsPerTick = t1 - t0;
  postState();
  // další tick - microtask přes setTimeout 0, aby zpráva mohla projít messageloopem
  setTimeout(loop, 0);
}

self.onmessage = (ev) => {
  const msg = ev.data;
  if (msg.type === 'reset') {
    sim.running = false;
    sim.N = msg.N;
    sim.coeff = msg.coeff;
    resetWorld();
    initScenario(msg.scenario);
    postOrigins();
    postState();
  } else if (msg.type === 'run') {
    const wasRunning = sim.running;
    sim.running = msg.running;
    if (sim.running && !wasRunning) loop();
  } else if (msg.type === 'step') {
    const t0 = performance.now();
    step();
    lastMsPerTick = performance.now() - t0;
    postState();
  } else if (msg.type === 'setCoeff') {
    sim.coeff = msg.coeff;
    recomputeLayouts();
  }
};

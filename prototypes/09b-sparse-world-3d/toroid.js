"use strict";

// Toroidní 3D svět pro comparison harness proti sparse světu.
// Port prototypu 9 toroidu rozšířený na 3D s 6 směry. Per-cell RNG seedovaný
// stejnou hash funkcí jako sparse svět (cellSeed). Tím obě fyziky sdílí stejný
// termální stav pro každou pozici a měly by být numericky identické,
// pokud sparse svět nepřekročí toroidní bbox.

const {
  OPCODES, DIR_NAMES, DIR_OFFSET, OPPOSITE, DIRS, makeRng,
} = require("./world.js");

const LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0];

// Hash precision flag matches the sparse world's `useMathImul` — see
// `world.js` for full discussion. Both reference (toroid) and sparse must
// pick the same backend or the equivalence harness diverges before tick 1.
function mulMod32F64(a, b) {
  return (a * b) >>> 0;
}
function imulMod32(a, b) {
  return Math.imul(a, b) >>> 0;
}

function cellSeed(worldSeed, x, y, z, useMathImul = false) {
  const mul = useMathImul ? imulMod32 : mulMod32F64;
  let h = (worldSeed >>> 0) ^ 0x9E3779B9;
  h = mul(h + (x | 0), 374761393);
  h ^= h >>> 13;
  h = mul(h + (y | 0), 668265263);
  h ^= h >>> 16;
  h = mul(h + (z | 0), 1274126177);
  h ^= h >>> 13;
  if (h === 0) h = 1;
  return h;
}

function cellTickSeed(worldSeed, x, y, z, tick, useMathImul = false) {
  const mul = useMathImul ? imulMod32 : mulMod32F64;
  let h = cellSeed(worldSeed, x, y, z, useMathImul);
  h = mul(h + (tick | 0), 2246822507);
  h ^= h >>> 16;
  if (h === 0) h = 1;
  return h;
}

function makeCell(x, y, z, worldSeed, useMathImul = false) {
  const seed = cellSeed(worldSeed, x, y, z, useMathImul);
  return {
    x, y, z,
    energy: 0,
    memory: new Uint32Array(0),
    pointers: [0, 0, 0, 0, 0, 0],
    rates: [0, 0, 0, 0, 0, 0],
    activeOutflow: [0, 0, 0, 0, 0, 0],
    pointerOverridden: [false, false, false, false, false, false],
    pc: 0,
    lastInst: null,
    instCount: 0,
    tickBudget: 0,
    originTag: seed,
    appearance: 0,
    rng: makeRng(seed),
  };
}

function rngFloat(rng) { return rng() / 0x100000000; }

class ToroidWorld {
  constructor(opts = {}) {
    this.N = opts.N ?? 64;
    this.diffusionCoeff = opts.diffusionCoeff ?? 0.15;
    this.cpuK = opts.cpuK ?? 1;
    this.moveThreshold = opts.moveThreshold ?? 2.0;
    this.worldSeed = (opts.seed ?? 1) >>> 0;
    this.useMathImul = opts.useMathImul ?? false;
    this.tick = 0;
    const N = this.N;
    this.cells = new Array(N * N * N);
    // Souřadnice toroidu: středovaný kolem (0,0,0). Cell (x, y, z) má index
    // ((z+half)*N*N + (y+half)*N + (x+half)). Bbox toroidu: [-N/2, N/2 - 1]
    // na každé z os.
    this.half = Math.floor(N / 2);
    for (let z = 0; z < N; z++) {
      for (let y = 0; y < N; y++) {
        for (let x = 0; x < N; x++) {
          const cx = x - this.half;
          const cy = y - this.half;
          const cz = z - this.half;
          this.cells[(z * N + y) * N + x] = makeCell(cx, cy, cz, this.worldSeed, this.useMathImul);
        }
      }
    }
  }

  idx(cx, cy, cz) {
    const N = this.N;
    const xx = ((cx + this.half) % N + N) % N;
    const yy = ((cy + this.half) % N + N) % N;
    const zz = ((cz + this.half) % N + N) % N;
    return (zz * N + yy) * N + xx;
  }

  getCell(cx, cy, cz) {
    return this.cells[this.idx(cx, cy, cz)];
  }

  bigBang(eTotal, programSlots = []) {
    // Reset všech buněk
    for (let i = 0; i < this.cells.length; i++) {
      const cell = this.cells[i];
      cell.energy = 0;
      cell.memory = new Uint32Array(0);
      cell.pointers = [0, 0, 0, 0, 0, 0];
      cell.rates = [0, 0, 0, 0, 0, 0];
      cell.activeOutflow = [0, 0, 0, 0, 0, 0];
      cell.pointerOverridden = [false, false, false, false, false, false];
      cell.pc = 0;
      cell.lastInst = null;
      cell.instCount = 0;
      cell.tickBudget = 0;
      cell.appearance = 0;
      cell.rng = makeRng(cell.originTag);
    }
    this.tick = 0;
    if (eTotal <= 0) return;

    const cell = this.getCell(0, 0, 0);
    cell.energy = eTotal;
    const mem = new Uint32Array(eTotal);
    for (let i = 0; i < eTotal; i++) {
      mem[i] = i < programSlots.length ? (programSlots[i] >>> 0) : (cell.rng() >>> 0);
    }
    cell.memory = mem;
    cell.tickBudget = Math.floor(cell.energy / this.cpuK);
    this.recomputeAllLayouts();
  }

  totalEnergy() {
    let s = 0;
    for (const c of this.cells) s += c.energy;
    return s;
  }

  // Bbox aktivních buněk (E > 0). Pro porovnání se sparse světem.
  activeBbox() {
    let xMin = Infinity, xMax = -Infinity;
    let yMin = Infinity, yMax = -Infinity;
    let zMin = Infinity, zMax = -Infinity;
    let count = 0;
    for (const c of this.cells) {
      if (c.energy > 0) {
        count++;
        if (c.x < xMin) xMin = c.x;
        if (c.x > xMax) xMax = c.x;
        if (c.y < yMin) yMin = c.y;
        if (c.y > yMax) yMax = c.y;
        if (c.z < zMin) zMin = c.z;
        if (c.z > zMax) zMax = c.z;
      }
    }
    return count === 0 ? null : { xMin, xMax, yMin, yMax, zMin, zMax, count };
  }

  stochasticFloorRng(rng, value) {
    if (value <= 0) return 0;
    const whole = Math.floor(value);
    const frac = value - whole;
    return whole + (rngFloat(rng) < frac ? 1 : 0);
  }

  proportionalClamp(rates, cap) {
    let total = 0;
    for (let d = 0; d < DIRS; d++) total += rates[d];
    if (total <= cap) return;
    const scale = cap / total;
    let newTotal = 0;
    for (let d = 0; d < DIRS; d++) {
      rates[d] = Math.floor(rates[d] * scale);
      newTotal += rates[d];
    }
    let leftover = cap - newTotal;
    while (leftover > 0) {
      let added = false;
      for (let d = 0; d < DIRS; d++) {
        if (rates[d] > 0 && leftover > 0) {
          rates[d] += 1;
          leftover -= 1;
          added = true;
          break;
        }
      }
      if (!added) break;
    }
  }

  recomputeAllLayouts() {
    const coeff = this.diffusionCoeff;
    for (const cell of this.cells) {
      const rng = makeRng(cellTickSeed(this.worldSeed, cell.x, cell.y, cell.z, this.tick, this.useMathImul));
      const myE = cell.energy;
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        const [dx, dy, dz] = DIR_OFFSET[d];
        const neighbor = this.getCell(cell.x + dx, cell.y + dy, cell.z + dz);
        const nE = neighbor.energy;
        const diff = myE - nE;
        const rate = diff > 0 ? this.stochasticFloorRng(rng, diff * coeff) : 0;
        cell.rates[d] = rate;
        totalRate += rate;
      }
      const memSize = cell.memory.length;
      if (totalRate > memSize && totalRate > 0) {
        this.proportionalClamp(cell.rates, memSize);
      }
      let cursor = cell.memory.length;
      for (const d of LAYOUT_ORDER_FROM_END) {
        cursor -= cell.rates[d];
        cell.pointers[d] = Math.max(0, cursor);
      }
    }
  }

  applyCombinedLayout() {
    for (const cell of this.cells) {
      const memSize = cell.memory.length;
      if (memSize === 0) continue;

      const combined = new Array(DIRS);
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        combined[d] = cell.rates[d] + cell.activeOutflow[d];
        totalRate += combined[d];
      }
      if (totalRate > memSize && totalRate > 0) {
        this.proportionalClamp(combined, memSize);
      }

      let cursor = memSize;
      for (const d of LAYOUT_ORDER_FROM_END) {
        if (!cell.pointerOverridden[d]) {
          cursor -= combined[d];
          cell.pointers[d] = Math.max(0, cursor);
        }
      }
    }
  }

  executeOne(cell) {
    const mem = cell.memory;
    const memSize = mem.length;
    if (memSize === 0) return false;

    const pc = cell.pc % memSize;
    const slot = mem[pc];
    const opcode = slot & 0xFF;
    const op = OPCODES[opcode];

    if (!op) {
      cell.pc = (pc + 1) % memSize;
      cell.lastInst = { opcode, name: "??", args: [], addr: pc };
      cell.instCount++;
      return true;
    }

    const arg0 = op.len > 1 ? mem[(pc + 1) % memSize] : 0;
    const arg1 = op.len > 2 ? mem[(pc + 2) % memSize] : 0;
    const arg2 = op.len > 3 ? mem[(pc + 3) % memSize] : 0;
    let nextPc = (pc + op.len) % memSize;

    const addr = (v) => v % memSize;
    const wrap32 = (v) => v >>> 0;

    switch (opcode) {
      case 0x00: break;
      case 0x01: mem[addr(arg0)] = wrap32(arg1); break;
      case 0x02: mem[addr(arg0)] = mem[addr(arg1)]; break;
      case 0x03: mem[addr(arg0)] = wrap32(mem[addr(arg0)] + mem[addr(arg1)]); break;
      case 0x04: mem[addr(arg0)] = wrap32(mem[addr(arg0)] - mem[addr(arg1)]); break;
      case 0x05: mem[addr(arg0)] = wrap32(mem[addr(arg0)] + 1); break;
      case 0x06: mem[addr(arg0)] = wrap32(mem[addr(arg0)] - 1); break;
      case 0x07: nextPc = addr(arg0); break;
      case 0x08:
        if (mem[addr(arg0)] === 0) nextPc = addr(arg1);
        break;
      case 0x09: {
        const d = arg0 % DIRS;
        cell.pointers[d] = addr(arg1);
        cell.pointerOverridden[d] = true;
        break;
      }
      case 0x0A:
        mem[addr(arg1)] = cell.pointers[arg0 % DIRS];
        break;
      case 0x0B: {
        const dir = arg0 % DIRS;
        cell.activeOutflow[dir] = wrap32(cell.activeOutflow[dir] + arg1);
        break;
      }
      case 0x0C: {
        const d = arg0 % DIRS;
        const [dx, dy, dz] = DIR_OFFSET[d];
        const neighbor = this.getCell(cell.x + dx, cell.y + dy, cell.z + dz);
        mem[addr(arg1)] = wrap32(neighbor.energy);
        break;
      }
      case 0x0D:
        if (mem[addr(arg0)] !== 0) nextPc = addr(arg1);
        break;
      case 0x0E:
        if (mem[addr(arg0)] === mem[addr(arg1)]) nextPc = addr(arg2);
        break;
      case 0x0F:
        mem[addr(arg0)] = mem[addr(mem[addr(arg1)])];
        break;
      case 0x10:
        mem[addr(mem[addr(arg0)])] = mem[addr(arg1)];
        break;
      case 0x11: {
        const d = arg0 % DIRS;
        cell.pointers[d] = addr(mem[addr(arg1)]);
        cell.pointerOverridden[d] = true;
        break;
      }
      case 0x12:
        mem[addr(arg0)] = cell.originTag >>> 0;
        break;
      case 0x13:
        cell.appearance = arg0 >>> 0;
        break;
    }

    cell.pc = nextPc;
    cell.instCount++;
    return true;
  }

  runCpuPhase() {
    for (const cell of this.cells) {
      if (cell.energy <= 0) {
        cell.tickBudget = 0;
        continue;
      }
      while (cell.tickBudget > 0) {
        const ok = this.executeOne(cell);
        if (!ok) break;
        cell.tickBudget -= 1;
      }
    }
  }

  refillTickBudgets() {
    const K = this.cpuK;
    for (const cell of this.cells) {
      cell.tickBudget = cell.energy > 0 ? Math.floor(cell.energy / K) : 0;
    }
  }

  step() {
    this.runCpuPhase();
    this.applyCombinedLayout();

    const total = this.cells.length;
    const outflows = new Array(total);
    const combinedRates = new Array(total);

    for (let i = 0; i < total; i++) {
      const cell = this.cells[i];
      const memSize = cell.memory.length;
      const rates = new Array(DIRS);
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        rates[d] = cell.rates[d] + cell.activeOutflow[d];
        totalRate += rates[d];
      }
      if (totalRate > memSize && totalRate > 0) {
        this.proportionalClamp(rates, memSize);
      }
      combinedRates[i] = rates;

      const out = new Array(DIRS);
      for (let d = 0; d < DIRS; d++) {
        const rate = rates[d];
        const ptr = cell.pointers[d];
        const slots = new Uint32Array(rate);
        if (memSize > 0) {
          for (let k = 0; k < rate; k++) {
            slots[k] = cell.memory[(ptr + k) % memSize];
          }
        }
        out[d] = slots;
      }
      outflows[i] = out;
    }

    // Snapshot pre-step energie (stejný invariant jako v sparse — dominance
    // se počítá z útočníkovy energie před tickem, ne z hodnoty, která už
    // mohla být v této fázi přepsána).
    const preStepEnergy = new Array(total);
    for (let i = 0; i < total; i++) preStepEnergy[i] = this.cells[i].energy;

    const moveThreshold = this.moveThreshold;
    for (let i = 0; i < total; i++) {
      const cell = this.cells[i];
      const myRates = combinedRates[i];
      let totalOutflow = 0;
      for (let d = 0; d < DIRS; d++) totalOutflow += myRates[d];
      const oldMem = cell.memory;
      const sizeAfterOutflow = Math.max(0, oldMem.length - totalOutflow);
      const targetE = sizeAfterOutflow;

      const inflowEntries = [];
      for (let d = 0; d < DIRS; d++) {
        const [dx, dy, dz] = DIR_OFFSET[d];
        const nIdx = this.idx(cell.x + dx, cell.y + dy, cell.z + dz);
        const inSlots = outflows[nIdx][OPPOSITE[d]];
        if (inSlots.length === 0) continue;

        const neighbor = this.cells[nIdx];
        const nRates = combinedRates[nIdx];
        let neighborTotalOut = 0;
        for (let dd = 0; dd < DIRS; dd++) neighborTotalOut += nRates[dd];
        const attackerEPostBurn = Math.max(1, preStepEnergy[nIdx] - neighborTotalOut);

        const r = targetE / attackerEPostBurn;
        const dominance = Math.max(0, Math.min(1, 1 - r / moveThreshold));
        inflowEntries.push({ d, slots: inSlots, dominance, srcTag: neighbor.originTag });
      }

      inflowEntries.sort((a, b) => b.dominance - a.dominance || a.d - b.d);

      if (inflowEntries.length > 0 && inflowEntries[0].dominance >= 0.5) {
        cell.originTag = inflowEntries[0].srcTag;
      }

      let workMem = oldMem.subarray(0, sizeAfterOutflow);

      for (const entry of inflowEntries) {
        const slots = entry.slots;
        const dom = entry.dominance;
        const currentSize = workMem.length;
        const intrusionDepth = Math.floor(dom * currentSize);
        const writeStart = Math.max(0, currentSize - intrusionDepth);

        const newSize = currentSize + slots.length;
        const merged = new Uint32Array(newSize);
        let pos = 0;
        for (let k = 0; k < writeStart; k++) merged[pos++] = workMem[k];
        for (let k = 0; k < slots.length; k++) merged[pos++] = slots[k];
        for (let k = writeStart; k < currentSize; k++) merged[pos++] = workMem[k];
        workMem = merged;
      }

      if (workMem.buffer === oldMem.buffer && workMem.length > 0) {
        const copy = new Uint32Array(workMem.length);
        copy.set(workMem);
        workMem = copy;
      }
      if (workMem.length === 0) workMem = new Uint32Array(0);

      cell.memory = workMem;
      cell.energy = workMem.length;
      if (cell.memory.length > 0) {
        cell.pc = cell.pc % cell.memory.length;
      } else {
        cell.pc = 0;
      }
    }

    for (const cell of this.cells) {
      for (let d = 0; d < DIRS; d++) {
        cell.activeOutflow[d] = 0;
        cell.pointerOverridden[d] = false;
      }
    }

    this.recomputeAllLayouts();
    this.refillTickBudgets();
    this.tick += 1;
  }
}

module.exports = { ToroidWorld };

"use strict";

// ===== Konstanty (2D varianta) =====

const DIRS = 4;
// 0=xp, 1=xn, 2=yp, 3=yn
const OPPOSITE = [1, 0, 3, 2];
const DIR_OFFSET = [
  [+1, 0],
  [-1, 0],
  [0, +1],
  [0, -1],
];
const DIR_NAMES = ["xp", "xn", "yp", "yn"];
const LAYOUT_ORDER_FROM_END = [3, 2, 1, 0]; // yn, yp, xn, xp

const MAX_MEMORY = 16384;  // praktický cap per buňka (16K slotů = 64KB)
                            // Poznámka: pokud cela v Phase 2 přesáhne MAX_MEMORY, přebytek
                            // inflow se ořízne -> mírná energy leakage. Pro běžné scénáře
                            // s rozumnou počáteční energií (max ~ N * 4) se cap nedosáhne.

const OPCODES = {
  0x00: { name: "nop",     len: 1 },
  0x01: { name: "set",     len: 3 },
  0x02: { name: "copy",    len: 3 },
  0x03: { name: "add",     len: 3 },
  0x04: { name: "sub",     len: 3 },
  0x05: { name: "inc",     len: 2 },
  0x06: { name: "dec",     len: 2 },
  0x07: { name: "jmp",     len: 2 },
  0x08: { name: "jz",      len: 3 },
  0x09: { name: "setp",    len: 3 }, // pointers[arg0 % DIRS] = arg1
  0x0A: { name: "getp",    len: 3 }, // mem[arg1] = pointers[arg0 % DIRS]
  0x0B: { name: "port",    len: 3 }, // active outflow ve směru arg0%DIRS
  0x0C: { name: "senergy", len: 3 }, // mem[arg1] = energie souseda ve směru arg0%DIRS
  0x0D: { name: "jne",     len: 3 }, // if mem[arg0] != 0: PC = arg1
  0x0E: { name: "je",      len: 4 }, // if mem[arg0] == mem[arg1]: PC = arg2
  0x0F: { name: "ldi",     len: 3 }, // mem[arg0] = mem[mem[arg1]] - load indirect
  0x10: { name: "sti",     len: 3 }, // mem[mem[arg0]] = mem[arg1] - store indirect
  0x11: { name: "setpv",   len: 3 }, // pointers[arg0 % DIRS] = mem[arg1] - setp s runtime hodnotou
  0x12: { name: "sid",     len: 2 }, // mem[arg0] = vlastní origin_tag (call-sign)
  0x13: { name: "paint",   len: 2 }, // appearance = arg0 (war paint, čistě UI)
};

// Preset programy přizpůsobené 4 směrům (dir 0-3)
const PRESETS = {
  counter: "05 10 07 00",
  self_xp: "09 00 00 07 00",
  self_omni: "09 00 00 09 01 00 09 02 00 09 03 00 07 00",
  beacon: "05 20 09 00 00 07 00",
  quine_core: "01 10 de 01 11 ad 01 12 be 01 13 ef 09 00 10 07 00",
  deadbeef: "DE AD BE EF",
  projectile: "09 00 00 0b 00 20 07 00",
};

// ===== DOM odkazy =====

const dom = {
  canvas: document.getElementById("worldCanvas"),
  step: document.getElementById("stepValue"),
  total: document.getElementById("totalEnergyMetric"),
  range: document.getElementById("rangeMetric"),
  stats: document.getElementById("statsMetric"),
  run: document.getElementById("runButton"),
  stepBtn: document.getElementById("stepButton"),
  reset: document.getElementById("resetButton"),
  spf: document.getElementById("stepsPerFrameInput"),
  size: document.getElementById("sizeInput"),
  scenario: document.getElementById("scenarioInput"),
  maxEnergy: document.getElementById("maxEnergyInput"),
  diffusion: document.getElementById("diffusionInput"),
  diffusionVal: document.getElementById("diffusionValue"),
  cpuK: document.getElementById("cpuKInput"),
  moveThreshold: document.getElementById("moveThresholdInput"),
  moveThresholdVal: document.getElementById("moveThresholdValue"),
  viz: Array.from(document.querySelectorAll("input[name='viz']")),
  highlightPair: document.getElementById("highlightPair"),
  aX: document.getElementById("aX"),
  aY: document.getElementById("aY"),
  bX: document.getElementById("bX"),
  bY: document.getElementById("bY"),
  setAButton: document.getElementById("setAButton"),
  setBButton: document.getElementById("setBButton"),
  aCoord: document.getElementById("aCoord"),
  bCoord: document.getElementById("bCoord"),
  // A inspector
  aEnergy: document.getElementById("aEnergy"),
  aMemSize: document.getElementById("aMemSize"),
  aCpuRate: document.getElementById("aCpuRate"),
  aPc: document.getElementById("aPc"),
  aInstCount: document.getElementById("aInstCount"),
  aLastInst: document.getElementById("aLastInst"),
  aPointers: document.getElementById("aPointers"),
  aRates: document.getElementById("aRates"),
  aMemory: document.getElementById("aMemory"),
  aPreset: document.getElementById("aPresetInput"),
  aInject: document.getElementById("aInjectInput"),
  aInjectBtn: document.getElementById("aInjectButton"),
  aCpuStepBtn: document.getElementById("aCpuStepButton"),
  aTag: document.getElementById("aTag"),
  aPaint: document.getElementById("aPaint"),
  aTrackBtn: document.getElementById("aTrackButton"),
  // B inspector
  bEnergy: document.getElementById("bEnergy"),
  bMemSize: document.getElementById("bMemSize"),
  bCpuRate: document.getElementById("bCpuRate"),
  bPc: document.getElementById("bPc"),
  bInstCount: document.getElementById("bInstCount"),
  bLastInst: document.getElementById("bLastInst"),
  bPointers: document.getElementById("bPointers"),
  bRates: document.getElementById("bRates"),
  bMemory: document.getElementById("bMemory"),
  bPreset: document.getElementById("bPresetInput"),
  bInject: document.getElementById("bInjectInput"),
  bInjectBtn: document.getElementById("bInjectButton"),
  bCpuStepBtn: document.getElementById("bCpuStepButton"),
  bTag: document.getElementById("bTag"),
  bPaint: document.getElementById("bPaint"),
  bTrackBtn: document.getElementById("bTrackButton"),
  // Comm trace
  commAtoB: document.getElementById("commAtoB"),
  commBtoA: document.getElementById("commBtoA"),
};

const ctx = dom.canvas.getContext("2d");

// Naplň presety do dropdownu
function fillPresets(selectEl) {
  for (const [key, value] of Object.entries(PRESETS)) {
    const opt = document.createElement("option");
    opt.value = key;
    opt.textContent = key;
    selectEl.appendChild(opt);
  }
}
fillPresets(dom.aPreset);
fillPresets(dom.bPreset);

// ===== Stav simulace =====

const state = {
  N: 48,
  step: 0,
  running: false,
  viz: "energy",
  diffusionCoeff: 0.15,
  cpuK: 1,
  moveThreshold: 2.0,  // dominance = clamp(1 - r/moveThreshold, 0, 1)
  cells: [],
  imageData: null,
  // Komunikační stopa A↔B z posledního taktu
  lastCommAtoB: null,
  lastCommBtoA: null,
  // Lineage tracking
  trackA: null,  // { snapshot: Uint32Array, len: number } nebo null
  trackB: null,
};

const LINEAGE_SNAPSHOT_LEN = 16; // kolik prvních slotů zachytit jako podpis

function idx(x, y) {
  return x + y * state.N;
}

function neighborIdx(x, y, d) {
  const N = state.N;
  const [dx, dy] = DIR_OFFSET[d];
  const nx = (x + dx + N) % N;
  const ny = (y + dy + N) % N;
  return idx(nx, ny);
}

// ===== Aktivita / opcode =====

function isMeaningfulInst(opcode) {
  return opcode >= 0x01 && opcode <= 0x13;
}

const ACTIVITY_DECAY = 0.95;

// ===== Inicializace =====

function makeEmptyCell() {
  return {
    energy: 0,
    memory: new Uint32Array(0),
    pointers: [0, 0, 0, 0],
    rates: [0, 0, 0, 0],
    activeOutflow: [0, 0, 0, 0],
    pointerOverridden: [false, false, false, false],
    pc: 0,
    lastInst: null,
    instCount: 0,
    activity: 0,
    tickBudget: 0,
    originTag: 0,    // 32-bit identity, default 0; lze přiřadit randomly při resetu
    appearance: 0,   // 32-bit war paint, čistě pro UI
  };
}

function reset() {
  state.N = clamp(parseInt(dom.size.value, 10), 8, 128);
  state.step = 0;
  dom.aX.max = state.N - 1;
  dom.aY.max = state.N - 1;
  dom.bX.max = state.N - 1;
  dom.bY.max = state.N - 1;
  if (parseInt(dom.aX.value, 10) >= state.N) dom.aX.value = Math.floor(state.N / 2) - 1;
  if (parseInt(dom.aY.value, 10) >= state.N) dom.aY.value = Math.floor(state.N / 2);
  if (parseInt(dom.bX.value, 10) >= state.N) dom.bX.value = Math.floor(state.N / 2);
  if (parseInt(dom.bY.value, 10) >= state.N) dom.bY.value = Math.floor(state.N / 2);

  state.cells = new Array(state.N * state.N);
  for (let i = 0; i < state.cells.length; i++) {
    state.cells[i] = makeEmptyCell();
    // Každá buňka má unikátní pseudo-random origin_tag pro identifikaci
    state.cells[i].originTag = Math.floor(Math.random() * 0x100000000) >>> 0;
  }
  state.lastCommAtoB = null;
  state.lastCommBtoA = null;
  initScenario();
  recomputeAllLayouts();
  refillTickBudgets();
  setupImageData();
  render();
  updateMetrics();
  updateInspectors();
}

function initScenario() {
  const scenario = dom.scenario.value;
  const maxE = clamp(parseInt(dom.maxEnergy.value, 10), 1, MAX_MEMORY);
  if (scenario === "empty") return;
  if (scenario === "quiet") {
    const bg = Math.max(1, Math.floor(maxE / 4));
    for (let y = 0; y < state.N; y++)
      for (let x = 0; x < state.N; x++) {
        const slots = randomSlots(bg);
        setCellEnergy(x, y, bg, slots);
      }
  } else if (scenario === "noise") {
    for (let y = 0; y < state.N; y++)
      for (let x = 0; x < state.N; x++) {
        const e = Math.floor(Math.random() * (maxE + 1));
        if (e > 0) {
          const slots = randomSlots(e);
          setCellEnergy(x, y, e, slots);
        }
      }
  }
}

function setCellEnergy(x, y, energy, slots) {
  const cell = state.cells[idx(x, y)];
  cell.energy = energy;
  cell.memory = slots;
}

function randomSlots(n) {
  const a = new Uint32Array(n);
  for (let i = 0; i < n; i++) a[i] = Math.floor(Math.random() * 0x100000000) >>> 0;
  return a;
}

// ===== CPU =====

function executeOne(cell, cellIdx) {
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
    case 0x09: { // setp d, v
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
    case 0x0C: { // senergy d, a
      const d = arg0 % DIRS;
      // Najít sousední index v daném směru. Potřebujeme x,y aktuální buňky.
      const x = cellIdx % state.N;
      const y = Math.floor(cellIdx / state.N);
      const nIdx = neighborIdx(x, y, d);
      mem[addr(arg1)] = wrap32(state.cells[nIdx].energy);
      break;
    }
    case 0x0D: // jne a t
      if (mem[addr(arg0)] !== 0) nextPc = addr(arg1);
      break;
    case 0x0E: // je a b t
      if (mem[addr(arg0)] === mem[addr(arg1)]) nextPc = addr(arg2);
      break;
    case 0x0F: // ldi a, b - mem[a] = mem[mem[b]]
      mem[addr(arg0)] = mem[addr(mem[addr(arg1)])];
      break;
    case 0x10: // sti a, b - mem[mem[a]] = mem[b]
      mem[addr(mem[addr(arg0)])] = mem[addr(arg1)];
      break;
    case 0x11: { // setpv d, a - pointers[d % DIRS] = mem[a]
      const d = arg0 % DIRS;
      cell.pointers[d] = addr(mem[addr(arg1)]);
      cell.pointerOverridden[d] = true;
      break;
    }
    case 0x12: // sid a - mem[a] = vlastní origin_tag
      mem[addr(arg0)] = cell.originTag >>> 0;
      break;
    case 0x13: // paint v - appearance = v
      cell.appearance = arg0 >>> 0;
      break;
  }

  cell.pc = nextPc;
  const argList = op.len > 1
    ? (op.len > 3 ? [arg0, arg1, arg2] : (op.len > 2 ? [arg0, arg1] : [arg0]))
    : [];
  cell.lastInst = { opcode, name: op.name, args: argList, addr: pc };
  cell.instCount++;
  if (isMeaningfulInst(opcode)) cell.activity += 1;
  return true;
}

function runCpuPhase() {
  const cells = state.cells;
  if (state.cpuK <= 0) return;
  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    cell.activity *= ACTIVITY_DECAY;
    if (cell.energy <= 0) {
      cell.tickBudget = 0;
      continue;
    }
    // Vykonej zbylý budget pro tento takt (programátor mohl něco už ručně CPU-krokovat)
    while (cell.tickBudget > 0) {
      const ok = executeOne(cell, i);
      if (!ok) break;
      cell.tickBudget -= 1;
    }
  }
}

function refillTickBudgets() {
  const K = state.cpuK;
  for (const cell of state.cells) {
    cell.tickBudget = cell.energy > 0 ? Math.floor(cell.energy / K) : 0;
  }
}

// ===== Krok simulace =====

function step() {
  const N = state.N;
  const total = N * N;
  const cells = state.cells;

  // DIAGNOSTIKA: total energie před krokem
  let energyBefore = 0;
  for (let i = 0; i < total; i++) energyBefore += cells[i].energy;

  // Fáze 0: CPU
  runCpuPhase();

  // Fáze 0.5: přepočet pointer layoutu pro tento takt s combined_rate
  // Pro směry, které programátor nepřepsal přes setp/setpv, posuneme pointer
  // tak, aby segment od konce paměti odpovídal combined_rate (= natural + active)
  applyCombinedLayout();

  // Fáze 1: snímek odtoku s kombinovaným rate (passive + active)
  const outflows = new Array(total);
  const combinedRates = new Array(total);
  for (let i = 0; i < total; i++) {
    const cell = cells[i];
    const memSize = cell.memory.length;
    const rates = new Array(DIRS);
    let totalRate = 0;
    for (let d = 0; d < DIRS; d++) {
      rates[d] = cell.rates[d] + cell.activeOutflow[d];
      totalRate += rates[d];
    }
    if (totalRate > memSize && totalRate > 0) {
      const scale = memSize / totalRate;
      let newTotal = 0;
      for (let d = 0; d < DIRS; d++) {
        rates[d] = Math.floor(rates[d] * scale);
        newTotal += rates[d];
      }
      let leftover = memSize - newTotal;
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

  // Zachycení komunikační stopy A → B a B → A (pokud sousedí)
  state.lastCommAtoB = null;
  state.lastCommBtoA = null;
  const ax = clamp(parseInt(dom.aX.value, 10), 0, N - 1);
  const ay = clamp(parseInt(dom.aY.value, 10), 0, N - 1);
  const bx = clamp(parseInt(dom.bX.value, 10), 0, N - 1);
  const by = clamp(parseInt(dom.bY.value, 10), 0, N - 1);
  const aIdx = idx(ax, ay);
  const bIdx = idx(bx, by);
  for (let d = 0; d < DIRS; d++) {
    if (neighborIdx(ax, ay, d) === bIdx) {
      // A je soused B ve směru d, takže A posílá B přes svůj směr d
      state.lastCommAtoB = outflows[aIdx][d];
      // B posílá A v opačném směru
      state.lastCommBtoA = outflows[bIdx][OPPOSITE[d]];
      break;
    }
  }

  // Fáze 2: aplikace odtoku/přítoku s dominance/intrusion
  // Pro každou cellu spočítáme dominance per inflow směr, setřídíme,
  // a vsuneme inflows do paměti od nejhlubšího (nejvyšší dominance) k nejmělčímu.
  const moveThreshold = state.moveThreshold;
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const i = idx(x, y);
      const cell = cells[i];
      const myRates = combinedRates[i];

      // Můj celkový outflow
      let totalOutflow = 0;
      for (let d = 0; d < DIRS; d++) totalOutflow += myRates[d];

      // Můj cílový stav po vlastním outflow (před aplikací inflows)
      const oldMem = cell.memory;
      const sizeAfterOutflow = Math.max(0, oldMem.length - totalOutflow);
      const targetE = sizeAfterOutflow;  // = energie po vlastním zážehu

      // Sběr inflows + dominance per direction
      // attacker_E_post_burn = neighbor's energy - neighbor's total outflow (oba pre-tick stavy)
      const inflowEntries = [];
      for (let d = 0; d < DIRS; d++) {
        const nIdx = neighborIdx(x, y, d);
        const inSlots = outflows[nIdx][OPPOSITE[d]];
        if (inSlots.length === 0) continue;

        const neighbor = cells[nIdx];
        const nRates = combinedRates[nIdx];
        let neighborTotalOut = 0;
        for (let dd = 0; dd < DIRS; dd++) neighborTotalOut += nRates[dd];
        const attackerEPostBurn = Math.max(1, neighbor.energy - neighborTotalOut);

        const r = targetE / attackerEPostBurn;
        const dominance = Math.max(0, Math.min(1, 1 - r / moveThreshold));
        inflowEntries.push({ d, slots: inSlots, dominance });
      }

      // Setřídit podle dominance descending. Tie-break: směr d (stable order).
      inflowEntries.sort((a, b) => b.dominance - a.dominance || a.d - b.d);

      // Tag propagace: pokud nejvyšší dominance ≥ 0.5, target zdědí útočníkův originTag
      if (inflowEntries.length > 0 && inflowEntries[0].dominance >= 0.5) {
        const topEntry = inflowEntries[0];
        const nIdx = neighborIdx(x, y, topEntry.d);
        cell.originTag = cells[nIdx].originTag;
      }

      // Začneme s pamětí po vlastním outflow (zachovaných sizeAfterOutflow slotů z konce)
      let workMem = oldMem.slice(0, sizeAfterOutflow);

      // Postupně vsouvat inflows
      for (const entry of inflowEntries) {
        const slots = entry.slots;
        const dominance = entry.dominance;
        const currentSize = workMem.length;
        const intrusionDepth = Math.floor(dominance * currentSize);
        const writeStart = Math.max(0, currentSize - intrusionDepth);

        // Vsuvka: workMem[0..writeStart-1] + slots + workMem[writeStart..]
        const newSize = currentSize + slots.length;
        const cappedSize = Math.min(newSize, MAX_MEMORY);
        const merged = new Uint32Array(cappedSize);
        let pos = 0;
        // Část před writeStart
        for (let k = 0; k < writeStart && pos < cappedSize; k++) merged[pos++] = workMem[k];
        // Insertované sloty
        for (let k = 0; k < slots.length && pos < cappedSize; k++) merged[pos++] = slots[k];
        // Část za writeStart (původní obsah, posunutý nahoru)
        for (let k = writeStart; k < currentSize && pos < cappedSize; k++) merged[pos++] = workMem[k];
        workMem = merged;
      }

      cell.memory = workMem;
      cell.energy = workMem.length;
      // PC zůstává numericky stejné. Modulárně omezit pro safety:
      if (cell.memory.length > 0) {
        cell.pc = cell.pc % cell.memory.length;
      } else {
        cell.pc = 0;
      }
    }
  }

  // Fáze 3: přepočet rate a layoutu
  recomputeAllLayouts();

  // Reset active outflow a override flagů
  for (let i = 0; i < total; i++) {
    const ao = cells[i].activeOutflow;
    ao[0] = 0; ao[1] = 0; ao[2] = 0; ao[3] = 0;
    const po = cells[i].pointerOverridden;
    po[0] = false; po[1] = false; po[2] = false; po[3] = false;
  }

  // Doplň budget pro další takt (per nová energie)
  refillTickBudgets();

  // DIAGNOSTIKA: total energie po kroku, varování při leakage
  let energyAfter = 0;
  for (let i = 0; i < total; i++) energyAfter += cells[i].energy;
  if (energyAfter !== energyBefore) {
    const delta = energyAfter - energyBefore;
    console.warn(`Tick ${state.step}: energie změněna o ${delta} (před: ${energyBefore}, po: ${energyAfter})`);
  }

  state.step += 1;
}

function stochasticFloor(value) {
  if (value <= 0) return 0;
  const whole = Math.floor(value);
  const frac = value - whole;
  return whole + (Math.random() < frac ? 1 : 0);
}

// applyCombinedLayout: po CPU fázi přepočítá pointery pro směry bez programátorského
// override, používá combined_rate (= natural rate + active outflow). Override directions
// si zachovají programátorovy hodnoty, jejich segment se ale stejně počítá z combined.
function applyCombinedLayout() {
  const cells = state.cells;
  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    const memSize = cell.memory.length;
    if (memSize === 0) continue;

    // Spočítat combined rate, proporčně klampnout
    const combined = new Array(DIRS);
    let totalRate = 0;
    for (let d = 0; d < DIRS; d++) {
      combined[d] = cell.rates[d] + cell.activeOutflow[d];
      totalRate += combined[d];
    }
    if (totalRate > memSize && totalRate > 0) {
      const scale = memSize / totalRate;
      let newTotal = 0;
      for (let d = 0; d < DIRS; d++) {
        combined[d] = Math.floor(combined[d] * scale);
        newTotal += combined[d];
      }
      let leftover = memSize - newTotal;
      while (leftover > 0) {
        let added = false;
        for (let d = 0; d < DIRS; d++) {
          if (combined[d] > 0 && leftover > 0) {
            combined[d] += 1;
            leftover -= 1;
            added = true;
            break;
          }
        }
        if (!added) break;
      }
    }

    // Layout od konce paměti: jen pro non-overridden směry
    // Override směry si nechají programátorovu hodnotu, ale neclaimují segment
    let cursor = memSize;
    for (const d of LAYOUT_ORDER_FROM_END) {
      if (!cell.pointerOverridden[d]) {
        cursor -= combined[d];
        cell.pointers[d] = Math.max(0, cursor);
      }
    }
    // cell.rates zůstává natural (passive), cell.activeOutflow zůstává active.
    // Phase 1 pak kombinuje sama.
  }
}

function recomputeAllLayouts() {
  const N = state.N;
  const cells = state.cells;
  const coeff = state.diffusionCoeff;
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const i = idx(x, y);
      const cell = cells[i];
      const myE = cell.energy;
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        const nE = cells[neighborIdx(x, y, d)].energy;
        const diff = myE - nE;
        const rate = diff > 0 ? stochasticFloor(diff * coeff) : 0;
        cell.rates[d] = rate;
        totalRate += rate;
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
}

// ===== Vizualizace =====

function setupImageData() {
  ctx.clearRect(0, 0, dom.canvas.width, dom.canvas.height);
  state.imageData = ctx.createImageData(state.N, state.N);
}

function render() {
  const N = state.N;
  const data = state.imageData.data;
  const viz = state.viz;

  if (viz === "warpaint" || viz === "identity") {
    // Identity: čistá barva podle tagu, brightness je binary (alive/dead)
    // War paint: hue z appearance, brightness z energie (jak roste/oslabuje)
    let maxE = 0;
    if (viz === "warpaint") {
      for (let y = 0; y < N; y++) {
        for (let x = 0; x < N; x++) {
          const e = state.cells[idx(x, y)].energy;
          if (e > maxE) maxE = e;
        }
      }
      if (maxE < 1) maxE = 1;
    }
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const cell = state.cells[idx(x, y)];
        const tag = (viz === "warpaint" ? cell.appearance : cell.originTag) >>> 0;
        const hue = (tag % 360);
        let sat, v;
        if (viz === "identity") {
          // Pure identity - barva podle tagu, alive nebo dead, žádný drift
          sat = 0.7;
          v = cell.energy > 0 ? 0.95 : 0;
        } else {
          // War paint - appearance + energie
          sat = (tag === 0) ? 0 : 0.7;  // paint=0 → šedá
          v = Math.sqrt(cell.energy / maxE);
        }
        const color = hsvToRgb(hue / 360, sat, v);
        const p = (y * N + x) * 4;
        data[p] = color[0];
        data[p+1] = color[1];
        data[p+2] = color[2];
        data[p+3] = 255;
      }
    }
  } else {
    // Standard auto-scale s inferno paletou
    let maxValue = 0;
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const v = cellRawValue(state.cells[idx(x, y)]);
        if (v > maxValue) maxValue = v;
      }
    }
    if (maxValue < 1) maxValue = 1;
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const cell = state.cells[idx(x, y)];
        const value = cellRawValue(cell) / maxValue;
        const color = colorForValue(value);
        const p = (y * N + x) * 4;
        data[p] = color[0];
        data[p+1] = color[1];
        data[p+2] = color[2];
        data[p+3] = 255;
      }
    }
  }
  const tmp = document.createElement("canvas");
  tmp.width = N;
  tmp.height = N;
  tmp.getContext("2d").putImageData(state.imageData, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, dom.canvas.width, dom.canvas.height);
  ctx.drawImage(tmp, 0, 0, dom.canvas.width, dom.canvas.height);

  if (dom.highlightPair.checked) {
    const cellSize = dom.canvas.width / N;
    const ax = parseInt(dom.aX.value, 10);
    const ay = parseInt(dom.aY.value, 10);
    const bx = parseInt(dom.bX.value, 10);
    const by = parseInt(dom.bY.value, 10);
    ctx.strokeStyle = "#00ff66";
    ctx.lineWidth = 2;
    ctx.strokeRect(ax * cellSize, ay * cellSize, cellSize, cellSize);
    ctx.strokeStyle = "#ff66cc";
    ctx.strokeRect(bx * cellSize, by * cellSize, cellSize, cellSize);
  }
}

function cellRawValue(cell) {
  if (state.viz === "energy") return cell.energy;
  if (state.viz === "memory_top") {
    if (cell.memory.length === 0) return 0;
    return cell.memory[cell.memory.length - 1] & 0xFF;
  }
  if (state.viz === "memory_bottom") {
    if (cell.memory.length === 0) return 0;
    return cell.memory[0] & 0xFF;
  }
  if (state.viz === "activity") return cell.activity;
  return 0;
}

function hsvToRgb(h, s, v) {
  // h, s, v in [0,1]; returns [r,g,b] in [0,255]
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  let r, g, b;
  switch (i % 6) {
    case 0: r = v; g = t; b = p; break;
    case 1: r = q; g = v; b = p; break;
    case 2: r = p; g = v; b = t; break;
    case 3: r = p; g = q; b = v; break;
    case 4: r = t; g = p; b = v; break;
    default: r = v; g = p; b = q;
  }
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
}

function colorForValue(v) {
  v = Math.max(0, Math.min(1, v));
  const t = Math.sqrt(v);
  const stops = [
    [0.00,  10,   5,  20],
    [0.25,  90,  20,  90],
    [0.50, 220,  60,  60],
    [0.75, 250, 160,  60],
    [1.00, 255, 240, 200],
  ];
  let i = 0;
  while (i < stops.length - 1 && t > stops[i + 1][0]) i++;
  const a = stops[i];
  const b = stops[Math.min(i + 1, stops.length - 1)];
  const span = b[0] - a[0];
  const lerp = span > 0 ? (t - a[0]) / span : 0;
  return [
    Math.round(a[1] + (b[1] - a[1]) * lerp),
    Math.round(a[2] + (b[2] - a[2]) * lerp),
    Math.round(a[3] + (b[3] - a[3]) * lerp),
  ];
}

// ===== Metriky =====

function updateMetrics() {
  let total = 0, mn = Infinity, mx = -Infinity;
  let count = state.cells.length;
  for (const c of state.cells) {
    total += c.energy;
    if (c.energy < mn) mn = c.energy;
    if (c.energy > mx) mx = c.energy;
  }
  const avg = total / count;
  let varSum = 0;
  for (const c of state.cells) {
    const d = c.energy - avg;
    varSum += d * d;
  }
  const variance = varSum / count;
  dom.step.textContent = state.step;
  dom.total.textContent = total.toFixed(0);
  dom.range.textContent = `${mn.toFixed(0)} / ${mx.toFixed(0)}`;
  dom.stats.textContent = `${avg.toFixed(2)} / ${variance.toFixed(2)}`;
}

// ===== Inspectors =====

function updateInspectors() {
  updateInspector("a", parseInt(dom.aX.value, 10), parseInt(dom.aY.value, 10));
  updateInspector("b", parseInt(dom.bX.value, 10), parseInt(dom.bY.value, 10));
  updateCommTrace();
  updateCpuStepLabels();
}

function updateCpuStepLabels() {
  const aIdx = idx(clamp(parseInt(dom.aX.value, 10), 0, state.N - 1),
                   clamp(parseInt(dom.aY.value, 10), 0, state.N - 1));
  const bIdx = idx(clamp(parseInt(dom.bX.value, 10), 0, state.N - 1),
                   clamp(parseInt(dom.bY.value, 10), 0, state.N - 1));
  const aBudget = state.cells[aIdx].tickBudget;
  const bBudget = state.cells[bIdx].tickBudget;
  dom.aCpuStepBtn.textContent = aBudget > 0 ? `CPU krok (${aBudget})` : "Tik světa";
  dom.bCpuStepBtn.textContent = bBudget > 0 ? `CPU krok (${bBudget})` : "Tik světa";
}

function updateInspector(prefix, x, y) {
  const N = state.N;
  x = clamp(x, 0, N - 1);
  y = clamp(y, 0, N - 1);
  const cell = state.cells[idx(x, y)];
  document.getElementById(prefix + "Coord").textContent = `(${x}, ${y})`;
  document.getElementById(prefix + "Energy").textContent = cell.energy;
  document.getElementById(prefix + "MemSize").textContent = cell.memory.length;
  document.getElementById(prefix + "CpuRate").textContent = Math.floor(cell.energy / state.cpuK);
  document.getElementById(prefix + "Pc").textContent = hexAddr(cell.pc);
  document.getElementById(prefix + "InstCount").textContent = cell.instCount;
  document.getElementById(prefix + "LastInst").textContent = formatLastInst(cell);
  document.getElementById(prefix + "Pointers").textContent =
    cell.pointers.map((p, i) => `${DIR_NAMES[i]}=${hexAddr(p)}`).join(", ");
  const rates = cell.rates.slice();
  document.getElementById(prefix + "Rates").textContent =
    rates.map((r, i) => `${DIR_NAMES[i]}=${r}`).join(", ") + ` (Σ=${rates.reduce((a,b)=>a+b,0)})`;
  document.getElementById(prefix + "Memory").innerHTML = formatMemory(cell);

  // Tag a appearance
  const tagEl = document.getElementById(prefix + "Tag");
  const paintEl = document.getElementById(prefix + "Paint");
  if (tagEl) {
    const tag = cell.originTag >>> 0;
    const tagColor = hsvToRgb((tag % 360) / 360, 0.7, 0.9);
    tagEl.textContent = "0x" + tag.toString(16).padStart(8, "0");
    tagEl.style.background = `rgb(${tagColor[0]},${tagColor[1]},${tagColor[2]})`;
    tagEl.style.color = "#000";
  }
  if (paintEl) {
    const paint = cell.appearance >>> 0;
    paintEl.textContent = "0x" + paint.toString(16).padStart(8, "0");
    if (paint === 0) {
      paintEl.style.background = "#444";
      paintEl.style.color = "#888";
    } else {
      const paintColor = hsvToRgb((paint % 360) / 360, 0.7, 0.9);
      paintEl.style.background = `rgb(${paintColor[0]},${paintColor[1]},${paintColor[2]})`;
      paintEl.style.color = "#000";
    }
  }

  // Track button label
  const trackBtn = document.getElementById(prefix + "TrackButton");
  if (trackBtn) {
    const trackingState = (prefix === "a") ? state.trackA : state.trackB;
    trackBtn.textContent = trackingState ? "Stop track" : "Track";
    trackBtn.classList.toggle("track-active", !!trackingState);
  }
}

function updateCommTrace() {
  if (state.lastCommAtoB) {
    dom.commAtoB.textContent = formatBytes(state.lastCommAtoB);
  } else {
    dom.commAtoB.textContent = "(A a B nesousedí, žádná přímá výměna)";
  }
  if (state.lastCommBtoA) {
    dom.commBtoA.textContent = formatBytes(state.lastCommBtoA);
  } else {
    dom.commBtoA.textContent = "";
  }
}

function formatBytes(arr) {
  if (arr.length === 0) return "(prázdné)";
  const max = 16;
  const parts = [];
  for (let i = 0; i < Math.min(arr.length, max); i++) parts.push(hexSlot(arr[i]));
  if (arr.length > max) parts.push(`... (+${arr.length - max})`);
  return `${arr.length} sl.: ${parts.join(" ")}`;
}

function formatLastInst(cell) {
  if (!cell.lastInst) return "-";
  const li = cell.lastInst;
  if (li.args.length === 0) return `@${hexAddr(li.addr)} ${li.name}`;
  if (li.args.length === 1) return `@${hexAddr(li.addr)} ${li.name} ${hexSlot(li.args[0])}`;
  return `@${hexAddr(li.addr)} ${li.name} ${hexSlot(li.args[0])}, ${hexSlot(li.args[1])}`;
}

function formatMemory(cell) {
  const mem = cell.memory;
  if (mem.length === 0) return "<em>(prázdná)</em>";
  const memSize = mem.length;
  const pcAddr = cell.pc % memSize;

  // Mapa: address -> array of direction indexes pointing here
  const ptrAt = {};
  cell.pointers.forEach((p, d) => {
    if (!ptrAt[p]) ptrAt[p] = [];
    ptrAt[p].push(d);
  });

  const lines = [];
  let i = 0;
  while (i < memSize) {
    const slot = mem[i];
    const opcode = slot & 0xFF;
    const op = OPCODES[opcode];
    let mnem, len, args;
    if (op) {
      mnem = op.name;
      len = op.len;
      args = [];
      for (let a = 1; a < op.len; a++) {
        args.push(i + a < memSize ? mem[i + a] : 0);
      }
    } else {
      mnem = ".slot";
      len = 1;
      args = [slot];
    }

    // PC v rozsahu této instrukce?
    const pcInRange = pcAddr >= i && pcAddr < i + len;
    const pcMarker = pcInRange
      ? (pcAddr === i ? "►" : "►?")  // ►? pokud PC je v middleinstrukce (po metempsychóze)
      : " ";

    // Pointery v rozsahu této instrukce
    const ptrsInRange = [];
    for (let p = i; p < i + len && p < memSize; p++) {
      if (ptrAt[p]) ptrsInRange.push(...ptrAt[p].map(d => DIR_NAMES[d]));
    }
    const ptrMarker = ptrsInRange.length > 0
      ? `<span class="disasm-ptr">[${ptrsInRange.join(",")}]</span>`
      : `<span class="disasm-ptr">    </span>`;

    // Argumenty: zobrazit special pro setp/getp/port (první arg = direction)
    let argStr = "";
    if (args.length > 0) {
      const argParts = args.map((v, ai) => {
        if (ai === 0 && (mnem === "setp" || mnem === "getp" || mnem === "port" ||
                         mnem === "senergy" || mnem === "setpv")) {
          const dirIdx = (v >>> 0) % DIRS;
          return `${DIR_NAMES[dirIdx]}`;
        }
        return `0x${(v >>> 0).toString(16).toUpperCase()}`;
      });
      argStr = " " + argParts.join(", ");
    }

    // Raw hex slotů
    const rawHex = [];
    for (let s = 0; s < len; s++) {
      if (i + s < memSize) rawHex.push(hexSlot(mem[i + s]));
    }
    const rawStr = rawHex.join(" ");

    let cls = "disasm-line";
    if (pcInRange) cls += " disasm-pc";
    if (ptrsInRange.length > 0) cls += " disasm-ptr-line";

    lines.push(
      `<span class="${cls}"><span class="disasm-pc-marker">${pcMarker}</span> <span class="addr">${hexAddr(i)}</span> ${ptrMarker} <span class="disasm-mnem">${mnem}${argStr}</span><span class="disasm-raw">  ; ${rawStr}</span></span>`
    );

    i += len;
  }
  return lines.join("\n");
}

function hexAddr(n) {
  const s = (n >>> 0).toString(16);
  if (s.length <= 4) return s.padStart(4, "0");
  return s.padStart(8, "0");
}

function hexSlot(n) {
  return (n >>> 0).toString(16).padStart(8, "0");
}

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

// ===== Inject =====

// Lookup tabulky pro asembler
const MNEMONIC_TO_OPCODE = (() => {
  const m = {};
  for (const [opcode, info] of Object.entries(OPCODES)) {
    m[info.name] = { opcode: parseInt(opcode), len: info.len };
  }
  return m;
})();

const DIR_NAME_TO_INDEX = { xp: 0, xn: 1, yp: 2, yn: 3 };

// Asembler: parsuje text s mnemoniky, labely a hex/decimal literály.
// Zpětně kompatibilní s prostými hex bajty (= raw slot literály).
function parseProgram(text) {
  // Strip comments
  const rawLines = text.split("\n").map(line => {
    const ci = line.indexOf(";");
    return (ci >= 0 ? line.slice(0, ci) : line).trim();
  }).filter(l => l.length > 0);

  // Tokenize
  const tokenizedLines = rawLines.map(line =>
    line.split(/[\s,]+/).filter(t => t.length > 0)
  );

  // Pass 1: collect labels and their slot positions
  const labels = {};
  let pos = 0;
  for (const tokens of tokenizedLines) {
    let i = 0;
    while (i < tokens.length) {
      const t = tokens[i];
      if (t.endsWith(":")) {
        labels[t.slice(0, -1)] = pos;
        i++;
        continue;
      }
      const mnemonic = t.toLowerCase();
      if (mnemonic in MNEMONIC_TO_OPCODE) {
        const len = MNEMONIC_TO_OPCODE[mnemonic].len;
        pos += len;
        i += len;
      } else {
        pos += 1;
        i++;
      }
    }
  }

  // Pass 2: emit slots
  const slots = [];
  const errors = [];
  for (const tokens of tokenizedLines) {
    let i = 0;
    while (i < tokens.length) {
      const t = tokens[i];
      if (t.endsWith(":")) { i++; continue; }
      const mnemonic = t.toLowerCase();
      if (mnemonic in MNEMONIC_TO_OPCODE) {
        const op = MNEMONIC_TO_OPCODE[mnemonic];
        slots.push(op.opcode);
        const argCount = op.len - 1;
        for (let a = 0; a < argCount; a++) {
          const argToken = tokens[i + 1 + a];
          if (argToken === undefined) {
            errors.push(`Chybí argument ${a + 1} pro ${mnemonic}`);
            slots.push(0);
          } else {
            slots.push(parseValue(argToken, labels, errors));
          }
        }
        i += op.len;
      } else {
        slots.push(parseValue(t, labels, errors));
        i++;
      }
    }
  }

  if (errors.length > 0) {
    console.warn("Asembler upozornění:", errors);
  }
  return slots;
}

function parseValue(token, labels, errors) {
  const lower = token.toLowerCase();
  if (lower in DIR_NAME_TO_INDEX) return DIR_NAME_TO_INDEX[lower];
  if (token in labels) return labels[token];
  if (token.startsWith("0x") || token.startsWith("0X")) {
    const v = parseInt(token, 16);
    if (!Number.isNaN(v)) return v >>> 0;
  }
  if (/^\d+$/.test(token)) return parseInt(token, 10) >>> 0;
  if (/^[0-9a-fA-F]+$/.test(token)) return parseInt(token, 16) >>> 0;
  errors.push(`Neznámý token: ${token}`);
  return 0;
}

// Zachovaný název - parseProgram zvládá oba formáty (hex i asembler)
const parseHexBytes = parseProgram;

function injectAt(x, y, text) {
  const slots = parseHexBytes(text);
  if (slots.length === 0) {
    alert("Žádné sloty k vložení.");
    return;
  }
  const cell = state.cells[idx(x, y)];
  let mem = cell.memory;
  if (mem.length < slots.length) {
    const newMem = new Uint32Array(Math.min(MAX_MEMORY, slots.length));
    for (let i = 0; i < mem.length; i++) newMem[i] = mem[i];
    mem = newMem;
    cell.memory = mem;
    cell.energy = mem.length;
  }
  for (let i = 0; i < slots.length && i < mem.length; i++) mem[i] = slots[i];
  cell.pc = 0;
  cell.lastInst = null;
  cell.instCount = 0;
  cell.tickBudget = Math.floor(cell.energy / state.cpuK);  // fresh budget pro nový program
  recomputeAllLayouts();
  render();
  updateMetrics();
  updateInspectors();
}

// ===== Loop =====

let animationFrame = null;
function tick() {
  if (!state.running) return;
  const spf = clamp(parseInt(dom.spf.value, 10), 1, 50);
  for (let s = 0; s < spf; s++) step();
  followLineage();
  render();
  updateMetrics();
  updateInspectors();
  animationFrame = requestAnimationFrame(tick);
}

// ===== Listenery =====

dom.run.addEventListener("click", () => {
  state.running = !state.running;
  dom.run.textContent = state.running ? "Pauza" : "Spustit";
  if (state.running) tick();
});
dom.stepBtn.addEventListener("click", () => {
  step();
  followLineage();
  render();
  updateMetrics();
  updateInspectors();
});
dom.reset.addEventListener("click", () => {
  state.running = false;
  dom.run.textContent = "Spustit";
  reset();
});
dom.diffusion.addEventListener("input", () => {
  state.diffusionCoeff = parseFloat(dom.diffusion.value);
  dom.diffusionVal.textContent = state.diffusionCoeff.toFixed(2);
  recomputeAllLayouts();
  updateInspectors();
});
dom.cpuK.addEventListener("change", () => {
  state.cpuK = clamp(parseInt(dom.cpuK.value, 10) || 1, 1, 64);
  dom.cpuK.value = state.cpuK;
});
dom.moveThreshold.addEventListener("input", () => {
  state.moveThreshold = parseFloat(dom.moveThreshold.value);
  dom.moveThresholdVal.textContent = state.moveThreshold.toFixed(1);
});
dom.viz.forEach(r => r.addEventListener("change", () => {
  if (r.checked) { state.viz = r.value; render(); }
}));
dom.highlightPair.addEventListener("change", render);

[dom.aX, dom.aY, dom.bX, dom.bY].forEach(el => {
  el.addEventListener("input", () => { updateInspectors(); render(); });
});

dom.aPreset.addEventListener("change", () => {
  const k = dom.aPreset.value;
  if (k && PRESETS[k]) dom.aInject.value = PRESETS[k];
});
dom.bPreset.addEventListener("change", () => {
  const k = dom.bPreset.value;
  if (k && PRESETS[k]) dom.bInject.value = PRESETS[k];
});
dom.aInjectBtn.addEventListener("click", () => {
  injectAt(parseInt(dom.aX.value, 10), parseInt(dom.aY.value, 10), dom.aInject.value);
});
dom.bInjectBtn.addEventListener("click", () => {
  injectAt(parseInt(dom.bX.value, 10), parseInt(dom.bY.value, 10), dom.bInject.value);
});

function cpuStepAt(x, y) {
  const N = state.N;
  x = clamp(x, 0, N - 1);
  y = clamp(y, 0, N - 1);
  const i = idx(x, y);
  const cell = state.cells[i];
  if (cell.tickBudget > 0) {
    if (cell.energy > 0 && cell.memory.length > 0) {
      executeOne(cell, i);
      cell.tickBudget -= 1;
    } else {
      cell.tickBudget = 0;
    }
    updateInspectors();
    render();
  } else {
    // Budget vyčerpán - dokončit zbytek taktu světa
    step();
    render();
    updateMetrics();
    updateInspectors();
  }
}

dom.aCpuStepBtn.addEventListener("click", () => {
  cpuStepAt(parseInt(dom.aX.value, 10), parseInt(dom.aY.value, 10));
});
dom.bCpuStepBtn.addEventListener("click", () => {
  cpuStepAt(parseInt(dom.bX.value, 10), parseInt(dom.bY.value, 10));
});

// Lineage tracking
function captureSnapshot(x, y) {
  const cell = state.cells[idx(x, y)];
  const len = Math.min(LINEAGE_SNAPSHOT_LEN, cell.memory.length);
  const snap = new Uint32Array(len);
  for (let k = 0; k < len; k++) snap[k] = cell.memory[k];
  return { snapshot: snap, len };
}

function findBestLineageMatch(track) {
  const snap = track.snapshot;
  const snapLen = track.len;
  if (snapLen === 0) return -1;
  let bestIdx = -1;
  let bestScore = -1;
  for (let i = 0; i < state.cells.length; i++) {
    const cell = state.cells[i];
    const memSize = cell.memory.length;
    if (memSize === 0) continue;
    let score = 0;
    for (let k = 0; k < snapLen && k < memSize; k++) {
      if (cell.memory[k] === snap[k]) score++;
    }
    if (score > bestScore) {
      bestScore = score;
      bestIdx = i;
    }
  }
  return bestIdx;
}

dom.aTrackBtn.addEventListener("click", () => {
  if (state.trackA) {
    state.trackA = null;
  } else {
    const x = parseInt(dom.aX.value, 10);
    const y = parseInt(dom.aY.value, 10);
    state.trackA = captureSnapshot(x, y);
  }
  updateInspectors();
});

dom.bTrackBtn.addEventListener("click", () => {
  if (state.trackB) {
    state.trackB = null;
  } else {
    const x = parseInt(dom.bX.value, 10);
    const y = parseInt(dom.bY.value, 10);
    state.trackB = captureSnapshot(x, y);
  }
  updateInspectors();
});

function followLineage() {
  if (state.trackA) {
    const idx = findBestLineageMatch(state.trackA);
    if (idx >= 0) {
      const x = idx % state.N;
      const y = Math.floor(idx / state.N);
      dom.aX.value = x;
      dom.aY.value = y;
    }
  }
  if (state.trackB) {
    const idx = findBestLineageMatch(state.trackB);
    if (idx >= 0) {
      const x = idx % state.N;
      const y = Math.floor(idx / state.N);
      dom.bX.value = x;
      dom.bY.value = y;
    }
  }
}

dom.canvas.addEventListener("click", (e) => {
  const rect = dom.canvas.getBoundingClientRect();
  const fx = (e.clientX - rect.left) / rect.width;
  const fy = (e.clientY - rect.top) / rect.height;
  const x = clamp(Math.floor(fx * state.N), 0, state.N - 1);
  const y = clamp(Math.floor(fy * state.N), 0, state.N - 1);
  if (e.shiftKey) {
    dom.aX.value = x;
    dom.aY.value = y;
  } else {
    dom.bX.value = x;
    dom.bY.value = y;
  }
  updateInspectors();
  render();
});

// ===== Start =====

state.diffusionCoeff = parseFloat(dom.diffusion.value);
dom.diffusionVal.textContent = state.diffusionCoeff.toFixed(2);
state.cpuK = clamp(parseInt(dom.cpuK.value, 10) || 1, 1, 64);
state.moveThreshold = parseFloat(dom.moveThreshold.value) || 2.0;
dom.moveThresholdVal.textContent = state.moveThreshold.toFixed(1);
reset();

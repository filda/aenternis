"use strict";

// ===== Konstanty =====

const DIRS = 6;
// 0=xp, 1=xn, 2=yp, 3=yn, 4=zp, 5=zn
const OPPOSITE = [1, 0, 3, 2, 5, 4];
const DIR_OFFSET = [
  [+1, 0, 0],
  [-1, 0, 0],
  [0, +1, 0],
  [0, -1, 0],
  [0, 0, +1],
  [0, 0, -1],
];
const DIR_NAMES = ["xp", "xn", "yp", "yn", "zp", "zn"];
// Layout pointerů od konce paměti dolů: pořadí v paměti je xp, xn, yp, yn, zp, zn
// Takže xp je nejníž (start radiačního bufferu), zn nejvýš
// Layout počítáme od konce: zn_ptr = mem_size - rate_zn, atd.
const LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0]; // zn, zp, yn, yp, xn, xp

// MAX_MEMORY je počet slotů, ne bajtů. Každý slot je 32-bit unsigned integer.
// Praktický cap pro prototyp - reálná hodnota záleží na tom, kolik RAM mu chceš dát.
// Pro perf testování zvyšujeme cap, aby se vešla "veškerá energie světa = N³"
// do jedné cely (Big Bang scenario). 16M slotů = 64MB per cela max.
const MAX_MEMORY = 16777216; // 16M = 2^24

// ===== VM instrukce =====
// Slot = 32-bit unsigned integer.
// Opcode = nejnižší bajt slotu (slot & 0xFF).
// Operandy = celé sloty (32-bit hodnoty), interpretované při použití jako adresa
// modulárně přes velikost paměti.
// Délka instrukce je v slotech, ne bajtech.

const OPCODES = {
  0x00: { name: "nop",   len: 1 }, // nedělej nic
  0x01: { name: "set",   len: 3 }, // mem[arg0] = arg1 (immediate)
  0x02: { name: "copy",  len: 3 }, // mem[arg0] = mem[arg1]
  0x03: { name: "add",   len: 3 }, // mem[arg0] += mem[arg1]
  0x04: { name: "sub",   len: 3 }, // mem[arg0] -= mem[arg1]
  0x05: { name: "inc",   len: 2 }, // mem[arg0] += 1
  0x06: { name: "dec",   len: 2 }, // mem[arg0] -= 1
  0x07: { name: "jmp",   len: 2 }, // PC = arg0
  0x08: { name: "jz",    len: 3 }, // if mem[arg0] == 0: PC = arg1
  0x09: { name: "setp",  len: 3 }, // pointers[arg0 % 6] = arg1
  0x0A: { name: "getp",  len: 3 }, // mem[arg1] = pointers[arg0 % 6]
  0x0B: { name: "port",  len: 3 }, // active outflow: posílá arg1 slotů ve směru arg0%6 nad rámec passive rate
};

// Instrukce, které jsou neznámé (>0x0B), se chovají jako nop (krok o 1 slot).
// Hustota smysluplných opcodů: aktuálně 12/256 = 4.7%.

// ===== DOM odkazy =====

const dom = {
  canvas: document.getElementById("worldCanvas"),
  step: document.getElementById("stepValue"),
  total: document.getElementById("totalEnergyMetric"),
  range: document.getElementById("rangeMetric"),
  stats: document.getElementById("statsMetric"),
  cellCount: document.getElementById("cellCountMetric"),
  totalSlots: document.getElementById("totalSlotsMetric"),
  memUsage: document.getElementById("memUsageMetric"),
  msPerTick: document.getElementById("msPerTickMetric"),
  fps: document.getElementById("fpsMetric"),
  bench: document.getElementById("benchButton"),
  benchResult: document.getElementById("benchResult"),
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
  axis: Array.from(document.querySelectorAll("input[name='axis']")),
  viz: Array.from(document.querySelectorAll("input[name='viz']")),
  slice: document.getElementById("sliceInput"),
  sliceVal: document.getElementById("sliceValue"),
  inspX: document.getElementById("inspX"),
  inspY: document.getElementById("inspY"),
  inspZ: document.getElementById("inspZ"),
  inspEnergy: document.getElementById("inspEnergy"),
  inspMemSize: document.getElementById("inspMemSize"),
  inspPointers: document.getElementById("inspPointers"),
  inspRates: document.getElementById("inspRates"),
  inspPc: document.getElementById("inspPc"),
  inspCredit: document.getElementById("inspCredit"),
  inspLastInst: document.getElementById("inspLastInst"),
  inspInstCount: document.getElementById("inspInstCount"),
  inspMemory: document.getElementById("inspMemory"),
  injectInput: document.getElementById("injectInput"),
  injectButton: document.getElementById("injectButton"),
  injectTailButton: document.getElementById("injectTailButton"),
  presetInput: document.getElementById("presetInput"),
};

// ===== Preset programy =====
// Klíč = ID v dropdownu, hodnota = string s hex bajty (mezerou)

const PRESETS = {
  counter: "05 10 07 00",
  // 0: 05 10  inc 0x10
  // 2: 07 00  jmp 0x00
  self_xp: "09 00 00 07 00",
  // 0: 09 00 00  setp xp(0), 0x00  -- xp_ptr ukazuje na vlastní program
  // 3: 07 00     jmp 0x00
  self_omni:
    "09 00 00 09 01 00 09 02 00 09 03 00 09 04 00 09 05 00 07 00",
  // 6× setp d, 0x00 (všechny pointery na začátek programu); jmp 0
  beacon:
    "05 20 09 00 00 07 00",
  // 0: 05 20     inc 0x20  -- počítadlo na adrese 0x20
  // 2: 09 00 00  setp xp, 0x00 -- broadcast self
  // 5: 07 00     jmp 0
  quine_core:
    "01 10 de 01 11 ad 01 12 be 01 13 ef 09 00 10 07 00",
  // 0: 01 10 de  set 0x10, 0xde
  // 3: 01 11 ad  set 0x11, 0xad
  // 6: 01 12 be  set 0x12, 0xbe
  // 9: 01 13 ef  set 0x13, 0xef
  // 12: 09 00 10 setp xp, 0x10 -- vyzařuj DEADBEEF
  // 15: 07 00    jmp 0
  deadbeef: "DE AD BE EF",
  projectile: "09 00 00 0b 00 20 07 00",
  // 0: 09 00 00  setp xp, 0x00 - pointer xp na začátek programu
  // 3: 0b 00 20  port xp, 0x20 - active write 32 slotů ve směru xp (silný projektil)
  // 6: 07 00     jmp 0
};

const ctx = dom.canvas.getContext("2d");

// ===== Stav simulace =====

const state = {
  N: 32,
  step: 0,
  running: false,
  axis: "z",
  slice: 16,
  viz: "energy",
  diffusionCoeff: 0.1,
  cpuK: 1,             // instrukce_za_takt = energie / K (K=1 zachovává compute=energie)
  cells: [],          // pole objektů { energy, memory, pointers, rates, pc, computeCredit, lastInst, instCount }
  imageData: null,
};

function idx(x, y, z) {
  return x + y * state.N + z * state.N * state.N;
}

function neighborIdx(x, y, z, d) {
  const N = state.N;
  const [dx, dy, dz] = DIR_OFFSET[d];
  const nx = (x + dx + N) % N;
  const ny = (y + dy + N) % N;
  const nz = (z + dz + N) % N;
  return idx(nx, ny, nz);
}

// ===== Inicializace =====

function makeEmptyCell() {
  return {
    energy: 0,
    memory: new Uint32Array(0),
    pointers: [0, 0, 0, 0, 0, 0],
    rates: [0, 0, 0, 0, 0, 0],
    activeOutflow: [0, 0, 0, 0, 0, 0],  // queue z opcode 'port', resetuje se na konci taktu
    pointerOverridden: [false, false, false, false, false, false],
    pc: 0,
    lastInst: null,
    instCount: 0,
    activity: 0,
  };
}

// "Smysluplná" instrukce = nenulový opcode v rozsahu 0x01-0x0A
function isMeaningfulInst(opcode) {
  return opcode >= 0x01 && opcode <= 0x0A;
}

const ACTIVITY_DECAY = 0.95; // každý takt se aktivita násobí tímto faktorem

function reset() {
  state.N = clamp(parseInt(dom.size.value, 10), 8, 100);
  perf.msPerTickRing = [];
  perf.fpsRing = [];
  perf.lastFrameTime = 0;
  state.slice = Math.floor(state.N / 2);
  dom.slice.max = state.N - 1;
  dom.slice.value = state.slice;
  dom.sliceVal.textContent = state.slice;
  // Inspector souřadnice klampnout
  dom.inspX.max = state.N - 1;
  dom.inspY.max = state.N - 1;
  dom.inspZ.max = state.N - 1;
  if (parseInt(dom.inspX.value, 10) >= state.N) dom.inspX.value = Math.floor(state.N / 2);
  if (parseInt(dom.inspY.value, 10) >= state.N) dom.inspY.value = Math.floor(state.N / 2);
  if (parseInt(dom.inspZ.value, 10) >= state.N) dom.inspZ.value = Math.floor(state.N / 2);

  state.step = 0;
  state.cells = new Array(state.N * state.N * state.N);
  for (let i = 0; i < state.cells.length; i++) {
    state.cells[i] = makeEmptyCell();
  }

  initScenario();
  // Spočítat počáteční rate a layout, aby první tick měl s čím pracovat
  recomputeAllLayouts();

  setupImageData();
  render();
  updateMetrics();
  updatePerfMetrics();
  updateInspector();
}

function initScenario() {
  const scenario = dom.scenario.value;
  const maxE = clamp(parseInt(dom.maxEnergy.value, 10), 1, MAX_MEMORY);
  const c = Math.floor(state.N / 2);

  if (scenario === "point") {
    // Big Bang: veškerá energie světa (= N³) v centrální cele
    const N = state.N;
    const totalE = N * N * N;
    setCellEnergy(c, c, c, totalE, randomBytes(totalE));
  } else if (scenario === "ball") {
    const r = Math.max(2, Math.floor(state.N / 6));
    for (let z = 0; z < state.N; z++)
      for (let y = 0; y < state.N; y++)
        for (let x = 0; x < state.N; x++) {
          const dx = x - c, dy = y - c, dz = z - c;
          const dist = Math.sqrt(dx*dx + dy*dy + dz*dz);
          if (dist <= r) {
            const e = Math.floor(maxE * (1 - dist / (r + 1)));
            if (e > 0) setCellEnergy(x, y, z, e, randomBytes(e));
          }
        }
  } else if (scenario === "random") {
    // Rozdělení s průměrem 1 per cela = total ≈ N³ (zákon zachování)
    for (let z = 0; z < state.N; z++)
      for (let y = 0; y < state.N; y++)
        for (let x = 0; x < state.N; x++) {
          const e = Math.floor(Math.random() * 3); // 0, 1, nebo 2 = avg 1
          if (e > 0) setCellEnergy(x, y, z, e, randomBytes(e));
        }
  } else if (scenario === "two_balls") {
    const r = Math.max(2, Math.floor(state.N / 8));
    const offset = Math.floor(state.N / 4);
    placeBall(c - offset, c, c, r, maxE, () => 0xAA);
    placeBall(c + offset, c, c, r, maxE, () => 0x55);
  }
}

function placeBall(cx, cy, cz, r, maxE, slotFn) {
  for (let z = 0; z < state.N; z++)
    for (let y = 0; y < state.N; y++)
      for (let x = 0; x < state.N; x++) {
        const dx = x - cx, dy = y - cy, dz = z - cz;
        const dist = Math.sqrt(dx*dx + dy*dy + dz*dz);
        if (dist <= r) {
          const e = Math.floor(maxE * (1 - dist / (r + 1)));
          if (e > 0) {
            const slots = new Uint32Array(e);
            for (let k = 0; k < e; k++) slots[k] = slotFn(k);
            setCellEnergy(x, y, z, e, slots);
          }
        }
      }
}

function setCellEnergy(x, y, z, energy, slots) {
  const cell = state.cells[idx(x, y, z)];
  cell.energy = energy;
  cell.memory = slots;
}

function randomSlots(n) {
  const a = new Uint32Array(n);
  for (let i = 0; i < n; i++) a[i] = Math.floor(Math.random() * 0x100000000) >>> 0;
  return a;
}

// alias pro zpětnou kompatibilitu v rámci souboru
const randomBytes = randomSlots;

// ===== CPU =====

function executeOne(cell) {
  // Vykoná jednu instrukci na pozici cell.pc, vrátí true pokud OK
  const mem = cell.memory;
  const memSize = mem.length;
  if (memSize === 0) return false;

  const pc = cell.pc % memSize;
  const slot = mem[pc];
  const opcode = slot & 0xFF;  // opcode = nejnižší bajt slotu
  const op = OPCODES[opcode];

  // Neznámý opcode = chování jako nop (jen krok o 1 slot)
  if (!op) {
    cell.pc = (pc + 1) % memSize;
    cell.lastInst = { opcode, name: "??", args: [], addr: pc };
    cell.instCount++;
    return true;
  }

  // Operandy = celé sloty (32-bit hodnoty), interpretace adresy je modulárně
  const arg0 = op.len > 1 ? mem[(pc + 1) % memSize] : 0;
  const arg1 = op.len > 2 ? mem[(pc + 2) % memSize] : 0;
  let nextPc = (pc + op.len) % memSize;

  // Pomocné funkce pro adresování modulárně
  const addr = (v) => v % memSize;
  // 32-bit aritmetika přetéká přes 2^32; >>> 0 zajistí unsigned
  const wrap32 = (v) => v >>> 0;

  switch (opcode) {
    case 0x00: break; // nop
    case 0x01: mem[addr(arg0)] = wrap32(arg1); break; // set
    case 0x02: mem[addr(arg0)] = mem[addr(arg1)]; break; // copy
    case 0x03: mem[addr(arg0)] = wrap32(mem[addr(arg0)] + mem[addr(arg1)]); break; // add
    case 0x04: mem[addr(arg0)] = wrap32(mem[addr(arg0)] - mem[addr(arg1)]); break; // sub
    case 0x05: mem[addr(arg0)] = wrap32(mem[addr(arg0)] + 1); break; // inc
    case 0x06: mem[addr(arg0)] = wrap32(mem[addr(arg0)] - 1); break; // dec
    case 0x07: nextPc = addr(arg0); break; // jmp
    case 0x08: // jz
      if (mem[addr(arg0)] === 0) nextPc = addr(arg1);
      break;
    case 0x09: { // setp dir, value
      const d = arg0 % DIRS;
      cell.pointers[d] = addr(arg1);
      cell.pointerOverridden[d] = true;
      break;
    }
    case 0x0A: // getp dir, addr
      mem[addr(arg1)] = cell.pointers[arg0 % DIRS];
      break;
    case 0x0B: { // port dir, intensity - active outflow nad rámec passive rate
      const dir = arg0 % DIRS;
      cell.activeOutflow[dir] = wrap32(cell.activeOutflow[dir] + arg1);
      break;
    }
  }

  cell.pc = nextPc;
  cell.lastInst = { opcode, name: op.name, args: op.len > 1 ? (op.len > 2 ? [arg0, arg1] : [arg0]) : [], addr: pc };
  cell.instCount++;
  if (isMeaningfulInst(opcode)) cell.activity += 1;
  return true;
}

function runCpuPhase() {
  const cells = state.cells;
  const K = state.cpuK;
  if (K <= 0) return;
  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    // Decay aktivity (i u buněk, co nic nedělají)
    cell.activity *= ACTIVITY_DECAY;
    if (cell.energy <= 0) continue;
    // Počet instrukcí je čistě funkce aktuální energie - žádný accumulator,
    // žádná paměť kontextu mezi takty. Pod K-tinou jednotek je cela inertní.
    const instructions = Math.floor(cell.energy / K);
    for (let j = 0; j < instructions; j++) {
      const ok = executeOne(cell);
      if (!ok) break;
    }
  }
}

// ===== Krok simulace =====

function step() {
  const N = state.N;
  const total = N * N * N;
  const cells = state.cells;

  // Fáze 0: CPU exekuce v každé buňce
  // Program může modifikovat paměť i pointery (ephemeral override).
  runCpuPhase();

  // Fáze 0.5: přepočet pointer layoutu pro tento takt s combined_rate
  applyCombinedLayout();

  // Fáze 1: snímek odtoku
  // Pro každou buňku spočítej kombinovaný rate (passive + active outflow z 'port' opcode),
  // proporčně klampni pokud převyšuje paměť, a vyzařuj zkopírované sloty.
  // outflows[i][d] = Uint32Array slotů; combinedRates[i][d] = počet pro shrinkage
  const outflows = new Array(total);
  const combinedRates = new Array(total);
  for (let i = 0; i < total; i++) {
    const cell = cells[i];
    const memSize = cell.memory.length;

    // Kombinace passive a active rate
    const rates = new Array(DIRS);
    let totalRate = 0;
    for (let d = 0; d < DIRS; d++) {
      rates[d] = cell.rates[d] + cell.activeOutflow[d];
      totalRate += rates[d];
    }
    // Proporční clamp pokud kombinovaný rate přesahuje paměť
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

    // Vyzařování (kopírování slotů) podle kombinovaného rate
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

  // Fáze 2: aplikace změn na každou buňku
  // - odtok = celkový součet kombinovaného rate (passive + active z 'port')
  // - přítok = sloty od šesti sousedů
  // - inflows přidat na konec v pevném pořadí xp, xn, yp, yn, zp, zn
  for (let z = 0; z < N; z++) {
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const i = idx(x, y, z);
        const cell = cells[i];
        const myRates = combinedRates[i];

        let totalOutflow = 0;
        for (let d = 0; d < DIRS; d++) totalOutflow += myRates[d];

        // Sebrat přítok ze šesti sousedů
        let totalInflow = 0;
        const inflows = new Array(DIRS);
        for (let d = 0; d < DIRS; d++) {
          const nIdx = neighborIdx(x, y, z, d);
          // soused mi posílá ve svém směru OPPOSITE[d] (tj. směr od souseda ke mně)
          const inBytes = outflows[nIdx][OPPOSITE[d]];
          inflows[d] = inBytes;
          totalInflow += inBytes.length;
        }

        // Sestavit novou paměť
        const oldMem = cell.memory;
        const oldSize = oldMem.length;
        const sizeAfterOutflow = Math.max(0, oldSize - totalOutflow);
        let newSize = sizeAfterOutflow + totalInflow;
        if (newSize > MAX_MEMORY) newSize = MAX_MEMORY;
        const newMem = new Uint32Array(newSize);

        // 1. zachovaná část staré paměti (od 0 do sizeAfterOutflow) = jádro
        const preserveLen = Math.min(sizeAfterOutflow, newSize);
        for (let k = 0; k < preserveLen; k++) newMem[k] = oldMem[k];

        // 2. přítoky v pevném pořadí, na konec = membrána
        let pos = preserveLen;
        for (let d = 0; d < DIRS && pos < newSize; d++) {
          const inSlots = inflows[d];
          for (let k = 0; k < inSlots.length && pos < newSize; k++) {
            newMem[pos++] = inSlots[k];
          }
        }

        cell.memory = newMem;
        cell.energy = newSize;
      }
    }
  }

  // Fáze 3: přepočet rate a layoutu pointerů pro další takt
  recomputeAllLayouts();

  // Reset active outflow a override flagů - 'port' a 'setp' platí jen pro aktuální takt
  for (let i = 0; i < total; i++) {
    const ao = cells[i].activeOutflow;
    ao[0] = 0; ao[1] = 0; ao[2] = 0; ao[3] = 0; ao[4] = 0; ao[5] = 0;
    const po = cells[i].pointerOverridden;
    po[0] = false; po[1] = false; po[2] = false; po[3] = false; po[4] = false; po[5] = false;
  }

  state.step += 1;
}

// applyCombinedLayout: po CPU fázi přepočítá pointery pro směry bez programátorského
// override, používá combined_rate (= natural rate + active outflow). Override směry
// si zachovají programátorovu hodnotu.
function applyCombinedLayout() {
  const cells = state.cells;
  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    const memSize = cell.memory.length;
    if (memSize === 0) continue;

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

    let cursor = memSize;
    for (const d of LAYOUT_ORDER_FROM_END) {
      if (!cell.pointerOverridden[d]) {
        cursor -= combined[d];
        cell.pointers[d] = Math.max(0, cursor);
      }
    }
  }
}

function stochasticFloor(value) {
  // Floor s pravděpodobnostním zaokrouhlením frakční části:
  // floor(2.7) = 2 s pravděpodobností 0.3, jinak 3.
  // Tím se i malé gradienty občas přenesou, místo aby zamrzly na 0.
  if (value <= 0) return 0;
  const whole = Math.floor(value);
  const frac = value - whole;
  return whole + (Math.random() < frac ? 1 : 0);
}

function recomputeAllLayouts() {
  const N = state.N;
  const cells = state.cells;
  const coeff = state.diffusionCoeff;
  for (let z = 0; z < N; z++) {
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const i = idx(x, y, z);
        const cell = cells[i];
        const myE = cell.energy;

        // Spočítat rate v každém směru z rozdílu potenciálů
        // Stochastický floor zajistí, že malé gradienty občas přenesou bajt
        let totalRate = 0;
        for (let d = 0; d < DIRS; d++) {
          const nIdx = neighborIdx(x, y, z, d);
          const nE = cells[nIdx].energy;
          const diff = myE - nE;
          const rate = diff > 0 ? stochasticFloor(diff * coeff) : 0;
          cell.rates[d] = rate;
          totalRate += rate;
        }

        // Pokud totalRate převýší aktuální paměť, škálovat proporčně
        // (ne prioritně po směrech - to způsobuje šachovnicové artefakty)
        const memSize = cell.memory.length;
        if (totalRate > memSize && totalRate > 0) {
          const scale = memSize / totalRate;
          let newTotal = 0;
          for (let d = 0; d < DIRS; d++) {
            cell.rates[d] = Math.floor(cell.rates[d] * scale);
            newTotal += cell.rates[d];
          }
          // Zbylé bajty (kvůli floor) přidat náhodně do směru, kde má smysl
          let leftover = memSize - newTotal;
          while (leftover > 0) {
            // přidat do prvního směru s nenulovým ratem (zachovat proporčnost)
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

        // Layout pointerů od konce paměti dolů
        let cursor = cell.memory.length;
        for (const d of LAYOUT_ORDER_FROM_END) {
          cursor -= cell.rates[d];
          cell.pointers[d] = Math.max(0, cursor);
        }
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

  // První průchod: najít maximum v řezu pro auto-škálu
  let maxValue = 0;
  for (let v = 0; v < N; v++) {
    for (let u = 0; u < N; u++) {
      const value = cellRawValue(sliceCell(u, v));
      if (value > maxValue) maxValue = value;
    }
  }
  if (maxValue < 1) maxValue = 1; // zabrát dělení nulou a tmavé scény

  // Druhý průchod: normalizace + render
  for (let v = 0; v < N; v++) {
    for (let u = 0; u < N; u++) {
      const cell = sliceCell(u, v);
      const raw = cellRawValue(cell);
      const value = raw / maxValue;
      const color = colorForValue(value);
      const p = (v * N + u) * 4;
      data[p] = color[0];
      data[p+1] = color[1];
      data[p+2] = color[2];
      data[p+3] = 255;
    }
  }

  // Nakreslit ImageData zvětšeně do canvasu
  const tmp = document.createElement("canvas");
  tmp.width = N;
  tmp.height = N;
  tmp.getContext("2d").putImageData(state.imageData, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, dom.canvas.width, dom.canvas.height);
  ctx.drawImage(tmp, 0, 0, dom.canvas.width, dom.canvas.height);

  // Označit pozici inspektoru
  const ix = parseInt(dom.inspX.value, 10);
  const iy = parseInt(dom.inspY.value, 10);
  const iz = parseInt(dom.inspZ.value, 10);
  if (sliceMatches(ix, iy, iz)) {
    const px = projU(ix, iy, iz);
    const pv = projV(ix, iy, iz);
    const sx = (px / N) * dom.canvas.width;
    const sy = (pv / N) * dom.canvas.height;
    const cellSize = dom.canvas.width / N;
    ctx.strokeStyle = "#ffcc00";
    ctx.lineWidth = 2;
    ctx.strokeRect(sx, sy, cellSize, cellSize);
  }
}

function sliceCell(u, v) {
  const s = state.slice;
  if (state.axis === "x") return state.cells[idx(s, u, v)];
  if (state.axis === "y") return state.cells[idx(u, s, v)];
  return state.cells[idx(u, v, s)];
}

function sliceMatches(x, y, z) {
  const s = state.slice;
  if (state.axis === "x") return x === s;
  if (state.axis === "y") return y === s;
  return z === s;
}

function projU(x, y, z) {
  if (state.axis === "x") return y;
  if (state.axis === "y") return x;
  return x;
}

function projV(x, y, z) {
  if (state.axis === "x") return z;
  if (state.axis === "y") return z;
  return y;
}

function cellRawValue(cell) {
  if (state.viz === "energy") {
    return cell.energy;
  } else if (state.viz === "memory_top") {
    if (cell.memory.length === 0) return 0;
    // Vizualizujeme nejnižší bajt slotu (= "opcode část"), aby se hodnoty
    // chovaly v rozsahu 0-255 a vidíme změny i u velkých čísel.
    return cell.memory[cell.memory.length - 1] & 0xFF;
  } else if (state.viz === "memory_bottom") {
    if (cell.memory.length === 0) return 0;
    return cell.memory[0] & 0xFF;
  } else if (state.viz === "activity") {
    return cell.activity;
  }
  return 0;
}

function colorForValue(v) {
  v = Math.max(0, Math.min(1, v));
  // jasnější paleta inspirovaná inferno: černá -> tmavě fialová -> červená -> oranžová -> žlutá -> bílá
  // sqrt brightening pro lepší viditelnost nízkých hodnot
  const t = Math.sqrt(v);

  // 5 stops: 0=black, 0.25=dark purple, 0.5=red, 0.75=orange, 1=light yellow
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
  const r = Math.round(a[1] + (b[1] - a[1]) * lerp);
  const g = Math.round(a[2] + (b[2] - a[2]) * lerp);
  const bl = Math.round(a[3] + (b[3] - a[3]) * lerp);
  return [r, g, bl];
}

// ===== Metriky =====

function updateMetrics() {
  const cells = state.cells;
  const count = cells.length;
  // Single pass for total + min + max. total = totalSlots (energy = memory length).
  let mn = Infinity, mx = -Infinity, total = 0;
  for (let i = 0; i < count; i++) {
    const e = cells[i].energy;
    total += e;
    if (e < mn) mn = e;
    if (e > mx) mx = e;
  }
  perf.lastTotalSlots = total;  // cache pro updatePerfMetrics
  const avg = total / count;
  // Variance: druhý průchod jen pokud je rozsah netriviální
  let varSum = 0;
  if (mx - mn > 0.5) {
    for (let i = 0; i < count; i++) {
      const d = cells[i].energy - avg;
      varSum += d * d;
    }
  }
  const variance = varSum / count;
  dom.step.textContent = state.step;
  dom.total.textContent = total.toFixed(0);
  dom.range.textContent = `${mn.toFixed(0)} / ${mx.toFixed(0)}`;
  dom.stats.textContent = `${avg.toFixed(2)} / ${variance.toFixed(2)}`;
}

// ===== Inspector =====

function updateInspector() {
  const x = clamp(parseInt(dom.inspX.value, 10), 0, state.N - 1);
  const y = clamp(parseInt(dom.inspY.value, 10), 0, state.N - 1);
  const z = clamp(parseInt(dom.inspZ.value, 10), 0, state.N - 1);
  const cell = state.cells[idx(x, y, z)];

  dom.inspEnergy.textContent = cell.energy;
  dom.inspMemSize.textContent = cell.memory.length;
  dom.inspPointers.textContent = cell.pointers.map((p, i) => `${DIR_NAMES[i]}=${hexAddr(p)}`).join(", ");
  const rates = cell.rates.slice();
  dom.inspRates.textContent = rates.map((r, i) => `${DIR_NAMES[i]}=${r}`).join(", ") + ` (Σ=${rates.reduce((a,b)=>a+b,0)})`;
  dom.inspPc.textContent = hexAddr(cell.pc);
  dom.inspCredit.textContent = Math.floor(cell.energy / state.cpuK) + " /takt";
  dom.inspLastInst.textContent = formatLastInst(cell);
  dom.inspInstCount.textContent = cell.instCount;

  dom.inspMemory.innerHTML = formatMemory(cell);
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
  const ptrs = new Set(cell.pointers);
  // Per řádek: 8 slotů. Cap na 256 slotů (32 řádků), pro velké cely "..."
  const PER_LINE = 8;
  const DISPLAY_LIMIT = 256;
  const displayCount = Math.min(memSize, DISPLAY_LIMIT);
  const lines = [];
  let line = [];
  let lineStart = 0;
  for (let i = 0; i < displayCount; i++) {
    if (i % PER_LINE === 0 && line.length > 0) {
      lines.push(`<span class="addr">${hexAddr(lineStart)}:</span> ${line.join(" ")}`);
      line = [];
      lineStart = i;
    }
    const isPtr = ptrs.has(i);
    const isPc = i === pcAddr;
    let cls = "memory-slot";
    if (isPc && isPtr) cls += " memory-pc-and-pointer";
    else if (isPc) cls += " memory-pc";
    else if (isPtr) cls += " memory-pointer-marker";
    line.push(`<span class="${cls}">${hexSlot(mem[i])}</span>`);
  }
  if (line.length > 0) lines.push(`<span class="addr">${hexAddr(lineStart)}:</span> ${line.join(" ")}`);
  if (memSize > DISPLAY_LIMIT) {
    lines.push(`<em>... (+${(memSize - DISPLAY_LIMIT).toLocaleString()} dalších slotů)</em>`);
  }
  return lines.join("\n");
}

// hex pro adresu/pointer - dynamicky 4-8 znaků podle hodnoty
function hexAddr(n) {
  const s = (n >>> 0).toString(16);
  if (s.length <= 4) return s.padStart(4, "0");
  return s.padStart(8, "0");
}

// hex pro hodnotu slotu - vždy 8 znaků (32-bit)
function hexSlot(n) {
  return (n >>> 0).toString(16).padStart(8, "0");
}

function hex(n) {
  return n.toString(16).padStart(2, "0");
}

function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

// ===== Loop =====

// ===== Performance measurement =====
const perf = {
  msPerTickRing: [],
  ringSize: 30,
  fpsRing: [],
  lastFrameTime: 0,
  frameCount: 0,
  lastTotalSlots: 0,  // cache - updatováno v updateMetrics, čteno v updatePerfMetrics
};

function recordTickTime(ms) {
  perf.msPerTickRing.push(ms);
  if (perf.msPerTickRing.length > perf.ringSize) perf.msPerTickRing.shift();
}

function recordFrameTime(now) {
  if (perf.lastFrameTime > 0) {
    const dt = now - perf.lastFrameTime;
    perf.fpsRing.push(1000 / dt);
    if (perf.fpsRing.length > perf.ringSize) perf.fpsRing.shift();
  }
  perf.lastFrameTime = now;
}

function avgRing(ring) {
  if (ring.length === 0) return 0;
  let sum = 0;
  for (const v of ring) sum += v;
  return sum / ring.length;
}

function updatePerfMetrics() {
  // totalSlots = sum of cell.memory.length = sum of cell.energy (= total energy).
  // Tj. už víme z updateMetrics. Optimalizace: nebudeme to počítat znova,
  // místo toho cacheneme v perf.lastTotalSlots
  const N = state.N;
  const total = N * N * N;
  const totalSlots = perf.lastTotalSlots;
  const memMB = (totalSlots * 4 + total * 200) / (1024 * 1024);

  if (dom.cellCount) dom.cellCount.textContent = total.toLocaleString();
  if (dom.totalSlots) dom.totalSlots.textContent = totalSlots.toLocaleString();
  if (dom.memUsage) dom.memUsage.textContent = memMB.toFixed(1) + " MB";
  if (dom.msPerTick) dom.msPerTick.textContent = avgRing(perf.msPerTickRing).toFixed(2);
  if (dom.fps) dom.fps.textContent = avgRing(perf.fpsRing).toFixed(1);
}

let animationFrame = null;
let frameCounter = 0;
function tick() {
  if (!state.running) return;
  const spf = clamp(parseInt(dom.spf.value, 10), 1, 50);
  const t0 = performance.now();
  for (let s = 0; s < spf; s++) step();
  const t1 = performance.now();
  recordTickTime((t1 - t0) / spf);
  render();
  recordFrameTime(performance.now());
  // Sampled UI: metriky a inspector každý 4. frame (cca 4-5× vyšší FPS)
  frameCounter++;
  if (frameCounter % 4 === 0) {
    updateMetrics();
    updatePerfMetrics();
    updateInspector();
  }
  animationFrame = requestAnimationFrame(tick);
}

function runBenchmark() {
  const NUM_TICKS = 100;
  if (dom.benchResult) dom.benchResult.textContent = "Běží...";
  // Setting timeout to allow UI update
  setTimeout(() => {
    const t0 = performance.now();
    for (let i = 0; i < NUM_TICKS; i++) step();
    const t1 = performance.now();
    const totalMs = t1 - t0;
    const msPerTick = totalMs / NUM_TICKS;
    const N = state.N;
    const total = N * N * N;
    const slotsPerSec = (total / (msPerTick / 1000)).toFixed(0);
    if (dom.benchResult) {
      dom.benchResult.textContent =
        `${NUM_TICKS} tiků za ${totalMs.toFixed(0)} ms ` +
        `(${msPerTick.toFixed(2)} ms/tik, ` +
        `${parseInt(slotsPerSec).toLocaleString()} cel/s)`;
    }
    render();
    updateMetrics();
    updatePerfMetrics();
    updateInspector();
  }, 50);
}

// ===== Listenery =====

dom.run.addEventListener("click", () => {
  state.running = !state.running;
  dom.run.textContent = state.running ? "Pauza" : "Spustit";
  if (state.running) tick();
});
dom.stepBtn.addEventListener("click", () => {
  const t0 = performance.now();
  step();
  const t1 = performance.now();
  recordTickTime(t1 - t0);
  render();
  updateMetrics();
  updatePerfMetrics();
  updateInspector();
});
if (dom.bench) dom.bench.addEventListener("click", runBenchmark);
dom.reset.addEventListener("click", () => {
  state.running = false;
  dom.run.textContent = "Spustit";
  reset();
});
dom.size.addEventListener("change", () => { /* aplikuje se až při Resetu */ });
dom.scenario.addEventListener("change", () => { /* aplikuje se až při Resetu */ });
dom.maxEnergy.addEventListener("change", () => { /* aplikuje se až při Resetu */ });
dom.diffusion.addEventListener("input", () => {
  state.diffusionCoeff = parseFloat(dom.diffusion.value);
  dom.diffusionVal.textContent = state.diffusionCoeff.toFixed(2);
  recomputeAllLayouts();
  updateInspector();
});
dom.cpuK.addEventListener("change", () => {
  state.cpuK = clamp(parseInt(dom.cpuK.value, 10) || 16, 1, 64);
  dom.cpuK.value = state.cpuK;
});
dom.injectButton.addEventListener("click", () => {
  const x = clamp(parseInt(dom.inspX.value, 10), 0, state.N - 1);
  const y = clamp(parseInt(dom.inspY.value, 10), 0, state.N - 1);
  const z = clamp(parseInt(dom.inspZ.value, 10), 0, state.N - 1);
  const cell = state.cells[idx(x, y, z)];
  const slots = parseHexBytes(dom.injectInput.value);
  if (slots.length === 0) {
    alert("Žádné sloty k vložení. Zadej hex hodnoty oddělené mezerou nebo čárkou.");
    return;
  }
  // Pokud je paměť kratší než program, doplnit na minimum
  let mem = cell.memory;
  if (mem.length < slots.length) {
    const newMem = new Uint32Array(Math.min(MAX_MEMORY, slots.length));
    for (let i = 0; i < mem.length; i++) newMem[i] = mem[i];
    mem = newMem;
    cell.memory = mem;
    cell.energy = mem.length;
  }
  // Vložit program na začátek
  for (let i = 0; i < slots.length && i < mem.length; i++) mem[i] = slots[i];
  cell.pc = 0;
  cell.lastInst = null;
  cell.instCount = 0;
  recomputeAllLayouts();
  render();
  updateMetrics();
  updateInspector();
});

function parseHexBytes(text) {
  // Vrací pole 32-bit slot hodnot. Zachovaný název kvůli historii.
  const tokens = text.split(/[\s,]+/).filter(t => t.length > 0);
  const slots = [];
  for (const t of tokens) {
    const v = parseInt(t, 16);
    if (!Number.isNaN(v)) slots.push(v >>> 0);  // unsigned 32-bit
  }
  return slots;
}

dom.presetInput.addEventListener("change", () => {
  const key = dom.presetInput.value;
  if (key && PRESETS[key]) {
    dom.injectInput.value = PRESETS[key];
  }
});

dom.injectTailButton.addEventListener("click", () => {
  const x = clamp(parseInt(dom.inspX.value, 10), 0, state.N - 1);
  const y = clamp(parseInt(dom.inspY.value, 10), 0, state.N - 1);
  const z = clamp(parseInt(dom.inspZ.value, 10), 0, state.N - 1);
  const cell = state.cells[idx(x, y, z)];
  const slots = parseHexBytes(dom.injectInput.value);
  if (slots.length === 0) {
    alert("Žádné sloty k vložení.");
    return;
  }
  // Připíše sloty na konec paměti (rozšíří kapacitu, posune energii)
  const oldMem = cell.memory;
  const newSize = Math.min(MAX_MEMORY, oldMem.length + slots.length);
  const newMem = new Uint32Array(newSize);
  for (let i = 0; i < oldMem.length && i < newSize; i++) newMem[i] = oldMem[i];
  let pos = oldMem.length;
  for (let i = 0; i < slots.length && pos < newSize; i++) newMem[pos++] = slots[i];
  cell.memory = newMem;
  cell.energy = newSize;
  recomputeAllLayouts();
  render();
  updateMetrics();
  updateInspector();
});

dom.axis.forEach(r => r.addEventListener("change", () => {
  if (r.checked) {
    state.axis = r.value;
    render();
  }
}));
dom.viz.forEach(r => r.addEventListener("change", () => {
  if (r.checked) {
    state.viz = r.value;
    render();
  }
}));
dom.slice.addEventListener("input", () => {
  state.slice = parseInt(dom.slice.value, 10);
  dom.sliceVal.textContent = state.slice;
  render();
});

[dom.inspX, dom.inspY, dom.inspZ].forEach(el => {
  el.addEventListener("input", () => { updateInspector(); render(); });
});

dom.canvas.addEventListener("click", (e) => {
  const rect = dom.canvas.getBoundingClientRect();
  const fx = (e.clientX - rect.left) / rect.width;
  const fy = (e.clientY - rect.top) / rect.height;
  const u = clamp(Math.floor(fx * state.N), 0, state.N - 1);
  const v = clamp(Math.floor(fy * state.N), 0, state.N - 1);
  // Nastavit inspector souřadnice podle aktuální osy řezu
  if (state.axis === "x") {
    dom.inspX.value = state.slice;
    dom.inspY.value = u;
    dom.inspZ.value = v;
  } else if (state.axis === "y") {
    dom.inspX.value = u;
    dom.inspY.value = state.slice;
    dom.inspZ.value = v;
  } else {
    dom.inspX.value = u;
    dom.inspY.value = v;
    dom.inspZ.value = state.slice;
  }
  updateInspector();
  render();
});

// ===== Start =====

state.diffusionCoeff = parseFloat(dom.diffusion.value);
dom.diffusionVal.textContent = state.diffusionCoeff.toFixed(2);
state.cpuK = clamp(parseInt(dom.cpuK.value, 10) || 16, 1, 64);
reset();

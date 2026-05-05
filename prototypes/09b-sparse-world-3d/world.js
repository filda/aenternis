"use strict";

// ===== Sparse svět (3D) =====
//
// Centrální invariant: buňka existuje právě tehdy, když má nenulovou energii.
// Počet buněk světa <= E_total.
//
// Datová struktura: Map<bigint coord, Cell>, kde coord je zabalené (x, y, z) do
// bigint klíče. Souřadnice jsou 32-bit signed per osu (záporné jsou potřeba —
// svět expanduje všemi směry od (0, 0, 0)).
//
// Fyzika přejatá z prototypu 9 (2D sparse). Jediný rozdíl je dimenze: 6 směrů
// místo 4. Žádné nové opcody, žádná nová pravidla. Výchozí kamera, GC, big bang
// a comparison harness se škálují na 3D triviálně.

// ----- Konstanty -----

const DIRS = 6;
// 0=xp, 1=xn, 2=yp, 3=yn, 4=zp, 5=zn
const OPPOSITE = [1, 0, 3, 2, 5, 4];
const DIR_OFFSET = [
  [+1,  0,  0],
  [-1,  0,  0],
  [ 0, +1,  0],
  [ 0, -1,  0],
  [ 0,  0, +1],
  [ 0,  0, -1],
];
const DIR_NAMES = ["xp", "xn", "yp", "yn", "zp", "zn"];
const LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0]; // zn, zp, yn, yp, xn, xp

// Praktický cap na velikost paměti jedné buňky. Sparse svět ho dosáhne
// jen pokud do jediné buňky natlačí celé E_total — což je počáteční stav
// big bangu. Po prvním ticku je energie rozprostřená a cap je bezpečně mimo.
const MAX_MEMORY = 65536;

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
  0x09: { name: "setp",    len: 3 },
  0x0A: { name: "getp",    len: 3 },
  0x0B: { name: "port",    len: 3 },
  0x0C: { name: "senergy", len: 3 },
  0x0D: { name: "jne",     len: 3 },
  0x0E: { name: "je",      len: 4 },
  0x0F: { name: "ldi",     len: 3 },
  0x10: { name: "sti",     len: 3 },
  0x11: { name: "setpv",   len: 3 },
  0x12: { name: "sid",     len: 2 },
  0x13: { name: "paint",   len: 2 },
};

// ----- Coord packing -----
//
// Souřadnice jsou 32-bit signed integer per osu. Pro Map klíč zabalíme do
// bigint: bity [64..96) = x, [32..64) = y, [0..32) = z. Záporné se masknou
// na uint32 reprezentaci, aby BigInt operace byly konzistentní.

const MASK32 = 0xFFFFFFFFn;

function packCoord(x, y, z) {
  return ((BigInt(x | 0) & MASK32) << 64n)
       | ((BigInt(y | 0) & MASK32) << 32n)
       |  (BigInt(z | 0) & MASK32);
}

function unpackCoord(key) {
  // Reverze packCoord. Sign extend přes (Number ... | 0).
  const xRaw = Number((key >> 64n) & MASK32) | 0;
  const yRaw = Number((key >> 32n) & MASK32) | 0;
  const zRaw = Number(key & MASK32) | 0;
  return [xRaw, yRaw, zRaw];
}

// ----- Deterministická náhoda -----
//
// xorshift32 — stačí pro reprodukovatelnost experimentů. Stochastic floor
// a iniciální šum používají tento generátor, ne Math.random(), aby seed
// určoval celý průběh.

function makeRng(seed) {
  let s = (seed >>> 0) || 1;
  return function nextU32() {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s;
  };
}

function rngFloat(rng) {
  return rng() / 0x100000000;
}

// Seed per buňku pomocí hashe souřadnic + globálního worldSeed. Tím má
// každá buňka deterministický termální mikrostav bez ohledu na pořadí
// iterace, což je klíčové pro fair ekvivalenci se toroidem.
function cellSeed(worldSeed, x, y, z) {
  let h = (worldSeed >>> 0) ^ 0x9E3779B9;
  h = ((h + (x | 0)) * 374761393) >>> 0;
  h ^= h >>> 13;
  h = ((h + (y | 0)) * 668265263) >>> 0;
  h ^= h >>> 16;
  h = ((h + (z | 0)) * 1274126177) >>> 0;
  h ^= h >>> 13;
  if (h === 0) h = 1; // xorshift32 nesmí být 0
  return h;
}

// Seed kombinující coord + tick. Tím dostane každá buňka fresh rng pro
// každý tick — žádná závislost na životním cyklu (alive/dead/realloc).
// Sparse i toroid implementace produkují stejnou sekvenci stochasticFloor
// hodnot per (coord, tick), bez ohledu na to, jestli buňka existovala
// kontinuálně nebo byla GC'd a re-alokovaná.
function cellTickSeed(worldSeed, x, y, z, tick) {
  let h = cellSeed(worldSeed, x, y, z);
  h = ((h + (tick | 0)) * 2246822507) >>> 0;
  h ^= h >>> 16;
  if (h === 0) h = 1;
  return h;
}

// ----- Cell -----

function makeCell(x, y, z, worldSeed) {
  const seed = cellSeed(worldSeed, x, y, z);
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
    // originTag deterministicky z coord+seed, takže sparse i toroid mají
    // stejnou hodnotu pro stejnou pozici. Aktivní část termálního stavu
    // je rng — drží se per buňka, aby pořadí iterace nemělo vliv.
    originTag: seed,
    appearance: 0,
    rng: makeRng(seed),
  };
}

// ----- SparseWorld -----

class SparseWorld {
  constructor(opts = {}) {
    this.cells = new Map(); // bigint coord -> Cell
    this.diffusionCoeff = opts.diffusionCoeff ?? 0.15;
    this.cpuK = opts.cpuK ?? 1;
    this.moveThreshold = opts.moveThreshold ?? 2.0;
    this.worldSeed = (opts.seed ?? 1) >>> 0;
    this.tick = 0;
    // E_total se neukládá jako parametr — kdykoli ho lze spočítat ze sumy.
    // Drží se ale výchozí hodnota pro asserce a UI.
    this.initialETotal = 0;
  }

  // ----- Lookup / alokace -----

  getCell(x, y, z) {
    return this.cells.get(packCoord(x, y, z));
  }

  hasCell(x, y, z) {
    return this.cells.has(packCoord(x, y, z));
  }

  // Alokace nové buňky s prázdnou pamětí (E = 0). Volá se z inflow fáze
  // při prvním zápisu do dosud neexistující pozice.
  allocateCell(x, y, z) {
    const cell = makeCell(x, y, z, this.worldSeed);
    this.cells.set(packCoord(x, y, z), cell);
    return cell;
  }

  getOrCreate(x, y, z) {
    const key = packCoord(x, y, z);
    let cell = this.cells.get(key);
    if (!cell) {
      cell = makeCell(x, y, z, this.worldSeed);
      this.cells.set(key, cell);
    }
    return cell;
  }

  // ----- Inicializace -----

  // Big bang: jedna buňka v (0, 0, 0) drží celé E_total. Volitelný program
  // se vloží na začátek paměti, zbytek se naplní šumem z buňčiného RNG
  // (které je seedované hashí coord+worldSeed, takže různé worldSeed
  // dávají různé počáteční šumy, ale všechno je deterministické).
  bigBang(eTotal, programSlots = []) {
    this.cells.clear();
    this.tick = 0;
    this.initialETotal = eTotal;
    if (eTotal <= 0) return;

    const cell = this.allocateCell(0, 0, 0);
    cell.energy = eTotal;
    const mem = new Uint32Array(eTotal);
    for (let i = 0; i < eTotal; i++) {
      mem[i] = i < programSlots.length ? (programSlots[i] >>> 0) : (cell.rng() >>> 0);
    }
    cell.memory = mem;
    cell.tickBudget = Math.floor(cell.energy / this.cpuK);
    this.recomputeAllLayouts();
  }

  // ----- Diagnostika -----

  totalEnergy() {
    let sum = 0;
    for (const cell of this.cells.values()) sum += cell.energy;
    return sum;
  }

  size() {
    return this.cells.size;
  }

  boundingBox() {
    if (this.cells.size === 0) return null;
    let xMin = Infinity, xMax = -Infinity;
    let yMin = Infinity, yMax = -Infinity;
    let zMin = Infinity, zMax = -Infinity;
    for (const cell of this.cells.values()) {
      if (cell.x < xMin) xMin = cell.x;
      if (cell.x > xMax) xMax = cell.x;
      if (cell.y < yMin) yMin = cell.y;
      if (cell.y > yMax) yMax = cell.y;
      if (cell.z < zMin) zMin = cell.z;
      if (cell.z > zMax) zMax = cell.z;
    }
    return { xMin, xMax, yMin, yMax, zMin, zMax };
  }

  // Energetické těžiště — pro default kameru.
  centroid() {
    if (this.cells.size === 0) return null;
    let sumE = 0, sumX = 0, sumY = 0, sumZ = 0;
    for (const cell of this.cells.values()) {
      sumE += cell.energy;
      sumX += cell.energy * cell.x;
      sumY += cell.energy * cell.y;
      sumZ += cell.energy * cell.z;
    }
    if (sumE === 0) return null;
    return { x: sumX / sumE, y: sumY / sumE, z: sumZ / sumE };
  }

  // ----- CPU fáze (přejato z prototypu 9) -----

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
        const [dx, dy, dz] = DIR_OFFSET[d];
        const neighbor = this.getCell(cell.x + dx, cell.y + dy, cell.z + dz);
        // V sparse světě neexistující soused = energie 0.
        mem[addr(arg1)] = wrap32(neighbor ? neighbor.energy : 0);
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
    const argList = op.len > 1
      ? (op.len > 3 ? [arg0, arg1, arg2] : (op.len > 2 ? [arg0, arg1] : [arg0]))
      : [];
    cell.lastInst = { opcode, name: op.name, args: argList, addr: pc };
    cell.instCount++;
    return true;
  }

  runCpuPhase() {
    if (this.cpuK <= 0) return;
    for (const cell of this.cells.values()) {
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
    for (const cell of this.cells.values()) {
      cell.tickBudget = cell.energy > 0 ? Math.floor(cell.energy / K) : 0;
    }
  }

  // ----- Layout -----
  //
  // recomputeAllLayouts: spočítá natural rate pro každou buňku z rozdílu
  // potenciálů a rozloží pointery od konce paměti. Sousedi jsou hledáni
  // přes getCell — neexistující soused má E = 0.

  // stochasticFloor s lokálním rng — každá buňka dostane fresh rng při
  // vstupu do recompute, takže pořadí iterace ani životní cyklus buňky
  // nemá vliv na výsledek.
  stochasticFloorRng(rng, value) {
    if (value <= 0) return 0;
    const whole = Math.floor(value);
    const frac = value - whole;
    return whole + (rngFloat(rng) < frac ? 1 : 0);
  }

  recomputeAllLayouts() {
    const coeff = this.diffusionCoeff;
    for (const cell of this.cells.values()) {
      // Fresh rng per (coord, tick) — viz cellTickSeed komentář.
      const rng = makeRng(cellTickSeed(this.worldSeed, cell.x, cell.y, cell.z, this.tick));
      const myE = cell.energy;
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        const [dx, dy, dz] = DIR_OFFSET[d];
        const neighbor = this.getCell(cell.x + dx, cell.y + dy, cell.z + dz);
        const nE = neighbor ? neighbor.energy : 0;
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

  applyCombinedLayout() {
    for (const cell of this.cells.values()) {
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

  // ----- Step -----
  //
  // Pořadí fází (z `mechanics.md` rozšířené o GC krok):
  // 1. CPU
  // 2. Sub-tick reflow s combined_rate
  // 3. Outflow snapshot (per buňka, per směr — sloty zkopírované z pointeru)
  // 4. Inflow + alokace (zápis do existujících i nově vzniklých buněk)
  // 5. Reset active outflow + override flagů
  // 6. GC: smaž buňky s E = 0
  // 7. Layout pro další tick

  step() {
    // Fáze 1
    this.runCpuPhase();

    // Fáze 2
    this.applyCombinedLayout();

    // Fáze 3: outflow snapshot
    // Pro každou buňku spočítáme combined rate (s clampem), zkopírujeme
    // sloty z pointer pozice. Zapamatujeme si i target pozice neexistujících
    // sousedů — ty se v inflow fázi alokují.
    const outflows = []; // [{ srcCell, dir, slots }]
    const combinedRatesByCell = new Map(); // bigint -> [DIRS]
    for (const cell of this.cells.values()) {
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
      combinedRatesByCell.set(packCoord(cell.x, cell.y, cell.z), rates);

      for (let d = 0; d < DIRS; d++) {
        const rate = rates[d];
        if (rate === 0) continue;
        const ptr = cell.pointers[d];
        const slots = new Uint32Array(rate);
        if (memSize > 0) {
          for (let k = 0; k < rate; k++) {
            slots[k] = cell.memory[(ptr + k) % memSize];
          }
        }
        outflows.push({ srcCell: cell, dir: d, slots });
      }
    }

    // Fáze 4: inflow + alokace + dominance/intrusion mixing.
    //
    // Sběr inflows per cílová buňka. Cílová buňka může v tomhle ticku ještě
    // neexistovat — pak se alokuje s prázdnou pamětí. Po sběru aplikujeme
    // outflow vlastní + dominance-sortovaný insert pro každou cílovou buňku.

    // Zaprvé spočítáme pro každou existující buňku její outflow loss
    // (= sum combined_rate) — ten odečteme od její paměti před aplikací inflows.
    // Dále sběr inflows per target.
    const inflowsByTarget = new Map(); // bigint -> [{ srcCell, fromDir (= směr, kterým src vyzařoval), slots }]

    for (const out of outflows) {
      const src = out.srcCell;
      const [dx, dy, dz] = DIR_OFFSET[out.dir];
      const tx = src.x + dx;
      const ty = src.y + dy;
      const tz = src.z + dz;
      const key = packCoord(tx, ty, tz);
      let bucket = inflowsByTarget.get(key);
      if (!bucket) {
        bucket = [];
        inflowsByTarget.set(key, bucket);
      }
      bucket.push({ srcCell: src, fromDir: out.dir, slots: out.slots });
    }

    // Aplikace na existující buňky — pamatujeme si oldMem.length - sumOutflow
    // a přidáváme inflows insertem podle dominance. Pro nové cílové buňky
    // (target neexistuje) prostě stackujeme inflows seřazené podle dominance.
    //
    // Důležité: musíme processit VŠECHNY existující buňky včetně těch, co
    // nemají inflow (jen mají outflow), a všechny target buňky (i nově
    // alokované).
    //
    // Snapshot pre-step energie pro dominance výpočet — neighbor.energy se
    // mění v průběhu této fáze (target dostane novou energii), ale dominance
    // má vycházet z útočníkovy ENERGIE PŘED TICKEM.

    const preStepEnergy = new Map();
    for (const cell of this.cells.values()) {
      preStepEnergy.set(packCoord(cell.x, cell.y, cell.z), cell.energy);
    }

    const allTargets = new Set();
    // Existing cells s outflow
    for (const cell of this.cells.values()) allTargets.add(packCoord(cell.x, cell.y, cell.z));
    // Targets z inflowsByTarget
    for (const key of inflowsByTarget.keys()) allTargets.add(key);

    const moveThreshold = this.moveThreshold;

    for (const targetKey of allTargets) {
      const [tx, ty, tz] = unpackCoord(targetKey);
      let cell = this.cells.get(targetKey);
      const isNew = !cell;
      if (isNew) {
        cell = this.allocateCell(tx, ty, tz);
      }

      // Můj outflow loss
      const myRates = combinedRatesByCell.get(targetKey);
      let totalOutflow = 0;
      if (myRates) for (let d = 0; d < DIRS; d++) totalOutflow += myRates[d];

      const oldMem = cell.memory;
      const sizeAfterOutflow = Math.max(0, oldMem.length - totalOutflow);

      // Inflow entries pro tuto buňku
      const bucket = inflowsByTarget.get(targetKey) || [];
      const inflowEntries = [];
      for (const inf of bucket) {
        const slots = inf.slots;
        if (slots.length === 0) continue;

        const neighbor = inf.srcCell;
        const nKey = packCoord(neighbor.x, neighbor.y, neighbor.z);
        const nRates = combinedRatesByCell.get(nKey);
        let neighborTotalOut = 0;
        if (nRates) for (let dd = 0; dd < DIRS; dd++) neighborTotalOut += nRates[dd];
        // Pre-step energie útočníka (může být už updated, pokud byl zpracován dřív)
        const neighborPreE = preStepEnergy.get(nKey) ?? neighbor.energy;
        const attackerEPostBurn = Math.max(1, neighborPreE - neighborTotalOut);

        const targetE = sizeAfterOutflow;
        const r = targetE / attackerEPostBurn;
        const dominance = Math.max(0, Math.min(1, 1 - r / moveThreshold));

        // d = směr ze strany targetu, pro stable tie-break (= opposite z fromDir)
        const dirFromTarget = OPPOSITE[inf.fromDir];
        inflowEntries.push({ d: dirFromTarget, slots, dominance, srcTag: neighbor.originTag });
      }

      inflowEntries.sort((a, b) => b.dominance - a.dominance || a.d - b.d);

      if (inflowEntries.length > 0 && inflowEntries[0].dominance >= 0.5) {
        cell.originTag = inflowEntries[0].srcTag >>> 0;
      }

      // Začneme s pamětí po vlastním outflow
      let workMem = oldMem.subarray(0, sizeAfterOutflow);

      for (const entry of inflowEntries) {
        const slots = entry.slots;
        const dom = entry.dominance;
        const currentSize = workMem.length;
        const intrusionDepth = Math.floor(dom * currentSize);
        const writeStart = Math.max(0, currentSize - intrusionDepth);

        const newSize = currentSize + slots.length;
        const cappedSize = Math.min(newSize, MAX_MEMORY);
        const merged = new Uint32Array(cappedSize);
        let pos = 0;
        for (let k = 0; k < writeStart && pos < cappedSize; k++) merged[pos++] = workMem[k];
        for (let k = 0; k < slots.length && pos < cappedSize; k++) merged[pos++] = slots[k];
        for (let k = writeStart; k < currentSize && pos < cappedSize; k++) merged[pos++] = workMem[k];
        workMem = merged;
      }

      // Pokud je workMem subarray, materializujeme do nové Uint32Array, ať
      // odpojíme od oldMem (jinak by Phase reset mohl zápis do mem starých buněk).
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

    // Fáze 5: reset active outflow a override flagů
    for (const cell of this.cells.values()) {
      for (let d = 0; d < DIRS; d++) {
        cell.activeOutflow[d] = 0;
        cell.pointerOverridden[d] = false;
      }
    }

    // Fáze 6: GC. Smazat všechny buňky s E = 0.
    // Důležité: provádět AŽ TEĎ, ne mezi outflow a inflow. Buňka, která
    // v outflow vyhodila všechnu energii, ale v inflow něco přijala, nezaniká.
    const toRemove = [];
    for (const [key, cell] of this.cells) {
      if (cell.energy === 0) toRemove.push(key);
    }
    for (const key of toRemove) this.cells.delete(key);

    // Fáze 7: layout pro další tick + refill budgetů
    this.recomputeAllLayouts();
    this.refillTickBudgets();

    this.tick += 1;
  }
}

// ----- Asembler (přejato z prototypu 9, rozšířené o zp/zn) -----

const MNEMONIC_TO_OPCODE = (() => {
  const m = {};
  for (const [opcode, info] of Object.entries(OPCODES)) {
    m[info.name] = { opcode: parseInt(opcode), len: info.len };
  }
  return m;
})();

const DIR_NAME_TO_INDEX = { xp: 0, xn: 1, yp: 2, yn: 3, zp: 4, zn: 5 };

function parseProgram(text) {
  const rawLines = text.split("\n").map(line => {
    const ci = line.indexOf(";");
    return (ci >= 0 ? line.slice(0, ci) : line).trim();
  }).filter(l => l.length > 0);

  const tokenizedLines = rawLines.map(line =>
    line.split(/[\s,]+/).filter(t => t.length > 0)
  );

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

  return { slots, errors };
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

// ----- Export (browser + Node) -----

const _exports = {
  SparseWorld, parseProgram, OPCODES, DIR_NAMES, DIR_OFFSET, OPPOSITE,
  makeRng, packCoord, unpackCoord, MAX_MEMORY, DIRS,
};

if (typeof window !== "undefined") {
  for (const [k, v] of Object.entries(_exports)) window[k] = v;
}
if (typeof module !== "undefined" && module.exports) {
  module.exports = _exports;
}

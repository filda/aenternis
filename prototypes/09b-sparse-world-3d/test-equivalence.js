"use strict";

// Equivalence test (3D): spustí stejný program v sparse světě i v toroidu se
// stejným seedem. Po každém ticku porovná stavy buňka po buňce. Pokud se
// rozejdou, vypíše první rozdíl.
//
// Předpokládá: dokud sparse world's bbox nepřekročí toroidní bbox,
// měly by být obě fyziky bit-identické.

const { SparseWorld, parseProgram, DIRS } = require("./world.js");
const { ToroidWorld } = require("./toroid.js");

const E_TOTAL = parseInt(process.env.E_TOTAL || "64", 10);
const TICKS = parseInt(process.env.TICKS || "200", 10);
const N_TOROID = parseInt(process.env.N || "32", 10);
const SEED = parseInt(process.env.SEED || "1234", 10);

const PROG = process.env.PROG || "self_xp_replicator";
const PROGRAMS = {
  pure_noise: null,
  counter: "loop:\n  inc 0x10\n  jmp loop",
  self_xp_replicator: "start:\n  setp xp, start\n  jmp start",
};

const programText = PROGRAMS[PROG];
if (programText === undefined) {
  console.error(`Neznámý program: ${PROG}. Volby: ${Object.keys(PROGRAMS).join(", ")}`);
  process.exit(2);
}

const programSlots = programText
  ? parseProgram(programText).slots
  : [];

const sparse = new SparseWorld({
  seed: SEED,
  diffusionCoeff: 0.15,
  cpuK: 1,
  moveThreshold: 2.0,
});
const toroid = new ToroidWorld({
  N: N_TOROID,
  seed: SEED,
  diffusionCoeff: 0.15,
  cpuK: 1,
  moveThreshold: 2.0,
});

sparse.bigBang(E_TOTAL, programSlots);
toroid.bigBang(E_TOTAL, programSlots);

const half = Math.floor(N_TOROID / 2);

function compareTick(t) {
  // Pro každou buňku v toroidu zkontroluj odpovídající buňku v sparse světě.
  // - Toroid cell s E > 0 musí mít sparse counterpart
  // - Toroid cell s E = 0 nesmí mít sparse counterpart (sparse GC ji smazal)
  // - Memory, PC, pointers, energy musí souhlasit
  // - Test platí jen dokud sparse bbox nepřekročí toroid range

  // Nejdřív: sparse bbox je uvnitř toroidu?
  const sparseBb = sparse.boundingBox();
  if (sparseBb) {
    if (sparseBb.xMin < -half || sparseBb.xMax > half - 1 ||
        sparseBb.yMin < -half || sparseBb.yMax > half - 1 ||
        sparseBb.zMin < -half || sparseBb.zMax > half - 1) {
      return { diverged: true, reason: "sparse přesáhl toroid bbox", t };
    }
  }

  // Výčet všech buněk v toroidu
  for (const tCell of toroid.cells) {
    const sCell = sparse.getCell(tCell.x, tCell.y, tCell.z);
    if (tCell.energy === 0) {
      if (sCell && sCell.energy > 0) {
        return { diverged: true, reason: `toroid (${tCell.x},${tCell.y},${tCell.z}) má E=0, sparse má E=${sCell.energy}`, t };
      }
      continue;
    }
    if (!sCell) {
      return { diverged: true, reason: `toroid (${tCell.x},${tCell.y},${tCell.z}) má E=${tCell.energy}, sparse buňka neexistuje`, t };
    }
    if (sCell.energy !== tCell.energy) {
      return { diverged: true, reason: `(${tCell.x},${tCell.y},${tCell.z}) energie: toroid=${tCell.energy}, sparse=${sCell.energy}`, t };
    }
    if (sCell.memory.length !== tCell.memory.length) {
      return { diverged: true, reason: `(${tCell.x},${tCell.y},${tCell.z}) memSize: toroid=${tCell.memory.length}, sparse=${sCell.memory.length}`, t };
    }
    for (let i = 0; i < tCell.memory.length; i++) {
      if (sCell.memory[i] !== tCell.memory[i]) {
        return { diverged: true, reason: `(${tCell.x},${tCell.y},${tCell.z}) mem[${i}]: toroid=0x${tCell.memory[i].toString(16)}, sparse=0x${sCell.memory[i].toString(16)}`, t };
      }
    }
    if (sCell.pc !== tCell.pc) {
      return { diverged: true, reason: `(${tCell.x},${tCell.y},${tCell.z}) pc: toroid=${tCell.pc}, sparse=${sCell.pc}`, t };
    }
    for (let d = 0; d < DIRS; d++) {
      if (sCell.pointers[d] !== tCell.pointers[d]) {
        return { diverged: true, reason: `(${tCell.x},${tCell.y},${tCell.z}) ptr[${d}]: toroid=${tCell.pointers[d]}, sparse=${sCell.pointers[d]}`, t };
      }
    }
  }
  return { diverged: false };
}

console.log(`Equivalence test (3D): program=${PROG}, E_total=${E_TOTAL}, ticks=${TICKS}, N_toroid=${N_TOROID}, seed=${SEED}`);

let lastGood = 0;
let firstDiverge = null;
for (let t = 1; t <= TICKS; t++) {
  sparse.step();
  toroid.step();
  const cmp = compareTick(t);
  if (cmp.diverged) {
    if (!firstDiverge) firstDiverge = cmp;
    if (cmp.reason && cmp.reason.startsWith("sparse přesáhl")) {
      console.log(`tick ${t}: ${cmp.reason} — test ukončen, předchozí ticky souhlasily`);
      break;
    }
    console.log(`tick ${t}: DIVERGENCE — ${cmp.reason}`);
    process.exit(1);
  }
  lastGood = t;
}

console.log(`OK: ${lastGood} ticek souhlasilo bit-identicky.`);
process.exit(0);

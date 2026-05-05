"use strict";

// Detailní debug — projde jeden tick po druhém a ukáže přesně, kde se
// implementace rozejdou. E_total=8 pro malý a čitelný stav.

const { SparseWorld, parseProgram } = require("./world.js");
const { ToroidWorld } = require("./toroid.js");

const E_TOTAL = parseInt(process.env.E_TOTAL || "8", 10);
const SEED = parseInt(process.env.SEED || "1234", 10);

const sparse = new SparseWorld({ seed: SEED, diffusionCoeff: 0.15, cpuK: 1, moveThreshold: 2.0 });
const toroid = new ToroidWorld({ N: 32, seed: SEED, diffusionCoeff: 0.15, cpuK: 1, moveThreshold: 2.0 });

sparse.bigBang(E_TOTAL);
toroid.bigBang(E_TOTAL);

function dumpCell(label, cell) {
  if (!cell) { console.log(`${label}: <neexistuje>`); return; }
  if (cell.energy === 0) { console.log(`${label}: E=0`); return; }
  const mem = Array.from(cell.memory).map(v => v.toString(16).padStart(8, "0")).join(" ");
  console.log(`${label}: E=${cell.energy} pc=${cell.pc} ptrs=${cell.pointers.join(",")} rates=${cell.rates.join(",")} mem=${mem}`);
}

function dumpBoth(coord, t) {
  const [x, y, z] = coord;
  console.log(`--- (${x},${y},${z}) tick ${t} ---`);
  dumpCell("  sparse", sparse.getCell(x, y, z));
  dumpCell("  toroid", toroid.getCell(x, y, z));
}

// Origin + 6 přímých sousedů + několik diagonálních / second-shell pozic.
const coords = [
  [ 0,  0,  0],
  [+1,  0,  0], [-1,  0,  0],
  [ 0, +1,  0], [ 0, -1,  0],
  [ 0,  0, +1], [ 0,  0, -1],
  [+2,  0,  0], [-2,  0,  0],
  [ 0, +2,  0], [ 0, -2,  0],
  [ 0,  0, +2], [ 0,  0, -2],
];
const TICKS = parseInt(process.env.TICKS || "2", 10);

console.log("\n=== TICK 0 (po bigBang) ===");
for (const c of coords) dumpBoth(c, 0);

for (let t = 1; t <= TICKS; t++) {
  sparse.step();
  toroid.step();
  console.log(`\n=== TICK ${t} ===`);
  for (const c of coords) dumpBoth(c, t);
}

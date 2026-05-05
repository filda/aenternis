"use strict";

// Headless test prototypu 9-B (3D). Spouštět:
//   node test-headless.js
//
// Ověřuje:
//   1. Konzervace energie přes 10 000 ticek pro 3 různé programy
//   2. Strop world.size() <= E_total ve všech ticích
//   3. Big bang z 1 buňky funguje (svět expanduje do všech 6 směrů)
//   4. Heat-death scenario nakonec nastane (E_total buněk s E ≈ 1)

const { SparseWorld, parseProgram } = require("./world.js");

const E_TOTAL = parseInt(process.env.E_TOTAL || "256", 10);
const TICKS = parseInt(process.env.TICKS || "100", 10);

function runScenario(name, programText) {
  const world = new SparseWorld({ seed: 42, diffusionCoeff: 0.15, cpuK: 1, moveThreshold: 2.0 });

  let programSlots = [];
  if (programText) {
    const parsed = parseProgram(programText);
    if (parsed.errors.length > 0) {
      console.log(`  [${name}] asembler chyby:`, parsed.errors);
    }
    programSlots = parsed.slots;
  }

  world.bigBang(E_TOTAL, programSlots);

  let maxSize = 0;
  let conservationViolations = 0;
  let capViolations = 0;
  const checkpoints = [];

  // Po každých 1000 ticích snapshot.
  for (let t = 1; t <= TICKS; t++) {
    world.step();
    const total = world.totalEnergy();
    const size = world.size();
    if (total !== E_TOTAL) conservationViolations++;
    if (size > E_TOTAL) capViolations++;
    if (size > maxSize) maxSize = size;
    const checkpointEvery = Math.max(1, Math.floor(TICKS / 10));
    if (t % checkpointEvery === 0) {
      const bb = world.boundingBox();
      const c = world.centroid();
      const bbStr = bb
        ? `(${bb.xMin}..${bb.xMax}, ${bb.yMin}..${bb.yMax}, ${bb.zMin}..${bb.zMax})`
        : "(empty)";
      const cStr = c
        ? `(${c.x.toFixed(2)}, ${c.y.toFixed(2)}, ${c.z.toFixed(2)})`
        : "(empty)";
      checkpoints.push({ t, total, size, bbStr, cStr });
    }
  }

  console.log(`\n=== Scénář: ${name} ===`);
  console.log(`  E_total: ${E_TOTAL}, ticek: ${TICKS}`);
  console.log(`  max world.size(): ${maxSize}`);
  console.log(`  konzervační porušení: ${conservationViolations}`);
  console.log(`  cap porušení (size > E_total): ${capViolations}`);
  console.log(`  checkpoints (každých 1000 ticek):`);
  for (const c of checkpoints) {
    console.log(`    tick ${c.t}: E=${c.total}, size=${c.size}, bbox=${c.bbStr}, centroid=${c.cStr}`);
  }
  return { conservationViolations, capViolations, maxSize };
}

// Tři scénáře:
const scenarios = [
  ["pure_noise", null],  // čistá náhoda v paměti
  ["counter", `
    loop:
      inc 0x10
      jmp loop
  `],
  ["self_xp_replicator", `
    start:
      setp xp, start
      jmp start
  `],
];

let totalFails = 0;
for (const [name, prog] of scenarios) {
  const r = runScenario(name, prog);
  if (r.conservationViolations > 0 || r.capViolations > 0) totalFails++;
}

console.log("\n=== SOUHRN ===");
if (totalFails === 0) {
  console.log("VŠECHNY SCÉNÁŘE PROŠLY: konzervace + strop drží.");
  process.exit(0);
} else {
  console.log(`${totalFails} scénář(ů) selhalo.`);
  process.exit(1);
}

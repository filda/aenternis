"use strict";

// Diagnostic dump of world state for cell-by-cell comparison against
// the Rust port. Mirror is in `crates/aenternis-core/tests/
// dump_state_for_diff.rs`. Both files take TICKS as input — defaults
// to 5 — so divergence can be bisected tick by tick.
//
//   TICKS=1 node prototypes/09b-sparse-world-3d/dump-state.js
//
// Writes to `reports/9b-tick<N>.txt` directly (no shell redirect — Git
// Bash on Windows trips on `node ... > file` with "stdout is not a
// tty"; this avoids that whole class of redirect quirks).
//
// Output line format `(x,y,z) E=N mem[0..3]=[a,b,c]` is fixed — the
// Rust dump prints the same shape so `diff` reads cleanly.

const fs = require("fs");
const path = require("path");
const { SparseWorld, parseProgram } = require("./world.js");

const TICKS = parseInt(process.env.TICKS || "5", 10);

const { slots: program } = parseProgram("start:\n  setp xp, start\n  jmp start");

const w = new SparseWorld({
  seed: 1234,
  diffusionCoeff: 0.15,
  cpuK: 1,
  moveThreshold: 1.0,
  useMathImul: true, // klíčové: musí matchnout Rust xs32 mode
});
w.bigBang(65536, program);

for (let t = 0; t < TICKS; t++) w.step();

const cells = [...w.cells.values()].sort((a, b) =>
  a.x - b.x || a.y - b.y || a.z - b.z,
);

const lines = [];
for (const c of cells) {
  const m0 = c.memory[0] >>> 0 || 0;
  const m1 = c.memory[1] >>> 0 || 0;
  const m2 = c.memory[2] >>> 0 || 0;
  lines.push(`(${c.x},${c.y},${c.z}) E=${c.energy} mem[0..3]=[${m0},${m1},${m2}]`);
}
lines.push(`total energy: ${w.totalEnergy()}, cells: ${w.size()}`);

// Resolve `reports/` relative to the repo root (= two levels up from
// this script in `prototypes/09b-sparse-world-3d/`).
const repoRoot = path.resolve(__dirname, "..", "..");
const reportsDir = path.join(repoRoot, "reports");
fs.mkdirSync(reportsDir, { recursive: true });
const outPath = path.join(reportsDir, `9b-tick${TICKS}.txt`);
fs.writeFileSync(outPath, lines.join("\n") + "\n");

// One short status message via process.stderr so the user sees where
// the dump landed without polluting the comparison file.
process.stderr.write(`9-B dump @ tick ${TICKS} → ${outPath} (${cells.length} cells)\n`);

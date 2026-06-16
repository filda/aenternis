#!/usr/bin/env node
// Headless generator for the render-tuner's static snapshot
// (prototypes/10-render-tuner/snapshot.bin + snapshot.meta.json).
//
// Mirrors the in-browser `captureSnapshot()` recipe in
// prototypes/10-render-tuner/main.ts exactly (same seed / energy / ticks /
// coeff / k / move_threshold) so the cached field matches what the tuner
// would generate live — but runs in Node against the current core, so the
// committed snapshot stays in sync with the simulation as it evolves.
//
// PREREQUISITE: a fresh nodejs-target wasm build (the repo's ./build only
// emits the web target, so pkg-node can go stale):
//
//   wasm-pack build crates/aenternis-wasm --target nodejs --release --out-dir pkg-node
//
// Then: node scripts/gen-tuner-snapshot.mjs

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

// pkg-node is a CommonJS module (wasm-bindgen nodejs target) and loads its
// .wasm synchronously at require time; createRequire bridges it into this ESM.
const require = createRequire(import.meta.url);
const { World } = require('../crates/aenternis-wasm/pkg-node/aenternis_wasm.js');

// Recipe — keep in lock-step with the CAPTURE_* constants in main.ts.
const SEED = 1234;
const ENERGY = 1_000_000;
const TICKS = 250;
const COEFF = 0.15;
const K = 1;
const MOVE_THRESHOLD = 1.0;

const world = World.newWithProgram(SEED, ENERGY, new Uint32Array(0));
world.setMoveThreshold(MOVE_THRESHOLD);
for (let i = 0; i < TICKS; i += 1) world.step(COEFF, K);

// Copy out of WASM linear memory before any further call invalidates the view.
const snap = new Uint32Array(world.cellsSnapshotView());
const bb = world.boundingBox();
if (bb.length < 6) throw new Error('World produced empty bbox after capture ticks');

const meta = {
  stride: world.snapshotStride,
  cellCount: world.cellCount(),
  totalEnergy: world.totalEnergy(),
  bbox: { minX: bb[0], maxX: bb[1], minY: bb[2], maxY: bb[3], minZ: bb[4], maxZ: bb[5] },
  tick: world.tick(),
  seed: SEED,
  ticks: TICKS,
  energyIn: ENERGY,
};
world.free();

const outDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'prototypes', '10-render-tuner');
fs.writeFileSync(path.join(outDir, 'snapshot.bin'), Buffer.from(snap.buffer));
fs.writeFileSync(path.join(outDir, 'snapshot.meta.json'), `${JSON.stringify(meta, null, 2)}\n`);

console.log(`wrote snapshot: ${meta.cellCount.toLocaleString()} cells, ` +
  `E=${meta.totalEnergy.toLocaleString()}, tick=${meta.tick}, stride=${meta.stride}`);
console.log(`bbox x[${meta.bbox.minX},${meta.bbox.maxX}] ` +
  `y[${meta.bbox.minY},${meta.bbox.maxY}] z[${meta.bbox.minZ},${meta.bbox.maxZ}]`);

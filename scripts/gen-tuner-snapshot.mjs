#!/usr/bin/env node
// Headless generator for the render-tuner's static snapshot
// (prototypes/10-render-tuner/snapshot.bin + snapshot.meta.json).
//
// Uses the shared production config (src/sim-defaults.ts) — the exact same
// world the viewer boots and the in-browser captureSnapshot() generates, so
// the cached field never drifts from production. Runs in Node against the
// current core, keeping the committed snapshot in sync as the sim evolves.
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

import { parseProgramText } from '../src/program-text.ts';
import {
  CAPTURE_TICKS,
  DEFAULT_PROGRAM_TEXT,
  DEFAULT_SIM_CONFIG,
  applySimConfig,
} from '../src/sim-defaults.ts';

// pkg-node is a CommonJS module (wasm-bindgen nodejs target) and loads its
// .wasm synchronously at require time; createRequire bridges it into this ESM.
const require = createRequire(import.meta.url);
const { World } = require('../crates/aenternis-wasm/pkg-node/aenternis_wasm.js');

// Config = the shared production defaults (src/sim-defaults.ts), so this
// matches the viewer and the in-browser capture without any hand-syncing.
const cfg = DEFAULT_SIM_CONFIG;
const { program } = parseProgramText(DEFAULT_PROGRAM_TEXT);

const world = World.newWithProgram(cfg.seed, cfg.energy, program ?? new Uint32Array(0));
applySimConfig(world, cfg);
for (let i = 0; i < CAPTURE_TICKS; i += 1) world.step(cfg.coeff, cfg.k);

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
  seed: cfg.seed,
  ticks: CAPTURE_TICKS,
  energyIn: cfg.energy,
};
world.free();

const outDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'prototypes', '10-render-tuner');
fs.writeFileSync(path.join(outDir, 'snapshot.bin'), Buffer.from(snap.buffer));
fs.writeFileSync(path.join(outDir, 'snapshot.meta.json'), `${JSON.stringify(meta, null, 2)}\n`);

console.log(`wrote snapshot: ${meta.cellCount.toLocaleString()} cells, ` +
  `E=${meta.totalEnergy.toLocaleString()}, tick=${meta.tick}, stride=${meta.stride}`);
console.log(`bbox x[${meta.bbox.minX},${meta.bbox.maxX}] ` +
  `y[${meta.bbox.minY},${meta.bbox.maxY}] z[${meta.bbox.minZ},${meta.bbox.maxZ}]`);

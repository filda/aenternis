// @vitest-environment happy-dom

// Wasm side of the wasm ↔ native physics-parity gate.
//
// `tests/fixtures/parity-world.json` holds a golden snapshot generated
// and pinned by the native core
// (`crates/aenternis-core/tests/wasm_parity_fixture.rs`). This test
// drives the **compiled wasm bundle** through the production JS
// boundary — `World.newWithProgram` + the physics setters, the exact
// calls `worker-handler.ts` makes — and asserts the same bytes fall
// out. Native and wasm both pinning to one committed artifact turns
// any cross-backend divergence (codegen, float semantics, RNG,
// genesis) into a red `./check` instead of a silent physics fork.
//
// Conditionally skipped when `crates/aenternis-wasm/pkg/` is missing,
// same as `wasm-bundle.smoke.test.ts` (and for the same reason: the
// vitest CI job doesn't build the bundle).
//
// To regenerate the fixture after an intentional physics change:
//   UPDATE_PARITY_FIXTURE=1 cargo test -p aenternis-core --test wasm_parity_fixture

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, it, expect } from 'vitest';

import fixture from '../fixtures/parity-world.json';

const PKG_PATH = resolve(
  process.cwd(),
  'crates/aenternis-wasm/pkg/aenternis_wasm.js',
);
const WASM_PATH = resolve(
  process.cwd(),
  'crates/aenternis-wasm/pkg/aenternis_wasm_bg.wasm',
);

const pkgExists = existsSync(PKG_PATH) && existsSync(WASM_PATH);

/** Minimal slice of the generated `World` class this test drives. */
interface ParityWorld {
  setMoveThreshold(v: number): void;
  setGravity(v: number): void;
  setGravityAlpha(v: number): void;
  setGravityRadius(v: number): void;
  setPressure(v: number): void;
  setPressureGamma(v: number): void;
  setPressureEref(v: number): void;
  setMutationStrength(v: number): void;
  setMutationHalfDensity(v: number): void;
  step(coeff: number, k: number): void;
  cellsSnapshot(): Uint32Array;
  cellCount(): number;
  totalEnergy(): number;
}

describe.skipIf(!pkgExists)('wasm ↔ native physics parity (golden fixture)', () => {
  it('the compiled wasm bundle reproduces the native golden world byte-for-byte', async () => {
    const wasm = (await import(PKG_PATH)) as {
      default: (bytes: Uint8Array) => Promise<unknown>;
      World: {
        newWithProgram(
          seed: number,
          energy: number,
          program: Uint32Array,
          window: number,
          fertility: number,
        ): ParityWorld;
      };
    };
    // Feed the `.wasm` bytes directly so `init()` skips the fetch path
    // happy-dom can't resolve (see wasm-bundle.smoke.test.ts).
    await wasm.default(new Uint8Array(readFileSync(WASM_PATH)));

    const cfg = fixture.config;
    const world = wasm.World.newWithProgram(
      cfg.seed,
      cfg.energy,
      new Uint32Array(cfg.program),
      cfg.genesisWindow,
      cfg.genesisFertility,
    );
    // Same setter sequence the worker applies (`applyStateToWorld`).
    world.setMoveThreshold(cfg.moveThreshold);
    world.setGravity(cfg.gravity);
    world.setGravityAlpha(cfg.gravityAlpha);
    world.setGravityRadius(cfg.gravityRadius);
    world.setPressure(cfg.pressure);
    world.setPressureGamma(cfg.pressureGamma);
    world.setPressureEref(cfg.pressureEref);
    world.setMutationStrength(cfg.mutationStrength);
    world.setMutationHalfDensity(cfg.mutationHalfDensity);

    for (let i = 0; i < cfg.ticks; i += 1) {
      world.step(cfg.coeff, cfg.k);
    }

    expect(world.cellCount()).toBe(fixture.expected.cellCount);
    expect(world.totalEnergy()).toBe(fixture.expected.totalEnergy);
    expect(Array.from(world.cellsSnapshot())).toEqual(fixture.expected.snapshot);
  });
});

describe.skipIf(pkgExists)('wasm ↔ native parity (skipped — pkg/ not built)', () => {
  it('skipped: run `wasm-pack build crates/aenternis-wasm --target web` (or `bash scripts/build-wasm.sh`) to enable the parity suite', () => {
    expect(pkgExists).toBe(false);
  });
});

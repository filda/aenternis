// @vitest-environment happy-dom

// Single-threaded WASM bundle smoke test.
//
// Loads `crates/aenternis-wasm/pkg/aenternis_wasm.js`, runs a tiny
// World end-to-end, and asserts the basic invariants. The point is to
// catch the class of regressions where the bundle is structurally
// broken (missing export, init throws, step panics in non-threaded
// code path) without needing a real browser.
//
// **What this test does NOT cover:** `initThreadPool` and any
// multi-threaded execution. Calling `initThreadPool` here would spawn
// Web Workers, and happy-dom's Worker shim runs them in the same JS
// context — that either deadlocks on `Atomics.wait` or fakes
// concurrency in a way that doesn't exercise the real race-sensitive
// code path. Bugs that only manifest under true threading need a
// headless-browser smoke (see `docs/optimalizace-2026-05.md`).
//
// **Conditionally skipped** when `crates/aenternis-wasm/pkg/` is
// missing. The vitest job in CI doesn't currently run `wasm-pack
// build` (only the typecheck + deploy jobs do); locally, devs may not
// have built the bundle before `npm test`. Skipping with a clear
// message beats hard-failing on a missing build artifact.

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, it, expect } from 'vitest';

// vitest runs with cwd = repo root. `import.meta.url` would be the
// more idiomatic source-relative anchor, but happy-dom rewrites it
// to something `fileURLToPath` can't parse; falling back to
// `process.cwd()` is robust across environments.
const PKG_PATH = resolve(
  process.cwd(),
  'crates/aenternis-wasm/pkg/aenternis_wasm.js',
);
const WASM_PATH = resolve(
  process.cwd(),
  'crates/aenternis-wasm/pkg/aenternis_wasm_bg.wasm',
);

const pkgExists = existsSync(PKG_PATH) && existsSync(WASM_PATH);

// wasm-bindgen's generated `init()` defaults to `fetch(new URL('aenternis_
// _wasm_bg.wasm', import.meta.url))`, which happy-dom can't resolve to a
// local file and which hangs indefinitely under vitest. Feeding the
// `.wasm` bytes in as a `BufferSource` makes `init()` route through
// `WebAssembly.instantiate(bytes, imports)` instead — fast, no I/O,
// works in any Node-flavoured environment.
function loadWasmBytes(): Uint8Array {
  // Wrap in a fresh Uint8Array so the underlying ArrayBuffer is a
  // plain heap buffer, not Node's internal pool slice — wasm-bindgen
  // accepts both but the plain form is portable across Node versions.
  return new Uint8Array(readFileSync(WASM_PATH));
}

describe.skipIf(!pkgExists)('wasm bundle (single-threaded path)', () => {
  it('imports without throwing and exposes the expected API surface', async () => {
    // Dynamic import so the missing-pkg case above can suppress this
    // module entirely. Static import at the file top would error out
    // at the test-file load step before any `skipIf` can fire.
    const wasm = (await import(PKG_PATH)) as {
      default: (init?: unknown) => Promise<unknown>;
      World: new (seed: number, energy: number) => unknown;
      initThreadPool?: (n: number) => Promise<void>;
    };
    expect(typeof wasm.default).toBe('function');
    expect(typeof wasm.World).toBe('function');
    // `initThreadPool` is optional — present on the threaded build,
    // absent on the single-threaded build. We don't assert either way
    // (both shapes are valid); we do assert it's never something
    // other than `function | undefined`, which catches a malformed
    // export shape.
    expect(['function', 'undefined']).toContain(typeof wasm.initThreadPool);
  });

  it('runs `init` + `World.new` + 50 ticks + snapshot without throwing', async () => {
    const wasm = await import(PKG_PATH);
    await wasm.default(loadWasmBytes());

    const world = new wasm.World(42, 1_000) as {
      step: (coeff: number, k: number) => void;
      cellsSnapshot: () => Uint32Array;
      cellCount: () => number;
      tick: () => number;
      readonly snapshotStride: number;
    };

    for (let i = 0; i < 50; i++) {
      world.step(0.2, 1);
    }

    expect(world.tick()).toBe(50);
    expect(world.cellCount()).toBeGreaterThan(0);

    const snap = world.cellsSnapshot();
    expect(snap).toBeInstanceOf(Uint32Array);
    expect(snap.length).toBe(world.cellCount() * world.snapshotStride);
  });

  it('is deterministic — same seed + energy produce identical snapshots', async () => {
    const wasm = await import(PKG_PATH);
    await wasm.default(loadWasmBytes());

    type World = {
      step: (coeff: number, k: number) => void;
      cellsSnapshot: () => Uint32Array;
    };

    const a = new wasm.World(7, 500) as World;
    const b = new wasm.World(7, 500) as World;
    for (let i = 0; i < 20; i++) {
      a.step(0.15, 1);
      b.step(0.15, 1);
    }

    const snapA = a.cellsSnapshot();
    const snapB = b.cellsSnapshot();
    expect(snapA.length).toBe(snapB.length);
    // Byte-equality. Two `Uint32Array`s with identical contents but
    // different backing buffers are not `===` and vitest's
    // `.toEqual` on typed arrays compares element-wise — exactly
    // what we want.
    expect(Array.from(snapA)).toEqual(Array.from(snapB));
  });
});

describe.skipIf(pkgExists)('wasm bundle (skipped — pkg/ not built)', () => {
  it('skipped: run `wasm-pack build crates/aenternis-wasm --target web` (or `bash scripts/build-wasm.sh`) to enable the smoke suite', () => {
    // Marker test so the skip is visible in the test output rather
    // than the smoke-tests just disappearing silently.
    expect(pkgExists).toBe(false);
  });
});

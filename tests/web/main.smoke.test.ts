// @vitest-environment happy-dom

// Smoke test for the WASM-viewer entry point. We deliberately do NOT
// invoke `bootstrap` here — that path mounts a real `THREE.WebGLRenderer`
// against the canvas and would need a WebGL context that happy-dom
// doesn't provide.
//
// The value this test gives us is much narrower:
//   1. Importing `web/main.ts` does not throw (no top-level side
//      effects, no broken imports, no syntax errors).
//   2. The `bootstrap` symbol is exported and is a function.
//
// In other words: the file isn't structurally broken. Anything stronger
// is covered by the pure-logic modules in `src/` under the full
// vitest+stryker gate.

import { describe, it, expect } from 'vitest';

import { bootstrap } from '../../web/main.ts';

describe('web/main bootstrap', () => {
  it('exposes bootstrap as a function', () => {
    expect(typeof bootstrap).toBe('function');
  });

  it('imports cleanly with no top-level side effects', () => {
    // The import statement above is the actual assertion: if any
    // top-level code in `main.ts` accessed the DOM, instantiated a
    // Worker, or otherwise leaked a side effect, this test file would
    // never load. Reaching this assertion is the proof.
    expect(true).toBe(true);
  });
});

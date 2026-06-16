// Single source of truth for the production default simulation config.
//
// The 3D viewer (web/main.ts), the render-tuner capture
// (prototypes/10-render-tuner/main.ts) and the headless snapshot generator
// (scripts/gen-tuner-snapshot.mjs) all derive their world configuration from
// here, so the tuner always tunes against the same world the viewer shows —
// without anyone hand-syncing values across files.
//
// NOTE: this is NOT the engine fall-back. `DEFAULT_STATE` in worker-state.ts
// stays all-off (the frozen-baseline SparseWorld defaults); these values are
// the viewer's chosen starting point (the "Cauldron" preset).

/** World configuration the viewer boots with and the tuner captures against. */
export interface SimConfig {
  seed: number;
  energy: number;
  coeff: number;
  k: number;
  moveThreshold: number;
  gravity: number;
  gravityAlpha: number;
  gravityRadius: number;
  pressure: number;
  pressureGamma: number;
  pressureEref: number;
  mutationStrength: number;
  mutationHalfDensity: number;
}

/** Production default config — the "Cauldron" preset (2026-06-14): mutagenic
 *  dense cores held by gravity, a gentle player. Pressure (not mutation) caps
 *  core density; high eref + strong gravity densify cores into the tens of
 *  thousands of E while a gentle player stays calm. See docs/mechanics.md. */
export const DEFAULT_SIM_CONFIG: SimConfig = Object.freeze({
  seed: 1234,
  energy: 1_000_000,
  coeff: 0.15,
  k: 1,
  moveThreshold: 1.0,
  gravity: 1.0,
  gravityAlpha: 0.05,
  gravityRadius: 4,
  pressure: 0.2,
  pressureGamma: 2.0,
  pressureEref: 50_000.0,
  mutationStrength: 1.0,
  mutationHalfDensity: 40_000,
});

/** Tick count for a captured static snapshot (the viewer itself runs forever;
 *  a snapshot needs a fixed cutoff). */
export const CAPTURE_TICKS = 250;

/** Default origin-cell program overlaid on the macro-genesis base — the
 *  viewer's starting textarea content. Genesis fills the whole memory; this
 *  just seeds the origin cell's first slots. */
export const DEFAULT_PROGRAM_TEXT = 'start:\n  setp xp, start\n  jmp start';

/** A world exposing the per-knob sim setters (wasm `World`, the node-target
 *  build, or the worker `WorldHandle` — all share these signatures). */
export interface SimSettableWorld {
  setMoveThreshold(v: number): void;
  setGravity(v: number): void;
  setGravityAlpha(v: number): void;
  setGravityRadius(v: number): void;
  setPressure(v: number): void;
  setPressureGamma(v: number): void;
  setPressureEref(v: number): void;
  setMutationStrength(v: number): void;
  setMutationHalfDensity(v: number): void;
}

/** Push every physics knob from `cfg` onto a freshly created world. Kept here
 *  so the viewer and both capture paths apply the exact same sequence. */
export function applySimConfig(world: SimSettableWorld, cfg: SimConfig): void {
  world.setMoveThreshold(cfg.moveThreshold);
  world.setGravity(cfg.gravity);
  world.setGravityAlpha(cfg.gravityAlpha);
  world.setGravityRadius(cfg.gravityRadius);
  world.setPressure(cfg.pressure);
  world.setPressureGamma(cfg.pressureGamma);
  world.setPressureEref(cfg.pressureEref);
  world.setMutationStrength(cfg.mutationStrength);
  world.setMutationHalfDensity(cfg.mutationHalfDensity);
}

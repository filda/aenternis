// Worker-side simulation state — the subset of worker state that does
// not include the `World` instance itself. Pure reducer functions over
// this state make the message-handling logic testable without a real
// WASM World.

import type { ConfigMsg, InitMsg } from './protocol.ts';

export interface WorkerSimState {
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold: number;
  readonly gravity: number;
  readonly gravityAlpha: number;
  readonly gravityRadius: number;
  readonly pressure: number;
  readonly pressureGamma: number;
  readonly pressureEref: number;
  readonly mutationStrength: number;
  readonly mutationHalfDensity: number;
  /** Code-metrics sampling cadence in ticks; `0` = disabled (full speed). */
  readonly metricsEvery: number;
}

/** Default state. Matches the fall-back values used in the original
 *  `web/worker.js` top-level `let` declarations, plus the engine's
 *  gravity/pressure defaults (both off). */
export const DEFAULT_STATE: WorkerSimState = Object.freeze({
  coeff: 0.20,
  k: 1,
  moveThreshold: 2.0,
  gravity: 0.0,
  gravityAlpha: 0.0,
  gravityRadius: 1,
  pressure: 0.0,
  pressureGamma: 2.0,
  pressureEref: 1.0,
  mutationStrength: 0.0,
  mutationHalfDensity: 40_000,
  metricsEvery: 0,
});

/** Reducer: applies an `init` message to produce the initial state.
 *  Each optional physics field falls back to the default if absent. */
export function stateFromInit(msg: InitMsg): WorkerSimState {
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: msg.moveThreshold ?? DEFAULT_STATE.moveThreshold,
    gravity: msg.gravity ?? DEFAULT_STATE.gravity,
    gravityAlpha: msg.gravityAlpha ?? DEFAULT_STATE.gravityAlpha,
    gravityRadius: msg.gravityRadius ?? DEFAULT_STATE.gravityRadius,
    pressure: msg.pressure ?? DEFAULT_STATE.pressure,
    pressureGamma: msg.pressureGamma ?? DEFAULT_STATE.pressureGamma,
    pressureEref: msg.pressureEref ?? DEFAULT_STATE.pressureEref,
    mutationStrength: msg.mutationStrength ?? DEFAULT_STATE.mutationStrength,
    mutationHalfDensity: msg.mutationHalfDensity ?? DEFAULT_STATE.mutationHalfDensity,
    metricsEvery: msg.metricsEvery ?? DEFAULT_STATE.metricsEvery,
  };
}

/** Reducer: applies a `config` message to existing state. `coeff` and
 *  `k` are always overwritten (they're required on `ConfigMsg`); every
 *  other field only updates if present (a `typeof === 'number'` guard so
 *  an explicit `0` still applies, but `undefined` keeps the prior value). */
export function applyConfig(state: WorkerSimState, msg: ConfigMsg): WorkerSimState {
  const pick = (next: number | undefined, prev: number): number =>
    typeof next === 'number' ? next : prev;
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: pick(msg.moveThreshold, state.moveThreshold),
    gravity: pick(msg.gravity, state.gravity),
    gravityAlpha: pick(msg.gravityAlpha, state.gravityAlpha),
    gravityRadius: pick(msg.gravityRadius, state.gravityRadius),
    pressure: pick(msg.pressure, state.pressure),
    pressureGamma: pick(msg.pressureGamma, state.pressureGamma),
    pressureEref: pick(msg.pressureEref, state.pressureEref),
    mutationStrength: pick(msg.mutationStrength, state.mutationStrength),
    mutationHalfDensity: pick(msg.mutationHalfDensity, state.mutationHalfDensity),
    metricsEvery: pick(msg.metricsEvery, state.metricsEvery),
  };
}

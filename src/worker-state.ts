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
  readonly pressure: number;
  readonly pressureGamma: number;
  readonly pressureEref: number;
  readonly baseMutationRate: number;
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
  pressure: 0.0,
  pressureGamma: 2.0,
  pressureEref: 1.0,
  baseMutationRate: 0.0,
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
    pressure: msg.pressure ?? DEFAULT_STATE.pressure,
    pressureGamma: msg.pressureGamma ?? DEFAULT_STATE.pressureGamma,
    pressureEref: msg.pressureEref ?? DEFAULT_STATE.pressureEref,
    baseMutationRate: msg.baseMutationRate ?? DEFAULT_STATE.baseMutationRate,
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
    pressure: pick(msg.pressure, state.pressure),
    pressureGamma: pick(msg.pressureGamma, state.pressureGamma),
    pressureEref: pick(msg.pressureEref, state.pressureEref),
    baseMutationRate: pick(msg.baseMutationRate, state.baseMutationRate),
  };
}

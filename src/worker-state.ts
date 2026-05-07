// Worker-side simulation state — the subset of worker state that does
// not include the `World` instance itself. Pure reducer functions over
// this state make the message-handling logic testable without a real
// WASM World.

import type { ConfigMsg, InitMsg } from './protocol.ts';

export interface WorkerSimState {
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold: number;
}

/** Default state. Matches the fall-back values used in the original
 *  `web/worker.js` top-level `let` declarations. */
export const DEFAULT_STATE: WorkerSimState = Object.freeze({
  coeff: 0.20,
  k: 1,
  moveThreshold: 2.0,
});

/** Reducer: applies an `init` message to produce the initial state.
 *  `moveThreshold` falls back to the default if not provided. */
export function stateFromInit(msg: InitMsg): WorkerSimState {
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: msg.moveThreshold ?? DEFAULT_STATE.moveThreshold,
  };
}

/** Reducer: applies a `config` message to existing state. `coeff` and
 *  `k` are always overwritten (they're required on `ConfigMsg`);
 *  `moveThreshold` only updates if present. */
export function applyConfig(state: WorkerSimState, msg: ConfigMsg): WorkerSimState {
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: typeof msg.moveThreshold === 'number'
      ? msg.moveThreshold
      : state.moveThreshold,
  };
}

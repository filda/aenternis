// Worker-side simulation state — the subset of worker state that does
// not include the `World` instance itself. Pure reducer functions over
// this state make the message-handling logic testable without a real
// WASM World.

import type { ConfigMsg, InitMsg } from './protocol.ts';

export interface WorkerSimState {
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold: number;
  readonly legacyTickOffset: boolean;
  readonly legacyFullPrecision: boolean;
  readonly legacyPortWrap: boolean;
  readonly legacyOpcodeSet: boolean;
}

/** Default state. Matches the fall-back values used in the original
 *  `web/worker.js` top-level `let` declarations. */
export const DEFAULT_STATE: WorkerSimState = Object.freeze({
  coeff: 0.20,
  k: 1,
  moveThreshold: 2.0,
  legacyTickOffset: false,
  legacyFullPrecision: false,
  legacyPortWrap: false,
  legacyOpcodeSet: false,
});

/** Reducer: applies an `init` message to produce the initial state.
 *  All `legacy*` flags coerce missing/undefined to `false` (matching
 *  the original `!!msg.foo`); `moveThreshold` falls back to the
 *  default. */
export function stateFromInit(msg: InitMsg): WorkerSimState {
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: msg.moveThreshold ?? DEFAULT_STATE.moveThreshold,
    legacyTickOffset: msg.legacyTickOffset === true,
    legacyFullPrecision: msg.legacyFullPrecision === true,
    legacyPortWrap: msg.legacyPortWrap === true,
    legacyOpcodeSet: msg.legacyOpcodeSet === true,
  };
}

/** Reducer: applies a `config` message to existing state. `coeff` and
 *  `k` are always overwritten (they're required on `ConfigMsg`); the
 *  remaining fields only update if present. */
export function applyConfig(state: WorkerSimState, msg: ConfigMsg): WorkerSimState {
  return {
    coeff: msg.coeff,
    k: msg.k,
    moveThreshold: typeof msg.moveThreshold === 'number'
      ? msg.moveThreshold
      : state.moveThreshold,
    legacyTickOffset: typeof msg.legacyTickOffset === 'boolean'
      ? msg.legacyTickOffset
      : state.legacyTickOffset,
    legacyFullPrecision: typeof msg.legacyFullPrecision === 'boolean'
      ? msg.legacyFullPrecision
      : state.legacyFullPrecision,
    legacyPortWrap: typeof msg.legacyPortWrap === 'boolean'
      ? msg.legacyPortWrap
      : state.legacyPortWrap,
    legacyOpcodeSet: typeof msg.legacyOpcodeSet === 'boolean'
      ? msg.legacyOpcodeSet
      : state.legacyOpcodeSet,
  };
}

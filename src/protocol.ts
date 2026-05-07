// Worker ↔ main thread message protocol for the Aenternis WASM viewer.
//
// The viewer runs the simulation in a Web Worker (`web/worker.ts`) and
// the renderer on the main thread (`web/main.ts`). They communicate via
// `postMessage` with a fixed wire format. This module is the
// single source of truth for that format: discriminated unions for
// each direction plus a small program-normalization helper.
//
// Pure data. No DOM, no Worker globals, no THREE — fully unit-testable
// in Node.

// ---- Main → Worker ----------------------------------------------------------

export interface InitMsg {
  readonly type: 'init';
  readonly seed: number;
  readonly energy: number;
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold?: number;
  readonly program?: Uint32Array | readonly number[];
}

export interface ConfigMsg {
  readonly type: 'config';
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold?: number;
}

export interface RunningMsg {
  readonly type: 'running';
  readonly running: boolean;
}

export interface InspectMsg {
  readonly type: 'inspect';
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

export type MainToWorkerMsg = InitMsg | ConfigMsg | RunningMsg | InspectMsg;

// ---- Worker → Main ----------------------------------------------------------

export interface ReadyMsg {
  readonly type: 'ready';
}

export interface SnapshotMsg {
  readonly type: 'snapshot';
  readonly tick: number;
  readonly cellCount: number;
  readonly totalEnergy: number;
  readonly msPerTick: number;
  readonly snap: Uint32Array;
  readonly stride: number;
  readonly bbox: Int32Array;
}

export interface CellDetailMsg {
  readonly type: 'cellDetail';
  readonly x: number;
  readonly y: number;
  readonly z: number;
  readonly tick: number;
  readonly data: Uint32Array;
  readonly prefix: number;
}

export type WorkerToMainMsg = ReadyMsg | SnapshotMsg | CellDetailMsg;

// ---- Helpers ---------------------------------------------------------------

/** Normalize an `InitMsg.program` field to a `Uint32Array`. Accepts a
 *  pre-built typed array (returned as-is), a plain number array (copied),
 *  or `undefined` (returns an empty `Uint32Array`). */
export function normalizeProgram(
  value: InitMsg['program'],
): Uint32Array {
  if (value instanceof Uint32Array) return value;
  return new Uint32Array(value ?? []);
}

/** Type guard for `MainToWorkerMsg`. Used at the worker boundary to
 *  validate untrusted `MessageEvent.data`. */
export function isMainToWorkerMsg(value: unknown): value is MainToWorkerMsg {
  if (typeof value !== 'object' || value === null) return false;
  const t = (value as { readonly type?: unknown }).type;
  return t === 'init' || t === 'config' || t === 'running' || t === 'inspect';
}

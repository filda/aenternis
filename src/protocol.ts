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
  // Gravity / pressure physics (see docs/gravity-plan.md). All optional;
  // omitted fields fall back to the engine defaults (gravity/pressure off).
  readonly gravity?: number;
  readonly gravityAlpha?: number;
  readonly pressure?: number;
  readonly pressureGamma?: number;
  readonly pressureEref?: number;
  readonly baseMutationRate?: number;
  readonly program?: Uint32Array | readonly number[];
}

export interface ConfigMsg {
  readonly type: 'config';
  readonly coeff: number;
  readonly k: number;
  readonly moveThreshold?: number;
  readonly gravity?: number;
  readonly gravityAlpha?: number;
  readonly pressure?: number;
  readonly pressureGamma?: number;
  readonly pressureEref?: number;
  readonly baseMutationRate?: number;
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

/** Single-step request: advance the world by exactly one tick and emit
 *  one snapshot, regardless of the worker's `running` flag. Used by the
 *  Tick button while the loop is paused. */
export interface StepMsg {
  readonly type: 'step';
}

export type MainToWorkerMsg = InitMsg | ConfigMsg | RunningMsg | InspectMsg | StepMsg;

// ---- Worker → Main ----------------------------------------------------------

export interface ReadyMsg {
  readonly type: 'ready';
}

/** Server-side bootstrap message sent right after `ready` over the
 *  native WebSocket transport. Tells a fresh viewer whether the
 *  shared world it just joined is currently ticking, so the
 *  Pause/Resume button reflects reality instead of the page's
 *  default `running=false`. The Web Worker (WASM) transport never
 *  emits this message — workers always start with their world held
 *  on tick 0, so the default is correct there. */
export interface WelcomeMsg {
  readonly type: 'welcome';
  readonly running: boolean;
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

export type WorkerToMainMsg = ReadyMsg | WelcomeMsg | SnapshotMsg | CellDetailMsg;

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
  return t === 'init' || t === 'config' || t === 'running' || t === 'inspect' || t === 'step';
}

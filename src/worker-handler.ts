// Message handler factory for the WASM viewer worker.
//
// Owns the per-tick step loop and the `World` instance, but takes its
// side-effecting dependencies (postMessage, scheduling, timing) by
// injection. The point of the factory shape is testability: in
// production the dependencies wrap real Worker globals (see
// `web/worker.ts`); in tests they're spies / fakes.

import {
  type CellDetailMsg,
  type MainToWorkerMsg,
  normalizeProgram,
  type SnapshotMsg,
  type WorkerToMainMsg,
} from './protocol.ts';
import {
  applyConfig,
  DEFAULT_STATE,
  stateFromInit,
  type WorkerSimState,
} from './worker-state.ts';

/** Minimal subset of the `wasm-bindgen`-generated `World` API that the
 *  handler depends on. The real `World` from
 *  `crates/aenternis-wasm/pkg/aenternis_wasm.js` is structurally
 *  compatible. */
export interface WorldHandle {
  free(): void;
  setMoveThreshold(t: number): void;
  step(coeff: number, k: number): void;
  cellsSnapshot(): Uint32Array;
  boundingBox(): Int32Array;
  tick(): number;
  cellCount(): number;
  totalEnergy(): number;
  cellInspect(x: number, y: number, z: number): Uint32Array;
  readonly snapshotStride: number;
  readonly inspectPrefix: number;
}

/** Factory shape that the handler uses to instantiate a World. The
 *  WASM-generated `World` class exposes `newWithProgram` as a static
 *  method, so a class object satisfies this interface. */
export interface WorldFactory {
  newWithProgram(seed: number, energy: number, program: Uint32Array): WorldHandle;
}

export interface WorkerHandlerDeps {
  readonly worldFactory: WorldFactory;
  readonly postMessage: (msg: WorkerToMainMsg, transfer?: readonly Transferable[]) => void;
  /** Schedule `cb` for execution after the current message handler
   *  returns. In production `setTimeout(cb, 0)`; in tests, a spy. */
  readonly scheduleNext: (cb: () => void) => void;
  /** Monotonic millisecond clock. In production `performance.now`. */
  readonly now: () => number;
}

export interface WorkerHandler {
  readonly handleMessage: (msg: MainToWorkerMsg) => void;
}

export function createWorkerHandler(deps: WorkerHandlerDeps): WorkerHandler {
  let world: WorldHandle | null = null;
  let running = false;
  let state: WorkerSimState = DEFAULT_STATE;
  let lastMsPerTick = 0;

  function applyStateToWorld(w: WorldHandle, s: WorkerSimState): void {
    w.setMoveThreshold(s.moveThreshold);
  }

  // Both `send*` helpers take the live `World` as an argument rather
  // than reading the closed-over `world` variable; callers always know
  // the instance is non-null (they've checked it or just created it).
  // This avoids a redundant null guard whose untestable branch would
  // otherwise leak as a surviving mutant.

  function sendSnapshot(w: WorldHandle): void {
    const snap = w.cellsSnapshot();
    const bbox = w.boundingBox();
    const msg: SnapshotMsg = {
      type: 'snapshot',
      tick: w.tick(),
      cellCount: w.cellCount(),
      totalEnergy: w.totalEnergy(),
      msPerTick: lastMsPerTick,
      snap,
      stride: w.snapshotStride,
      bbox,
    };
    deps.postMessage(msg, [snap.buffer, bbox.buffer]);
  }

  function sendCellDetail(w: WorldHandle, x: number, y: number, z: number): void {
    const data = w.cellInspect(x, y, z);
    const msg: CellDetailMsg = {
      type: 'cellDetail',
      x,
      y,
      z,
      tick: w.tick(),
      data,
      prefix: w.inspectPrefix,
    };
    deps.postMessage(msg, [data.buffer]);
  }

  function loop(): void {
    const w = world;
    if (!w || !running) return;
    const t0 = deps.now();
    w.step(state.coeff, state.k);
    lastMsPerTick = deps.now() - t0;
    sendSnapshot(w);
    deps.scheduleNext(loop);
  }

  function handleMessage(msg: MainToWorkerMsg): void {
    if (msg.type === 'init') {
      if (world) world.free();
      const program = normalizeProgram(msg.program);
      const w = deps.worldFactory.newWithProgram(msg.seed, msg.energy, program);
      world = w;
      state = stateFromInit(msg);
      applyStateToWorld(w, state);
      running = true;
      sendSnapshot(w); // initial state, before any tick has run
      deps.scheduleNext(loop);
      return;
    }
    if (msg.type === 'config') {
      state = applyConfig(state, msg);
      if (world) {
        // Apply each per-flag setter conditionally (matches the
        // original behaviour: only push to `world` when the field is
        // actually present on the message).
        if (typeof msg.moveThreshold === 'number') {
          world.setMoveThreshold(state.moveThreshold);
        }
      }
      return;
    }
    if (msg.type === 'running') {
      const wasRunning = running;
      running = msg.running;
      if (running && !wasRunning) deps.scheduleNext(loop);
      return;
    }
    // msg.type === 'inspect'
    if (world) sendCellDetail(world, msg.x, msg.y, msg.z);
  }

  return { handleMessage };
}

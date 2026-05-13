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

/** Rolling-window size for the per-tick profile log emitted to the
 *  worker's console. Every `PROFILE_WINDOW` ticks the handler prints a
 *  one-line summary of step / snapshot timing — read from the browser
 *  DevTools console to decide whether `plan-wasm-zerocopy-threads`
 *  parts A (zero-copy `Uint32Array::view`) or B (multi-threaded WASM)
 *  are worth implementing. Throw-away instrumentation; remove once the
 *  measurement is done and the decision is committed. */
const PROFILE_WINDOW = 100;

export function createWorkerHandler(deps: WorkerHandlerDeps): WorkerHandler {
  let world: WorldHandle | null = null;
  let running = false;
  let state: WorkerSimState = DEFAULT_STATE;
  let lastMsPerTick = 0;
  let lastSnapshotMs = 0;

  // Profile-window accumulators. Reset every `PROFILE_WINDOW` ticks
  // after a console log fires. Sums are over the window; counts are
  // the number of contributing ticks (always `PROFILE_WINDOW` once
  // armed, but kept explicit so a partial window on shutdown is still
  // averaged correctly).
  let profileTicks = 0;
  let profileStepMsSum = 0;
  let profileSnapshotMsSum = 0;
  let profileSnapshotBytesLast = 0;

  function applyStateToWorld(w: WorldHandle, s: WorkerSimState): void {
    w.setMoveThreshold(s.moveThreshold);
  }

  // Both `send*` helpers take the live `World` as an argument rather
  // than reading the closed-over `world` variable; callers always know
  // the instance is non-null (they've checked it or just created it).
  // This avoids a redundant null guard whose untestable branch would
  // otherwise leak as a surviving mutant.

  function sendSnapshot(w: WorldHandle): void {
    const tSnap = deps.now();
    const snap = w.cellsSnapshot();
    lastSnapshotMs = deps.now() - tSnap;
    profileSnapshotBytesLast = snap.byteLength;
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

  /** Advance the world by one tick, capture timing, and emit one
   *  snapshot. Shared between the auto-running `loop` and the on-demand
   *  `step` message so single-step and run-mode behave identically. */
  function stepOnce(w: WorldHandle): void {
    const t0 = deps.now();
    w.step(state.coeff, state.k);
    lastMsPerTick = deps.now() - t0;
    sendSnapshot(w);

    // Profile-window accumulation. `sendSnapshot` set `lastSnapshotMs`
    // and `profileSnapshotBytesLast` as side-effects above.
    profileStepMsSum += lastMsPerTick;
    profileSnapshotMsSum += lastSnapshotMs;
    profileTicks += 1;
    if (profileTicks >= PROFILE_WINDOW) {
      const stepAvg = profileStepMsSum / profileTicks;
      const snapAvg = profileSnapshotMsSum / profileTicks;
      const cells = w.cellCount();
      const snapKB = profileSnapshotBytesLast / 1024;
      // eslint-disable-next-line no-console -- throwaway profile log
      console.log(
        `[profile] tick=${w.tick()} cells=${cells} ` +
          `step=${stepAvg.toFixed(2)}ms snap=${snapAvg.toFixed(2)}ms ` +
          `snap_bytes=${snapKB.toFixed(0)}KB ` +
          `frame_budget=${(stepAvg + snapAvg).toFixed(2)}ms ` +
          `(${(((stepAvg + snapAvg) / 16.67) * 100).toFixed(0)}% of 60Hz)`,
      );
      profileTicks = 0;
      profileStepMsSum = 0;
      profileSnapshotMsSum = 0;
    }
  }

  function loop(): void {
    const w = world;
    if (!w || !running) return;
    stepOnce(w);
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
    if (msg.type === 'step') {
      // Single-step ignores `running` by design — the Tick button is
      // useful exactly when the auto-loop is paused. No `scheduleNext`:
      // the caller controls when the next tick happens.
      if (world) stepOnce(world);
      return;
    }
    // msg.type === 'inspect'
    if (world) sendCellDetail(world, msg.x, msg.y, msg.z);
  }

  return { handleMessage };
}

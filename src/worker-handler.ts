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
 *  compatible.
 *
 *  `cellsSnapshotView` and `cellInspectView` return `Uint32Array`s
 *  that **alias WASM linear memory** — callers must copy the data
 *  (e.g. `new Uint32Array(view)`) before any further WASM call. The
 *  view's `.buffer` is the WASM memory itself; never transfer it via
 *  `postMessage`. See `docs/optimalizace-2026-05.md`. */
export interface WorldHandle {
  free(): void;
  setMoveThreshold(t: number): void;
  step(coeff: number, k: number): void;
  cellsSnapshotView(): Uint32Array;
  boundingBox(): Int32Array;
  tick(): number;
  cellCount(): number;
  totalEnergy(): number;
  cellInspectView(x: number, y: number, z: number): Uint32Array;
  /** Diagnostic snapshot of every container's allocated size on the
   *  Rust-side world plus the current WASM linear-memory page count.
   *  Returns a 21-`u32` flat array; layout documented on the Rust
   *  `World::memoryReport` method and validated against
   *  `memoryReportLen` at the worker boundary. */
  memoryReport(): Uint32Array;
  readonly snapshotStride: number;
  readonly inspectPrefix: number;
  readonly memoryReportLen: number;
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

/** Cadence (in ticks) of the diagnostic `memoryReport` console dump.
 *  50 keeps the worker console readable across a few thousand ticks
 *  while still catching pre-OOM growth that builds up over hundreds
 *  of ticks. Lives at module scope so tests can override the
 *  emission cadence by patching this constant if needed. */
const MEMORY_REPORT_EVERY = 50;

/** Field labels for the flat `u32` array returned by
 *  `WorldHandle.memoryReport`. The order *must* match the Rust-side
 *  `World::memory_report` layout (documented on that method); a length
 *  mismatch is caught at runtime against `memoryReportLen` before
 *  printing. */
const MEMORY_REPORT_LABELS: readonly string[] = [
  'wasm_memory_pages',
  'tick',
  'cell_count',
  'cells_slots_len',
  'cells_slots_cap',
  'cells_free_slots_len',
  'cells_free_slots_cap',
  'cells_coord_to_slot_cap',
  'scratch_neighbor_energies_cap',
  'scratch_outflow_cap',
  'scratch_outflow_inner_vec_cap_sum',
  'scratch_inflows_by_target_cap',
  'scratch_inflows_inner_vec_cap_sum',
  'scratch_per_source_total_outflow_cap',
  'sorted_cache_len',
  'sorted_cache_cap',
  'arena_capacity',
  'arena_slots_vec_cap',
  'arena_next_capacity',
  'arena_next_slots_vec_cap',
  'reserved',
];

/** Print a single `[mem-report]` console line summarizing every
 *  container size in the WASM world. Intentionally one line: keeps the
 *  DevTools console grep-friendly (`tick=` / `arena_capacity=`) and
 *  lets a long simulation be diffed line-by-line without per-field
 *  log inflation.
 *
 *  Length-checks against `memoryReportLen` so a future Rust-side
 *  layout change that drops the labels out of sync is caught at the
 *  boundary rather than producing silently mislabeled values. */
function logMemoryReport(w: WorldHandle): void {
  const data = w.memoryReport();
  if (data.length !== w.memoryReportLen || data.length !== MEMORY_REPORT_LABELS.length) {
    // eslint-disable-next-line no-console
    console.warn(
      `[mem-report] layout mismatch: data.length=${data.length} ` +
        `memoryReportLen=${w.memoryReportLen} ` +
        `labels.length=${MEMORY_REPORT_LABELS.length}`,
    );
    return;
  }
  const parts: string[] = ['[mem-report]'];
  for (let i = 0; i < data.length; i += 1) {
    parts.push(`${MEMORY_REPORT_LABELS[i]}=${data[i]}`);
  }
  // eslint-disable-next-line no-console
  console.log(parts.join(' '));
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
    // `cellsSnapshotView` aliases WASM linear memory; copy out into an
    // owned `Uint32Array` *before* any further WASM call (the next call
    // may grow memory or reallocate the underlying `Vec`, invalidating
    // the view) and *before* the `postMessage` transfer (the view's
    // `.buffer` is the WASM memory — transferring it would detach it).
    const view = w.cellsSnapshotView();
    const snap = new Uint32Array(view);
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
    // Same WASM-memory-aliasing contract as `sendSnapshot` — copy
    // before any further WASM call or the postMessage transfer.
    const view = w.cellInspectView(x, y, z);
    const data = new Uint32Array(view);
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
   *  `step` message so single-step and run-mode behave identically.
   *
   *  Diagnostic: every `MEMORY_REPORT_EVERY` ticks, dump the world's
   *  container sizes (via `WorldHandle.memoryReport`) to the worker
   *  console so the page's DevTools can plot the trajectory leading up
   *  to a WASM-side OOM trap. Cheap enough at the chosen cadence —
   *  the cost is one `Uint32Array` of 21 elements plus an `O(cells)`
   *  walk inside Rust for the nested-scratch inner-Vec sum. */
  function stepOnce(w: WorldHandle): void {
    const t0 = deps.now();
    w.step(state.coeff, state.k);
    lastMsPerTick = deps.now() - t0;
    sendSnapshot(w);
    const tick = w.tick();
    if (tick > 0 && tick % MEMORY_REPORT_EVERY === 0) {
      logMemoryReport(w);
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

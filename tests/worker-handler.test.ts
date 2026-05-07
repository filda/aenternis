import { describe, it, expect, vi } from 'vitest';
import type {
  ConfigMsg,
  InitMsg,
  InspectMsg,
  RunningMsg,
} from '../src/protocol.ts';
import {
  createWorkerHandler,
  type WorkerHandlerDeps,
  type WorldFactory,
  type WorldHandle,
} from '../src/worker-handler.ts';

// ---- Test fixtures ---------------------------------------------------------

function makeMockWorld(): WorldHandle {
  return {
    free: vi.fn(),
    setMoveThreshold: vi.fn(),
    step: vi.fn(),
    cellsSnapshot: vi.fn(() => new Uint32Array([10, 20, 30, 40])),
    boundingBox: vi.fn(() => new Int32Array([0, 0, 0, 1, 1, 1])),
    tick: vi.fn(() => 42),
    cellCount: vi.fn(() => 5),
    totalEnergy: vi.fn(() => 10_000),
    cellInspect: vi.fn(() => new Uint32Array([0xCAFE, 0xBABE])),
    snapshotStride: 4,
    inspectPrefix: 8,
  };
}

interface Harness {
  readonly handler: ReturnType<typeof createWorkerHandler>;
  readonly deps: {
    readonly worldFactory: WorldFactory & { newWithProgram: ReturnType<typeof vi.fn> };
    readonly postMessage: ReturnType<typeof vi.fn>;
    readonly scheduleNext: ReturnType<typeof vi.fn>;
    readonly now: ReturnType<typeof vi.fn>;
  };
  /** Mirror of every callback passed to `scheduleNext`. Lives outside
   *  the spy so that `scheduleNext.mockClear()` doesn't erase the
   *  reference we need to drive the loop in tests. */
  readonly scheduled: Array<() => void>;
  readonly world: WorldHandle;
}

function makeHarness(overrides?: { now?: () => number }): Harness {
  const world = makeMockWorld();
  const factory = vi.fn(() => world);
  const worldFactory = { newWithProgram: factory };
  const postMessage = vi.fn();
  const scheduled: Array<() => void> = [];
  const scheduleNext = vi.fn((cb: () => void) => { scheduled.push(cb); });
  const now = vi.fn(overrides?.now ?? (() => 0));
  const deps: WorkerHandlerDeps = {
    worldFactory,
    postMessage,
    scheduleNext,
    now,
  };
  const handler = createWorkerHandler(deps);
  return {
    handler,
    deps: { worldFactory, postMessage, scheduleNext, now },
    scheduled,
    world,
  };
}

const baseInit: InitMsg = {
  type: 'init',
  seed: 1234,
  energy: 10_000_000,
  coeff: 0.15,
  k: 1,
};

// ---- init -------------------------------------------------------------------

describe('createWorkerHandler — init', () => {
  it('builds a World via the factory with seed, energy and an empty program', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    expect(h.deps.worldFactory.newWithProgram).toHaveBeenCalledTimes(1);
    const args = h.deps.worldFactory.newWithProgram.mock.calls[0]!;
    expect(args[0]).toBe(1234);
    expect(args[1]).toBe(10_000_000);
    expect(args[2]).toBeInstanceOf(Uint32Array);
    expect(args[2]).toHaveLength(0);
  });

  it('normalizes a number-array program into a Uint32Array', () => {
    const h = makeHarness();
    h.handler.handleMessage({ ...baseInit, program: [0x09, 0x00, 0x00] });
    const program = h.deps.worldFactory.newWithProgram.mock.calls[0]![2];
    expect(program).toBeInstanceOf(Uint32Array);
    expect(Array.from(program)).toEqual([9, 0, 0]);
  });

  it('passes a Uint32Array program through unchanged', () => {
    const h = makeHarness();
    const program = new Uint32Array([1, 2, 3]);
    h.handler.handleMessage({ ...baseInit, program });
    expect(h.deps.worldFactory.newWithProgram.mock.calls[0]![2]).toBe(program);
  });

  it('applies all state setters to the new world', () => {
    const h = makeHarness();
    h.handler.handleMessage({ ...baseInit, moveThreshold: 2.5 });
    expect(h.world.setMoveThreshold).toHaveBeenCalledWith(2.5);
  });

  it('emits an initial snapshot with transferable buffers', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    expect(h.deps.postMessage).toHaveBeenCalledTimes(1);
    const [msg, transfer] = h.deps.postMessage.mock.calls[0]!;
    expect(msg).toMatchObject({
      type: 'snapshot',
      tick: 42,
      cellCount: 5,
      totalEnergy: 10_000,
      stride: 4,
    });
    expect(transfer).toHaveLength(2);
  });

  it('schedules the first tick loop', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    expect(h.deps.scheduleNext).toHaveBeenCalledTimes(1);
  });

  it('frees the previous world when re-initialized', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    const firstWorld = h.world;
    // A second init triggers the factory again; the first world is freed.
    h.deps.worldFactory.newWithProgram.mockReturnValueOnce(makeMockWorld());
    h.handler.handleMessage(baseInit);
    expect(firstWorld.free).toHaveBeenCalledTimes(1);
  });
});

// ---- config -----------------------------------------------------------------

describe('createWorkerHandler — config', () => {
  it('updates coeff and k on the next tick', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'config', coeff: 0.42, k: 7 });
    // Drive one loop iteration to observe the new values being passed
    // to `world.step`.
    runScheduledLoop(h);
    expect(h.world.step).toHaveBeenCalledWith(0.42, 7);
  });

  it('does not call setMoveThreshold when no moveThreshold is given', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    vi.mocked(h.world.setMoveThreshold).mockClear();

    h.handler.handleMessage({ type: 'config', coeff: 0.1, k: 1 });

    expect(h.world.setMoveThreshold).not.toHaveBeenCalled();
  });

  it('forwards moveThreshold to the world only when given', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    vi.mocked(h.world.setMoveThreshold).mockClear();
    h.handler.handleMessage({ type: 'config', coeff: 0.1, k: 1, moveThreshold: 3.3 });
    expect(h.world.setMoveThreshold).toHaveBeenCalledWith(3.3);
  });

  it('forwards moveThreshold of 0 (truthy guard regression)', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    vi.mocked(h.world.setMoveThreshold).mockClear();
    h.handler.handleMessage({ type: 'config', coeff: 0.1, k: 1, moveThreshold: 0 });
    expect(h.world.setMoveThreshold).toHaveBeenCalledWith(0);
  });

  it('does nothing on the world side when config arrives before init', () => {
    const h = makeHarness();
    const cfg: ConfigMsg = { type: 'config', coeff: 0.42, k: 7, moveThreshold: 1.0 };
    h.handler.handleMessage(cfg);
    // No world was created, so no setters can have been called.
    expect(h.deps.postMessage).not.toHaveBeenCalled();
    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });
});

// ---- running ----------------------------------------------------------------

describe('createWorkerHandler — running', () => {
  it('does not schedule a new tick when the handler is paused (false → false)', () => {
    const h = makeHarness();
    const msg: RunningMsg = { type: 'running', running: false };
    h.handler.handleMessage(msg);
    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });

  it('schedules a tick on first running:true even before init', () => {
    // Regression: the initial value of the internal `running` flag
    // must be `false`, otherwise `running && !wasRunning` short-circuits
    // and never schedules.
    const h = makeHarness();
    h.handler.handleMessage({ type: 'running', running: true });
    expect(h.deps.scheduleNext).toHaveBeenCalledTimes(1);
  });

  it('does not schedule a duplicate tick when already running (true → true)', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit); // running=true after init
    h.deps.scheduleNext.mockClear();
    h.handler.handleMessage({ type: 'running', running: true });
    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });

  it('schedules a tick on resume (false → true)', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'running', running: false });
    h.deps.scheduleNext.mockClear();
    h.handler.handleMessage({ type: 'running', running: true });
    expect(h.deps.scheduleNext).toHaveBeenCalledTimes(1);
  });

  it('stops the loop on pause (true → false)', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'running', running: false });
    h.deps.scheduleNext.mockClear();
    runScheduledLoop(h); // loop was already scheduled by init; runs once
    // After pause, the loop's running check returns early — neither
    // step nor a fresh schedule should follow.
    expect(h.world.step).not.toHaveBeenCalled();
    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });
});

// ---- step -------------------------------------------------------------------

describe('createWorkerHandler — step', () => {
  it('advances the world by one tick and posts a snapshot when paused', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'running', running: false });
    vi.mocked(h.world.step).mockClear();
    h.deps.postMessage.mockClear();
    h.deps.scheduleNext.mockClear();

    h.handler.handleMessage({ type: 'step' });

    expect(h.world.step).toHaveBeenCalledTimes(1);
    expect(h.deps.postMessage).toHaveBeenCalledTimes(1);
    expect(h.deps.postMessage.mock.calls[0]![0]).toMatchObject({ type: 'snapshot' });
  });

  it('uses the current state.coeff and state.k for the step', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'config', coeff: 0.42, k: 7 });
    h.handler.handleMessage({ type: 'running', running: false });
    vi.mocked(h.world.step).mockClear();

    h.handler.handleMessage({ type: 'step' });

    expect(h.world.step).toHaveBeenCalledWith(0.42, 7);
  });

  it('does not schedule a follow-up loop iteration', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'running', running: false });
    h.deps.scheduleNext.mockClear();

    h.handler.handleMessage({ type: 'step' });

    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });

  it('updates msPerTick from the now() clock on the manual step', () => {
    let t = 0;
    const h = makeHarness({ now: () => { t += 3; return t; } });
    h.handler.handleMessage(baseInit);
    h.handler.handleMessage({ type: 'running', running: false });
    h.deps.postMessage.mockClear();

    h.handler.handleMessage({ type: 'step' });

    const snap = h.deps.postMessage.mock.calls[0]![0];
    expect(snap).toMatchObject({ msPerTick: 3 });
  });

  it('steps even when running is true (caller is responsible for ordering)', () => {
    // The Tick button auto-pauses before sending step, but the worker
    // itself does not depend on the running flag — single-step is a
    // pure function of the world.
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    vi.mocked(h.world.step).mockClear();

    h.handler.handleMessage({ type: 'step' });

    expect(h.world.step).toHaveBeenCalledTimes(1);
  });

  it('does nothing when step arrives before init', () => {
    const h = makeHarness();
    h.handler.handleMessage({ type: 'step' });
    expect(h.deps.postMessage).not.toHaveBeenCalled();
    expect(h.deps.scheduleNext).not.toHaveBeenCalled();
  });
});

// ---- inspect ----------------------------------------------------------------

describe('createWorkerHandler — inspect', () => {
  it('emits a cellDetail message with transferable data', () => {
    const h = makeHarness();
    h.handler.handleMessage(baseInit);
    h.deps.postMessage.mockClear();

    const inspect: InspectMsg = { type: 'inspect', x: 1, y: 2, z: 3 };
    h.handler.handleMessage(inspect);

    expect(h.world.cellInspect).toHaveBeenCalledWith(1, 2, 3);
    const [msg, transfer] = h.deps.postMessage.mock.calls[0]!;
    expect(msg).toMatchObject({
      type: 'cellDetail',
      x: 1, y: 2, z: 3,
      tick: 42,
      prefix: 8,
    });
    expect(transfer).toHaveLength(1);
  });

  it('does nothing when inspect arrives before init', () => {
    const h = makeHarness();
    h.handler.handleMessage({ type: 'inspect', x: 0, y: 0, z: 0 });
    expect(h.deps.postMessage).not.toHaveBeenCalled();
  });
});

// ---- tick loop --------------------------------------------------------------

describe('createWorkerHandler — tick loop', () => {
  it('on each tick, steps the world and posts a fresh snapshot', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.deps.postMessage.mockClear();
    runScheduledLoop(h);
    expect(h.world.step).toHaveBeenCalledTimes(1);
    expect(h.deps.postMessage).toHaveBeenCalledTimes(1);
    expect(h.deps.postMessage.mock.calls[0]![0]).toMatchObject({ type: 'snapshot' });
  });

  it('schedules the next loop after each tick', () => {
    const h = makeHarness({ now: makeMonotonicNow() });
    h.handler.handleMessage(baseInit);
    h.deps.scheduleNext.mockClear();
    runScheduledLoop(h);
    expect(h.deps.scheduleNext).toHaveBeenCalledTimes(1);
  });

  it('reports msPerTick from the now() clock', () => {
    let t = 0;
    const h = makeHarness({ now: () => { t += 5; return t; } });
    // After init, t=5 (snapshot doesn't sample the clock).
    h.handler.handleMessage(baseInit);
    h.deps.postMessage.mockClear();
    runScheduledLoop(h);
    // loop: t0=10, step, t=15, msPerTick = 5
    const snap = h.deps.postMessage.mock.calls[0]![0];
    expect(snap).toMatchObject({ msPerTick: 5 });
  });
});

// ---- helpers ----------------------------------------------------------------

/** Returns a `now`-like function whose return value increases by 1 each
 *  call, so anything timing-related is deterministic and non-zero. */
function makeMonotonicNow(): () => number {
  let t = 0;
  return () => { t += 1; return t; };
}

/** Invokes the most recently scheduled callback, simulating one tick
 *  of the worker loop. Reads from `harness.scheduled` (not the spy
 *  history) so tests can `mockClear` the spy without losing the
 *  callback handle. */
function runScheduledLoop(h: Harness): void {
  const cb = h.scheduled[h.scheduled.length - 1];
  if (!cb) throw new Error('scheduleNext was never called');
  cb();
}

import { describe, expect, it } from 'vitest';

import {
  CELL_DETAIL_TAG,
  SNAPSHOT_TAG,
  type IncomingFromServer,
  type WebSocketLike,
  createNativeClient,
  decodeIncoming,
  encodeOutgoing,
} from '../src/native-client.ts';
import type { MainToWorkerMsg, WorkerToMainMsg } from '../src/protocol.ts';

/** Minimal MockWebSocket that records outbound frames and exposes
 *  hooks for the tests to drive `onopen` / `onmessage` / `onclose`. */
class MockWebSocket implements WebSocketLike {
  readonly url: string;
  binaryType: string = '';
  readonly sent: (string | ArrayBuffer)[] = [];
  closed = false;
  onmessage: ((ev: { readonly data: string | ArrayBuffer }) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((err: unknown) => void) | null = null;

  constructor(url: string) {
    this.url = url;
  }

  send(data: string | ArrayBuffer | Blob | ArrayBufferView): void {
    if (typeof data === 'string' || data instanceof ArrayBuffer) {
      this.sent.push(data);
    } else {
      throw new Error('test mock only handles string / ArrayBuffer');
    }
  }

  close(): void {
    this.closed = true;
    this.onclose?.();
  }

  // Test-side drivers.
  triggerOpen(): void {
    this.onopen?.();
  }
  triggerMessage(data: string | ArrayBuffer): void {
    this.onmessage?.({ data });
  }
}

/** Build a snapshot binary frame with the layout the server emits. */
function buildSnapshotFrame(opts: {
  tick: number;
  cellCount: number;
  totalEnergy: number;
  msPerTick: number;
  stride: number;
  bbox: readonly [number, number, number, number, number, number];
  snap: readonly number[];
}): ArrayBuffer {
  const HEADER = 49;
  const buf = new ArrayBuffer(HEADER + opts.snap.length * 4);
  const view = new DataView(buf);
  view.setUint8(0, SNAPSHOT_TAG);
  view.setUint32(1, opts.tick, true);
  view.setUint32(5, opts.cellCount, true);
  view.setUint32(9, opts.totalEnergy, true);
  view.setFloat64(13, opts.msPerTick, true);
  view.setUint32(21, opts.stride, true);
  for (let i = 0; i < 6; i += 1) view.setInt32(25 + i * 4, opts.bbox[i]!, true);
  for (let i = 0; i < opts.snap.length; i += 1) view.setUint32(HEADER + i * 4, opts.snap[i]!, true);
  return buf;
}

/** Build a cellDetail binary frame matching the server layout. */
function buildCellDetailFrame(opts: {
  x: number;
  y: number;
  z: number;
  tick: number;
  prefix: number;
  data: readonly number[];
}): ArrayBuffer {
  const HEADER = 25;
  const buf = new ArrayBuffer(HEADER + opts.data.length * 4);
  const view = new DataView(buf);
  view.setUint8(0, CELL_DETAIL_TAG);
  view.setInt32(1, opts.x, true);
  view.setInt32(5, opts.y, true);
  view.setInt32(9, opts.z, true);
  view.setUint32(13, opts.tick, true);
  view.setUint32(17, opts.prefix, true);
  view.setUint32(21, opts.data.length, true);
  for (let i = 0; i < opts.data.length; i += 1) view.setUint32(HEADER + i * 4, opts.data[i]!, true);
  return buf;
}

// ---- encodeOutgoing --------------------------------------------------------

describe('encodeOutgoing', () => {
  it('serializes Init without program as JSON without the field', () => {
    const msg: MainToWorkerMsg = {
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
    };
    const json = encodeOutgoing(msg);
    expect(JSON.parse(json)).toEqual({
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
    });
  });

  it('serializes Init with Uint32Array program as a plain array', () => {
    const msg: MainToWorkerMsg = {
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: new Uint32Array([10, 20, 30]),
    };
    const json = encodeOutgoing(msg);
    expect(JSON.parse(json)).toEqual({
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: [10, 20, 30],
    });
  });

  it('serializes Init with number[] program unchanged', () => {
    const msg: MainToWorkerMsg = {
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: [7, 8, 9],
    };
    expect(JSON.parse(encodeOutgoing(msg))).toEqual({
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: [7, 8, 9],
    });
  });

  it('serializes Config with moveThreshold', () => {
    const msg: MainToWorkerMsg = { type: 'config', coeff: 0.3, k: 2, moveThreshold: 1.5 };
    expect(JSON.parse(encodeOutgoing(msg))).toEqual({
      type: 'config',
      coeff: 0.3,
      k: 2,
      moveThreshold: 1.5,
    });
  });

  it('serializes Running', () => {
    const msg: MainToWorkerMsg = { type: 'running', running: true };
    expect(JSON.parse(encodeOutgoing(msg))).toEqual({ type: 'running', running: true });
  });

  it('serializes Step', () => {
    const msg: MainToWorkerMsg = { type: 'step' };
    expect(JSON.parse(encodeOutgoing(msg))).toEqual({ type: 'step' });
  });

  it('serializes Inspect', () => {
    const msg: MainToWorkerMsg = { type: 'inspect', x: -1, y: 2, z: 3 };
    expect(JSON.parse(encodeOutgoing(msg))).toEqual({ type: 'inspect', x: -1, y: 2, z: 3 });
  });
});

// ---- decodeIncoming: text --------------------------------------------------

describe('decodeIncoming text', () => {
  it('parses ready', () => {
    expect(decodeIncoming('{"type":"ready"}')).toEqual({ type: 'ready' });
  });

  it('parses welcome with running=true', () => {
    expect(decodeIncoming('{"type":"welcome","running":true}')).toEqual({
      type: 'welcome',
      running: true,
    });
  });

  it('parses welcome with running=false', () => {
    expect(decodeIncoming('{"type":"welcome","running":false}')).toEqual({
      type: 'welcome',
      running: false,
    });
  });

  it('returns null for malformed JSON', () => {
    expect(decodeIncoming('{not json')).toBeNull();
  });

  it('returns null for unknown type', () => {
    expect(decodeIncoming('{"type":"bogus"}')).toBeNull();
  });

  it('returns null for welcome without running field', () => {
    expect(decodeIncoming('{"type":"welcome"}')).toBeNull();
  });

  it('returns null for welcome with non-boolean running', () => {
    expect(decodeIncoming('{"type":"welcome","running":"yes"}')).toBeNull();
  });

  it('returns null for non-object JSON', () => {
    expect(decodeIncoming('null')).toBeNull();
    expect(decodeIncoming('42')).toBeNull();
    expect(decodeIncoming('"ready"')).toBeNull();
  });

  it('returns null for object without type field', () => {
    expect(decodeIncoming('{"running":true}')).toBeNull();
  });
});

// ---- decodeIncoming: binary -------------------------------------------------

describe('decodeIncoming binary snapshot', () => {
  it('parses an empty snapshot', () => {
    const buf = buildSnapshotFrame({
      tick: 0,
      cellCount: 0,
      totalEnergy: 0,
      msPerTick: 0,
      stride: 6,
      bbox: [0, 0, 0, 0, 0, 0],
      snap: [],
    });
    const msg = decodeIncoming(buf);
    expect(msg?.type).toBe('snapshot');
    if (msg?.type !== 'snapshot') return;
    expect(msg.tick).toBe(0);
    expect(msg.cellCount).toBe(0);
    expect(msg.totalEnergy).toBe(0);
    expect(msg.msPerTick).toBe(0);
    expect(msg.stride).toBe(6);
    expect(Array.from(msg.bbox)).toEqual([0, 0, 0, 0, 0, 0]);
    expect(msg.snap.length).toBe(0);
  });

  it('parses a non-empty snapshot with negative bbox values', () => {
    const snap = [10, 20, 30, 40, 50, 60, 100, 200, 300, 400, 500, 600];
    const buf = buildSnapshotFrame({
      tick: 1234,
      cellCount: 2,
      totalEnergy: 175,
      msPerTick: 4.5,
      stride: 6,
      bbox: [-1, 1, -2, 0, 0, 3],
      snap,
    });
    const msg = decodeIncoming(buf);
    expect(msg?.type).toBe('snapshot');
    if (msg?.type !== 'snapshot') return;
    expect(msg.tick).toBe(1234);
    expect(msg.cellCount).toBe(2);
    expect(msg.totalEnergy).toBe(175);
    expect(msg.msPerTick).toBe(4.5);
    expect(msg.stride).toBe(6);
    expect(Array.from(msg.bbox)).toEqual([-1, 1, -2, 0, 0, 3]);
    expect(Array.from(msg.snap)).toEqual(snap);
  });

  it('returns null if header is truncated', () => {
    const truncated = new ArrayBuffer(20);
    new DataView(truncated).setUint8(0, SNAPSHOT_TAG);
    expect(decodeIncoming(truncated)).toBeNull();
  });

  it('returns null if payload is shorter than cellCount * stride * 4', () => {
    // Header claims cellCount=10, but the actual buffer has only 1
    // cell's worth of payload after the header.
    const buf = buildSnapshotFrame({
      tick: 0,
      cellCount: 10,
      totalEnergy: 0,
      msPerTick: 0,
      stride: 6,
      bbox: [0, 0, 0, 0, 0, 0],
      snap: [1, 2, 3, 4, 5, 6],
    });
    expect(decodeIncoming(buf)).toBeNull();
  });
});

describe('decodeIncoming binary cellDetail', () => {
  it('parses a present cell with prefix data', () => {
    const data = Array.from({ length: 33 }, (_, i) => i + 100);
    const buf = buildCellDetailFrame({ x: -3, y: 4, z: -5, tick: 99, prefix: 28, data });
    const msg = decodeIncoming(buf);
    expect(msg?.type).toBe('cellDetail');
    if (msg?.type !== 'cellDetail') return;
    expect(msg.x).toBe(-3);
    expect(msg.y).toBe(4);
    expect(msg.z).toBe(-5);
    expect(msg.tick).toBe(99);
    expect(msg.prefix).toBe(28);
    expect(Array.from(msg.data)).toEqual(data);
  });

  it('parses an empty cellDetail (no cell at coord)', () => {
    const buf = buildCellDetailFrame({ x: 0, y: 0, z: 0, tick: 1, prefix: 28, data: [] });
    const msg = decodeIncoming(buf);
    expect(msg?.type).toBe('cellDetail');
    if (msg?.type !== 'cellDetail') return;
    expect(msg.data.length).toBe(0);
  });

  it('returns null if header is truncated', () => {
    const truncated = new ArrayBuffer(10);
    new DataView(truncated).setUint8(0, CELL_DETAIL_TAG);
    expect(decodeIncoming(truncated)).toBeNull();
  });

  it('returns null if payload is shorter than dataLen * 4', () => {
    // dataLen=10 but only one u32 of payload.
    const buf = new ArrayBuffer(25 + 4);
    const view = new DataView(buf);
    view.setUint8(0, CELL_DETAIL_TAG);
    view.setInt32(1, 0, true);
    view.setInt32(5, 0, true);
    view.setInt32(9, 0, true);
    view.setUint32(13, 0, true);
    view.setUint32(17, 28, true);
    view.setUint32(21, 10, true);
    expect(decodeIncoming(buf)).toBeNull();
  });
});

describe('decodeIncoming binary misc', () => {
  it('returns null for unknown tag', () => {
    const buf = new ArrayBuffer(50);
    new DataView(buf).setUint8(0, 99);
    expect(decodeIncoming(buf)).toBeNull();
  });

  it('returns null for empty buffer', () => {
    expect(decodeIncoming(new ArrayBuffer(0))).toBeNull();
  });
});

// ---- createNativeClient ----------------------------------------------------

describe('createNativeClient', () => {
  it('forwards postMessage to the WebSocket once open', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    expect(ws.binaryType).toBe('arraybuffer');

    ws.triggerOpen();
    channel.postMessage({ type: 'step' });
    expect(ws.sent).toEqual([JSON.stringify({ type: 'step' })]);
  });

  it('queues postMessage calls before open and flushes on open', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);

    channel.postMessage({ type: 'step' });
    channel.postMessage({ type: 'running', running: true });
    expect(ws.sent).toEqual([]); // not yet sent

    ws.triggerOpen();
    expect(ws.sent).toEqual([
      JSON.stringify({ type: 'step' }),
      JSON.stringify({ type: 'running', running: true }),
    ]);
  });

  it('emits decoded ReadyMsg via channel.onmessage', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    const seen: WorkerToMainMsg[] = [];
    channel.onmessage = (ev) => {
      seen.push(ev.data);
    };

    ws.triggerMessage('{"type":"ready"}');
    expect(seen).toEqual([{ type: 'ready' }]);
  });

  it('emits decoded WelcomeMsg', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    const seen: WorkerToMainMsg[] = [];
    channel.onmessage = (ev) => {
      seen.push(ev.data);
    };

    ws.triggerMessage('{"type":"welcome","running":true}');
    expect(seen).toEqual([{ type: 'welcome', running: true }]);
  });

  it('emits decoded SnapshotMsg from binary frame', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    const seen: WorkerToMainMsg[] = [];
    channel.onmessage = (ev) => {
      seen.push(ev.data);
    };

    const buf = buildSnapshotFrame({
      tick: 7,
      cellCount: 1,
      totalEnergy: 50,
      msPerTick: 1,
      stride: 6,
      bbox: [0, 0, 0, 0, 0, 0],
      snap: [0, 0, 0, 50, 0xaa, 0xbb],
    });
    ws.triggerMessage(buf);

    expect(seen.length).toBe(1);
    const msg = seen[0]!;
    expect(msg.type).toBe('snapshot');
    if (msg.type !== 'snapshot') return;
    expect(msg.tick).toBe(7);
  });

  it('drops malformed frames without invoking onmessage', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    const seen: WorkerToMainMsg[] = [];
    channel.onmessage = (ev) => {
      seen.push(ev.data);
    };

    ws.triggerMessage('{not json');
    ws.triggerMessage(new ArrayBuffer(0));
    const unknownTag = new ArrayBuffer(50);
    new DataView(unknownTag).setUint8(0, 99);
    ws.triggerMessage(unknownTag);

    expect(seen).toEqual([]);
  });

  it('does not enter the Init typed-array path for non-Init messages', () => {
    // Cast through a Config message with a Uint32Array `program`
    // field (which the type system disallows). If the Init type
    // guard is mutated away, encodeOutgoing would call
    // `Array.from` on the typed array and emit a plain `[1,2,3]`.
    // Originál passes the message straight to JSON.stringify, which
    // serializes a Uint32Array as the indexed object form
    // `{"0":1,"1":2,"2":3}`.
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    ws.triggerOpen();
    channel.postMessage({
      type: 'config',
      coeff: 0.1,
      k: 1,
      program: new Uint32Array([1, 2, 3]),
    } as unknown as MainToWorkerMsg);

    const sent = JSON.parse(ws.sent[0] as string) as { program: unknown };
    expect(sent.program).toEqual({ '0': 1, '1': 2, '2': 3 });
  });

  it('passes a Uint32Array program through encodeOutgoing on the wire', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    ws.triggerOpen();

    channel.postMessage({
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: new Uint32Array([1, 2, 3]),
    });

    const sent = ws.sent[0];
    expect(typeof sent).toBe('string');
    expect(JSON.parse(sent as string)).toEqual({
      type: 'init',
      seed: 1,
      energy: 5,
      coeff: 0.15,
      k: 1,
      program: [1, 2, 3],
    });
  });

  it('terminate() closes the underlying WebSocket', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    expect(ws.closed).toBe(false);
    channel.terminate();
    expect(ws.closed).toBe(true);
  });

  it('starts onmessage at null so the viewer can hook it lazily', () => {
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);
    expect(channel.onmessage).toBeNull();
  });

  it('does not throw when a frame arrives before the viewer hooks onmessage', () => {
    // If the optional-chaining `?.` is dropped, this throws with
    // `channel.onmessage is not a function` because onmessage is null.
    const ws = new MockWebSocket('ws://test/sim');
    createNativeClient('ws://test/sim', () => ws);
    expect(() => ws.triggerMessage('{"type":"ready"}')).not.toThrow();
  });

  it('rebuffers postMessage after the WebSocket closes', () => {
    // After `onclose`, opened flips back to false — postMessage
    // queues again instead of synchronously calling `ws.send` (which
    // on a real closed socket would throw).
    const ws = new MockWebSocket('ws://test/sim');
    const channel = createNativeClient('ws://test/sim', () => ws);

    ws.triggerOpen();
    channel.postMessage({ type: 'step' });
    expect(ws.sent.length).toBe(1);

    // Simulate server-side close (not via terminate, which is
    // client-driven; this is the inbound onclose path).
    ws.onclose?.();

    // Next postMessage should queue, not send.
    channel.postMessage({ type: 'step' });
    expect(ws.sent.length).toBe(1);

    // Reopening flushes the queued frame.
    ws.triggerOpen();
    expect(ws.sent.length).toBe(2);
  });

  it('typing of IncomingFromServer covers the four expected variants', () => {
    // Smoke test: variants line up with the runtime decoder. The
    // assignments fail at compile time if `IncomingFromServer` drifts.
    const ready: IncomingFromServer = { type: 'ready' };
    const welcome: IncomingFromServer = { type: 'welcome', running: false };
    const snapshot: IncomingFromServer = {
      type: 'snapshot',
      tick: 0,
      cellCount: 0,
      totalEnergy: 0,
      msPerTick: 0,
      snap: new Uint32Array(0),
      stride: 6,
      bbox: new Int32Array(6),
    };
    const cellDetail: IncomingFromServer = {
      type: 'cellDetail',
      x: 0,
      y: 0,
      z: 0,
      tick: 0,
      data: new Uint32Array(0),
      prefix: 28,
    };
    expect([ready, welcome, snapshot, cellDetail].length).toBe(4);
  });
});

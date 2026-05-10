// Native dev backend WebSocket adapter.
//
// Provides a [`SimChannel`] interface that mimics the slice of the
// `Worker` API the viewer (`web/main.ts`) actually uses — `postMessage`
// + `onmessage` — but bridges to a remote `aenternis-server` over
// WebSocket. The viewer is unchanged: feature flag in `web/main.ts`
// picks `new Worker(...)` (WASM, default) or `createNativeClient(...)`
// (native dev), and the rest of the rendering pipeline is identical.
//
// ## Wire format mirror
//
// Mirrors `crates/aenternis-server/src/protocol.rs`:
// - **Outbound (client → server)**: every `MainToWorkerMsg` becomes a
//   single JSON text frame. Optional `program: Uint32Array` is
//   widened to a plain `number[]` first because `JSON.stringify` of a
//   typed array produces an indexed object, not an array.
// - **Inbound (server → client)**: text frames are JSON `{ type:
//   'ready' }` or `{ type: 'welcome', running: boolean }`. Binary
//   frames are tagged little-endian byte streams (tag 1 = snapshot,
//   tag 2 = cellDetail) — see `decodeBinaryFrame` for the layout.
//
// ## Open-state buffering
//
// `postMessage` invoked before the WS handshake completes queues the
// frame and flushes it on `onopen`. This matches the Worker contract,
// where `worker.postMessage` immediately after construction is fine
// because the Worker is already alive.
//
// Pure logic, no DOM, no network — the WebSocket constructor is
// injected so the unit tests can drive the channel with a mock socket.

import type {
  CellDetailMsg,
  MainToWorkerMsg,
  ReadyMsg,
  SnapshotMsg,
  WelcomeMsg,
  WorkerToMainMsg,
} from './protocol.ts';

/** Discriminated union the viewer reads off `onmessage.data`. */
export type IncomingFromServer = ReadyMsg | WelcomeMsg | SnapshotMsg | CellDetailMsg;

/** Subset of the browser `WebSocket` API the channel actually uses.
 *  Defined as an interface so tests can supply a mock implementation
 *  without pulling in `happy-dom`'s WebSocket. */
export interface WebSocketLike {
  send(data: string | ArrayBuffer | Blob | ArrayBufferView): void;
  close(): void;
  /** Native `WebSocket` reads `MessageEvent`; the only field we use
   *  is `.data`. Keep the contract that narrow. */
  onmessage: ((ev: { readonly data: string | ArrayBuffer }) => void) | null;
  onopen: (() => void) | null;
  onclose: (() => void) | null;
  onerror: ((err: unknown) => void) | null;
}

/** WebSocket constructor — injected so tests substitute a mock. */
export type WebSocketCtor = (url: string) => WebSocketLike;

/** Worker-like channel surface the viewer expects. Methods are kept
 *  to the strict subset `web/main.ts` uses today. */
export interface SimChannel {
  postMessage(msg: MainToWorkerMsg, transfer?: readonly Transferable[]): void;
  onmessage: ((ev: { readonly data: WorkerToMainMsg }) => void) | null;
  /** Close the underlying transport. Counterpart to `Worker.terminate()`. */
  terminate(): void;
}

/** Tag byte for snapshot binary frames (matches server `SNAPSHOT_TAG`). */
export const SNAPSHOT_TAG = 1;
/** Tag byte for cellDetail binary frames (matches server `CELL_DETAIL_TAG`). */
export const CELL_DETAIL_TAG = 2;

/** Construct a [`SimChannel`] talking to `aenternis-server` at `url`.
 *
 *  `wsCtor` is the WebSocket factory — pass `(u) => new WebSocket(u)`
 *  in production glue and a mock in tests. */
export function createNativeClient(url: string, wsCtor: WebSocketCtor): SimChannel {
  const ws = wsCtor(url);
  // Browser WebSocket delivers ArrayBuffer (not Blob) when we set
  // binaryType = 'arraybuffer'. Mocks already return ArrayBuffer.
  // Some `WebSocketLike` implementations may not expose binaryType,
  // so we set it best-effort via a property assertion.
  (ws as unknown as { binaryType?: string }).binaryType = 'arraybuffer';

  let opened = false;
  const queue: string[] = [];

  const channel: SimChannel = {
    onmessage: null,
    postMessage(msg) {
      const text = encodeOutgoing(msg);
      if (opened) ws.send(text);
      else queue.push(text);
    },
    terminate() {
      ws.close();
    },
  };

  ws.onopen = () => {
    opened = true;
    for (const text of queue) ws.send(text);
    queue.length = 0;
  };

  ws.onmessage = (ev) => {
    const decoded = decodeIncoming(ev.data);
    if (decoded === null) return;
    channel.onmessage?.({ data: decoded });
  };

  ws.onclose = () => {
    opened = false;
  };

  // `onerror` left as null — surfaces in onclose when WS terminates.
  // Production glue can wire a handler if needed; the channel stays
  // silent so unparseable frames don't spam the console.

  return channel;
}

/** Serialize a viewer-bound message to a JSON text frame. */
export function encodeOutgoing(msg: MainToWorkerMsg): string {
  if (msg.type === 'init' && msg.program !== undefined) {
    // `JSON.stringify` on a Uint32Array produces an indexed object,
    // not an array — convert to plain `number[]` first so the server
    // sees the canonical JSON shape (`[1,2,3]`).
    return JSON.stringify({ ...msg, program: Array.from(msg.program) });
  }
  return JSON.stringify(msg);
}

/** Decode a server-bound frame (JSON text or tagged binary) into the
 *  matching `WorkerToMainMsg` variant. Returns `null` on malformed
 *  input — the caller silently drops, matching the conservative
 *  Worker contract. */
export function decodeIncoming(data: string | ArrayBuffer): IncomingFromServer | null {
  if (typeof data === 'string') return decodeTextFrame(data);
  return decodeBinaryFrame(data);
}

function decodeTextFrame(text: string): ReadyMsg | WelcomeMsg | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const obj = parsed as { readonly type?: unknown; readonly running?: unknown };
  if (obj.type === 'ready') return { type: 'ready' };
  if (obj.type === 'welcome' && typeof obj.running === 'boolean') {
    return { type: 'welcome', running: obj.running };
  }
  return null;
}

/** Snapshot frame layout: see `crates/aenternis-server/src/protocol.rs`.
 *
 *  ```text
 *  tag       u8  @ 0
 *  tick      u32 @ 1
 *  cellCount u32 @ 5
 *  totalEng  u32 @ 9
 *  msPerTick f64 @ 13
 *  stride    u32 @ 21
 *  bbox      i32×6 @ 25..49
 *  snap      u32×(cellCount*stride) @ 49..
 *  ```
 *
 *  CellDetail frame layout:
 *
 *  ```text
 *  tag      u8  @ 0
 *  x,y,z    i32 @ 1, 5, 9
 *  tick     u32 @ 13
 *  prefix   u32 @ 17
 *  dataLen  u32 @ 21
 *  data     u32×dataLen @ 25..
 *  ```
 */
function decodeBinaryFrame(buf: ArrayBuffer): SnapshotMsg | CellDetailMsg | null {
  if (buf.byteLength < 1) return null;
  const view = new DataView(buf);
  const tag = view.getUint8(0);

  if (tag === SNAPSHOT_TAG) {
    if (buf.byteLength < SNAPSHOT_HEADER_LEN) return null;
    const tick = view.getUint32(1, true);
    const cellCount = view.getUint32(5, true);
    const totalEnergy = view.getUint32(9, true);
    const msPerTick = view.getFloat64(13, true);
    const stride = view.getUint32(21, true);
    const bbox = new Int32Array(buf.slice(25, 49));
    const expectedPayloadBytes = cellCount * stride * 4;
    if (buf.byteLength < SNAPSHOT_HEADER_LEN + expectedPayloadBytes) return null;
    const snap = new Uint32Array(
      buf.slice(SNAPSHOT_HEADER_LEN, SNAPSHOT_HEADER_LEN + expectedPayloadBytes),
    );
    return { type: 'snapshot', tick, cellCount, totalEnergy, msPerTick, snap, stride, bbox };
  }

  if (tag === CELL_DETAIL_TAG) {
    if (buf.byteLength < CELL_DETAIL_HEADER_LEN) return null;
    const x = view.getInt32(1, true);
    const y = view.getInt32(5, true);
    const z = view.getInt32(9, true);
    const tick = view.getUint32(13, true);
    const prefix = view.getUint32(17, true);
    const dataLen = view.getUint32(21, true);
    const expectedPayloadBytes = dataLen * 4;
    if (buf.byteLength < CELL_DETAIL_HEADER_LEN + expectedPayloadBytes) return null;
    const data = new Uint32Array(
      buf.slice(CELL_DETAIL_HEADER_LEN, CELL_DETAIL_HEADER_LEN + expectedPayloadBytes),
    );
    return { type: 'cellDetail', x, y, z, tick, data, prefix };
  }

  return null;
}

/** Snapshot header: tag(1) + tick(4) + cellCount(4) + totalEnergy(4)
 *  + msPerTick(8) + stride(4) + bbox(24) = 49 bytes. */
const SNAPSHOT_HEADER_LEN = 49;

/** CellDetail header: tag(1) + x(4) + y(4) + z(4) + tick(4) +
 *  prefix(4) + dataLen(4) = 25 bytes. */
const CELL_DETAIL_HEADER_LEN = 25;

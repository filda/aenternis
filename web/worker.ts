// Aenternis simulation worker — entrypoint.
//
// Owns the WASM `World` instance and runs the per-tick step loop in
// the background. Sends a snapshot back to the main thread after every
// tick via `postMessage`, transferring the underlying ArrayBuffer
// (zero-copy — the worker doesn't keep a handle to it after sending).
//
// All non-trivial logic lives in `src/worker-handler.ts` (factory) and
// `src/worker-state.ts` (pure reducer); the wire format is defined in
// `src/protocol.ts`. This file is the thin glue that wires real
// browser-side dependencies (WASM init, `setTimeout`, `performance.now`,
// the worker globals `self` / `postMessage`) into that handler.
//
// See `src/protocol.ts` for the message-protocol contract.

import init, { World } from '../crates/aenternis-wasm/pkg/aenternis_wasm.js';
import {
  isMainToWorkerMsg,
  type ReadyMsg,
  type WorkerToMainMsg,
} from '../src/protocol.ts';
import {
  createWorkerHandler,
  type WorldFactory,
  type WorldHandle,
} from '../src/worker-handler.ts';

await init();

// Worker globals: this file runs as a `DedicatedWorkerGlobalScope`.
// Cast `self` once so the rest of the file can address it without
// repeating the assertion.
const workerGlobal = self as unknown as DedicatedWorkerGlobalScope;

const ready: ReadyMsg = { type: 'ready' };
workerGlobal.postMessage(ready);

// `wasm-bindgen` generates `World` as a class with a `newWithProgram`
// static; structurally compatible with `WorldFactory`. Wrap it in a
// thin adapter so the cast is colocated with the wasm import.
const worldFactory: WorldFactory = {
  newWithProgram(seed, energy, program) {
    return World.newWithProgram(seed, energy, program) as unknown as WorldHandle;
  },
};

const handler = createWorkerHandler({
  worldFactory,
  postMessage(msg: WorkerToMainMsg, transfer?: readonly Transferable[]) {
    if (transfer && transfer.length > 0) {
      workerGlobal.postMessage(msg, transfer as Transferable[]);
    } else {
      workerGlobal.postMessage(msg);
    }
  },
  scheduleNext(cb) {
    setTimeout(cb, 0);
  },
  now() {
    return performance.now();
  },
});

workerGlobal.onmessage = (ev: MessageEvent<unknown>) => {
  if (isMainToWorkerMsg(ev.data)) {
    handler.handleMessage(ev.data);
  }
};

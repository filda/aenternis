// Aenternis — native dev backend glue.
//
// Wraps the pure logic in `src/native-client.ts` with the real
// browser `WebSocket`. `web/main.ts` picks this glue when the URL
// query / localStorage selects the native backend; otherwise it
// instantiates a plain Web Worker as before.
//
// Thin by design — the JSON encode/decode, queue-on-open, and
// frame parsing all live in the pure module so `npm run check`
// covers them under vitest + stryker. This file only provides the
// `WebSocket` constructor that `happy-dom` (test environment) does
// not.

import {
  createNativeClient as createPureClient,
  type SimChannel,
  type WebSocketLike,
} from '../src/native-client.ts';

/** Open a [`SimChannel`] against `aenternis-server` running at `url`.
 *  The returned object is shape-compatible with the slice of `Worker`
 *  the viewer actually uses (`postMessage`, `onmessage`, `terminate`),
 *  so `web/main.ts` treats the two backends identically.
 *
 *  The browser `WebSocket` is structurally close to `WebSocketLike`
 *  but its `MessageEvent` carries extra fields (`origin`, `ports`,
 *  …) that our narrowed contract doesn't list. The `unknown` cast
 *  acknowledges that — the pure client only ever reads `.data`. */
export function openNativeClient(url: string): SimChannel {
  return createPureClient(url, (u) => new WebSocket(u) as unknown as WebSocketLike);
}

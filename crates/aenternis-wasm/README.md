# aenternis-wasm

WebAssembly bindings for the Aenternis simulation core.

## Build

Once-per-machine prerequisites:

```sh
rustup target add wasm32-unknown-unknown
npm install -g wasm-pack       # or download a binary release if you prefer
```

Build the WASM bundle from the workspace root:

```sh
wasm-pack build crates/aenternis-wasm --target web
```

The bundle lands in `crates/aenternis-wasm/pkg/`:

- `aenternis_wasm_bg.wasm` — the compiled WebAssembly module
- `aenternis_wasm.js` — generated JS glue (ES module)
- `aenternis_wasm.d.ts` — TypeScript types

Import from the frontend:

```js
import init, { World } from "aenternis-wasm";

await init();                    // load and instantiate the .wasm
const w = new World(42, 100);    // seed = 42, big-bang energy = 100

w.step(0.15, 1);                 // one tick
console.log(w.total_energy());   // 100, conserved
w.free();                        // explicit handle drop
```

## Test

Host-target tests run via the standard workspace gate:

```sh
bash scripts/check.sh
```

These exercise the wrapper's Rust surface (types, conservation
invariants preserved across the boundary). They don't load the
`.wasm` itself — that needs `wasm-pack test --headless --chrome`,
which we'll wire into CI when phase 4 (frontend integration) lands.

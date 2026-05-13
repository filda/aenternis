import { defineConfig } from 'vite';

// Vite konfigurace pro Aenternis prototypy.
//
// - Projekt je multi-page: každý prototyp má vlastní index.html.
// - appType: 'mpa' vypne SPA fallback, takže Vite neservíruje root index.html
//   pro neznámé URL, ale 404 - chová se jako klasický statický server.
// - Dev server běží na portu 5173 (Vite default).
// - Web Workery (např. prototypes/08-viewer-3d/worker.js) jsou loadovány
//   přímo přes relativní path v `new Worker("worker.js")`. HTTP origin je
//   konzistentní (http://localhost:5173) takže žádné null-origin restrikce.
// - COOP/COEP hlavičky se nastavují server-side pro local dev tak, aby
//   `window.crossOriginIsolated === true` od první návštěvy. To aktivuje
//   `SharedArrayBuffer` pro `wasm-bindgen-rayon` thread pool ve threaded
//   WASM buildu (`scripts/build-wasm.sh`). Bez těchto hlaviček by se
//   musel spolehnout na `web/coi-serviceworker.js` shim, který funguje
//   ale zavádí 1s reload-flicker při první návštěvě.
// - COEP mode je `credentialless` (ne `require-corp`), aby cross-origin
//   resources bez CORP hlavičky (notably esm.sh CDN pro Three.js) stále
//   byly načitatelné. Matchuje default mode v `coi-serviceworker.js`.

const crossOriginIsolationHeaders = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'credentialless',
};

export default defineConfig({
  appType: 'mpa',
  server: {
    port: 5173,
    open: '/',
    headers: crossOriginIsolationHeaders,
  },
  preview: {
    headers: crossOriginIsolationHeaders,
  },
  // Žádný build/bundling - prototypy jsou raw HTML/JS/CSS.
  // Three.js se natáhne z CDN přímo v <script> tagech.
});

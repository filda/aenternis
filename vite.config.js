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

export default defineConfig({
  appType: 'mpa',
  server: {
    port: 5173,
    open: '/',
  },
  // Žádný build/bundling - prototypy jsou raw HTML/JS/CSS.
  // Three.js se natáhne z CDN přímo v <script> tagech.
});

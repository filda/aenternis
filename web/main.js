// Aenternis — WASM smoke test.
//
// Loads the Rust core compiled to WebAssembly, runs the simulation in
// a requestAnimationFrame loop, and renders cells as a 2D xy
// projection on a canvas. Z-axis is ignored at this stage — it'll
// become Three.js InstancedMesh in phase 4.

import init, { World } from "/crates/aenternis-wasm/pkg/aenternis_wasm.js";

await init();

// ----- Configuration state ----------------------------------------------------

const config = {
  seed: 42,
  energy: 500,
  coeff: 0.20,
  k: 1,
};

// ----- World construction -----------------------------------------------------

let world = new World(config.seed, config.energy);
let running = true;

function rebuild() {
  world.free();
  world = new World(config.seed, config.energy);
}

// ----- DOM ---------------------------------------------------------------------

const canvas = document.getElementById("canvas");
const ctx = canvas.getContext("2d", { alpha: false });

const dom = {
  tick: document.getElementById("tick"),
  cells: document.getElementById("cells"),
  energy: document.getElementById("energy"),
  fps: document.getElementById("fps"),
  pauseBtn: document.getElementById("pauseBtn"),
  resetBtn: document.getElementById("resetBtn"),
  seed: document.getElementById("seed"),
  energyIn: document.getElementById("energy_in"),
  coeff: document.getElementById("coeff"),
  coeffVal: document.getElementById("coeffVal"),
  k: document.getElementById("k"),
  kVal: document.getElementById("kVal"),
};

dom.pauseBtn.addEventListener("click", () => {
  running = !running;
  dom.pauseBtn.textContent = running ? "Pause" : "Resume";
});

dom.resetBtn.addEventListener("click", () => {
  config.seed = parseInt(dom.seed.value, 10) || 0;
  config.energy = parseInt(dom.energyIn.value, 10) || 0;
  rebuild();
});

dom.coeff.addEventListener("input", () => {
  config.coeff = parseFloat(dom.coeff.value);
  dom.coeffVal.textContent = config.coeff.toFixed(2);
});

dom.k.addEventListener("input", () => {
  config.k = parseInt(dom.k.value, 10) || 1;
  dom.kVal.textContent = config.k;
});

// ----- Canvas sizing ----------------------------------------------------------

function resizeCanvas() {
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = Math.floor(rect.width * dpr);
  canvas.height = Math.floor(rect.height * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}

window.addEventListener("resize", resizeCanvas);
resizeCanvas();

// ----- Rendering --------------------------------------------------------------
//
// Heat ramp: energy → color. Three reference stops:
//   0   → black (handled implicitly: empty cells aren't in the snapshot)
//   low → cool blue
//   mid → orange
//   high → white
//
// We sqrt-normalize against `maxEnergy` so a wide dynamic range still
// shows structure. `maxEnergy` is recomputed each frame from the
// snapshot — cheap, and keeps the color mapping responsive.

function heatColor(t) {
  // t in [0, 1]
  const stops = [
    [0.00, [0,   0,   0  ]],   // shouldn't appear (cells with E=0 aren't rendered)
    [0.10, [40,  60,  100]],   // dim blue
    [0.30, [80,  20,  60 ]],   // wine purple
    [0.55, [200, 70,  30 ]],   // orange
    [0.80, [240, 200, 80 ]],   // yellow
    [1.00, [255, 255, 220]],   // near-white
  ];
  let i = 0;
  while (i < stops.length - 1 && t > stops[i + 1][0]) i++;
  const a = stops[i];
  const b = stops[Math.min(i + 1, stops.length - 1)];
  const span = b[0] - a[0];
  const lerp = span > 0 ? (t - a[0]) / span : 0;
  return [
    Math.round(a[1][0] + (b[1][0] - a[1][0]) * lerp),
    Math.round(a[1][1] + (b[1][1] - a[1][1]) * lerp),
    Math.round(a[1][2] + (b[1][2] - a[1][2]) * lerp),
  ];
}

function render() {
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  const cx = w / 2;
  const cy = h / 2;

  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, w, h);

  const snap = world.cellsSnapshot();
  const stride = world.snapshotStride;
  if (snap.length === 0) return;

  // First pass: compute max energy for normalization.
  let maxE = 0;
  for (let i = 3; i < snap.length; i += stride) {
    if (snap[i] > maxE) maxE = snap[i];
  }
  if (maxE < 1) maxE = 1;

  // Pixels per cell. Auto-scale so the world always fits in view.
  // Compute world bounding box in xy.
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (let i = 0; i < snap.length; i += stride) {
    const x = snap[i] | 0;
    const y = snap[i + 1] | 0;
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  }
  const span = Math.max(maxX - minX, maxY - minY, 1);
  const padding = 40;
  const scale = Math.min((w - 2 * padding) / span, (h - 2 * padding) / span);
  const cellSize = Math.max(2, Math.floor(scale * 0.9));

  // Second pass: draw each cell.
  for (let i = 0; i < snap.length; i += stride) {
    const x = snap[i] | 0;
    const y = snap[i + 1] | 0;
    // z = snap[i+2] | 0;  // ignored in 2D
    const e = snap[i + 3];

    const t = Math.sqrt(e / maxE);
    const [r, g, b] = heatColor(t);
    ctx.fillStyle = `rgb(${r},${g},${b})`;

    const px = cx + (x - (minX + maxX) / 2) * scale;
    const py = cy + (y - (minY + maxY) / 2) * scale;
    ctx.fillRect(px - cellSize / 2, py - cellSize / 2, cellSize, cellSize);
  }
}

// ----- Animation loop ---------------------------------------------------------

let lastT = performance.now();
let fpsAvg = 0;

function frame(now) {
  const dt = now - lastT;
  lastT = now;
  if (dt > 0) fpsAvg = 0.9 * fpsAvg + 0.1 * (1000 / dt);

  if (running) {
    world.step(config.coeff, config.k);
  }

  render();

  // HUD update (cheap, every frame).
  dom.tick.textContent = world.tick();
  dom.cells.textContent = world.cellCount().toLocaleString();
  dom.energy.textContent = world.totalEnergy().toLocaleString();
  dom.fps.textContent = fpsAvg.toFixed(1);

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);

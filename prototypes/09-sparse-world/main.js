"use strict";

// Prototyp 9 — UI a renderování. Vlastní fyzika je v world.js.

// Presety stejné jako v prototypu 6 (pro snazší experimentaci).
const PRESETS = {
  counter: "loop:\n  inc 0x10\n  jmp loop",
  self_xp: "start:\n  setp xp, start\n  jmp start",
  self_omni: "start:\n  setp xp, start\n  setp xn, start\n  setp yp, start\n  setp yn, start\n  jmp start",
  beacon: "start:\n  inc 0x20\n  setp xp, start\n  jmp start",
  quine_core: "; quine — písmena DEADBEEF\nstart:\n  set 0x10, 0xDE\n  set 0x11, 0xAD\n  set 0x12, 0xBE\n  set 0x13, 0xEF\n  setp xp, start\n  jmp start",
  projectile: "start:\n  setp xp, start\n  port xp, 0x20\n  jmp start",
};

// ===== DOM =====

const dom = {
  canvas: document.getElementById("worldCanvas"),
  eTotal: document.getElementById("eTotalInput"),
  diffusion: document.getElementById("diffusionInput"),
  diffusionVal: document.getElementById("diffusionValue"),
  cpuK: document.getElementById("cpuKInput"),
  moveThreshold: document.getElementById("moveThresholdInput"),
  moveThresholdVal: document.getElementById("moveThresholdValue"),
  seed: document.getElementById("seedInput"),
  run: document.getElementById("runButton"),
  step: document.getElementById("stepButton"),
  reset: document.getElementById("resetButton"),
  spf: document.getElementById("stepsPerFrameInput"),
  zoom: document.getElementById("zoomInput"),
  zoomVal: document.getElementById("zoomValue"),
  viz: Array.from(document.querySelectorAll("input[name='viz']")),
  preset: document.getElementById("presetInput"),
  program: document.getElementById("programInput"),
  tickValue: document.getElementById("tickValue"),
  cellCount: document.getElementById("cellCountValue"),
  totalEnergy: document.getElementById("totalEnergyValue"),
  bbox: document.getElementById("bboxValue"),
  centroid: document.getElementById("centroidValue"),
  cap: document.getElementById("capValue"),
  conservationBtn: document.getElementById("conservationCheckButton"),
  conservationOut: document.getElementById("conservationOutput"),
};

const ctx = dom.canvas.getContext("2d");

// Naplň presety
for (const k of Object.keys(PRESETS)) {
  const opt = document.createElement("option");
  opt.value = k;
  opt.textContent = k;
  dom.preset.appendChild(opt);
}

// ===== Stav =====

const ui = {
  world: null,
  running: false,
  viz: "energy",
  zoom: 8,
  initialETotal: 0,
};

// ===== Reset =====

function reset() {
  ui.running = false;
  dom.run.textContent = "Spustit";

  const eTotal = clamp(parseInt(dom.eTotal.value, 10) || 1, 1, 65536);
  const seed = parseInt(dom.seed.value, 10) || 1;
  const diffusionCoeff = parseFloat(dom.diffusion.value);
  const cpuK = clamp(parseInt(dom.cpuK.value, 10) || 1, 1, 64);
  const moveThreshold = parseFloat(dom.moveThreshold.value);

  ui.world = new SparseWorld({ seed, diffusionCoeff, cpuK, moveThreshold });
  ui.initialETotal = eTotal;

  let programSlots = [];
  const text = dom.program.value.trim();
  if (text.length > 0) {
    const parsed = parseProgram(text);
    if (parsed.errors.length > 0) {
      console.warn("Asembler chyby:", parsed.errors);
    }
    programSlots = parsed.slots;
  }
  ui.world.bigBang(eTotal, programSlots);

  render();
  updateHud();
}

// ===== Render =====

function resizeCanvas() {
  // Backing store musí odpovídat zobrazené velikosti, jinak se obraz roztáhne
  // (a image-rendering: pixelated to dělá blocky). Volat před každým renderem.
  const cw = Math.floor(dom.canvas.clientWidth);
  const ch = Math.floor(dom.canvas.clientHeight);
  if (cw > 0 && ch > 0 && (dom.canvas.width !== cw || dom.canvas.height !== ch)) {
    dom.canvas.width = cw;
    dom.canvas.height = ch;
  }
}

function render() {
  resizeCanvas();
  const w = dom.canvas.width;
  const h = dom.canvas.height;
  ctx.fillStyle = "#050507";
  ctx.fillRect(0, 0, w, h);

  if (!ui.world || ui.world.cells.size === 0) return;

  const zoom = ui.zoom;
  const cx = w / 2;
  const cy = h / 2;

  // Defaultní kamera míří na energetický centroid (viz `prototype-09-plan.md`).
  const c = ui.world.centroid();
  const camX = c ? c.x : 0;
  const camY = c ? c.y : 0;

  // Vizualizace: spočítáme rozsah hodnot pro auto-scale
  let maxValue = 1;
  if (ui.viz === "energy") {
    for (const cell of ui.world.cells.values()) {
      if (cell.energy > maxValue) maxValue = cell.energy;
    }
  } else {
    for (const cell of ui.world.cells.values()) {
      const v = cellRawValue(cell);
      if (v > maxValue) maxValue = v;
    }
  }

  // Render každé buňky jako voxel
  ctx.imageSmoothingEnabled = false;
  for (const cell of ui.world.cells.values()) {
    const px = Math.floor(cx + (cell.x - camX) * zoom);
    const py = Math.floor(cy + (cell.y - camY) * zoom);
    if (px + zoom < 0 || py + zoom < 0 || px >= w || py >= h) continue;
    const v = cellRawValue(cell) / maxValue;
    const color = colorForValue(v);
    ctx.fillStyle = `rgb(${color[0]},${color[1]},${color[2]})`;
    ctx.fillRect(px, py, zoom, zoom);
  }

  // Bounding box rámeček
  const bb = ui.world.boundingBox();
  if (bb) {
    const x0 = Math.floor(cx + (bb.xMin - camX) * zoom);
    const y0 = Math.floor(cy + (bb.yMin - camY) * zoom);
    const x1 = Math.floor(cx + (bb.xMax + 1 - camX) * zoom);
    const y1 = Math.floor(cy + (bb.yMax + 1 - camY) * zoom);
    ctx.strokeStyle = "#3a8c4f";
    ctx.lineWidth = 1;
    ctx.strokeRect(x0 + 0.5, y0 + 0.5, x1 - x0 - 1, y1 - y0 - 1);
  }

  // Centroid jako bílá tečka uprostřed
  if (c) {
    ctx.fillStyle = "rgba(255,255,255,0.85)";
    ctx.beginPath();
    ctx.arc(cx, cy, 3, 0, Math.PI * 2);
    ctx.fill();
  }

  // Origin (0,0) jako červené plus, aby bylo vidět drift centroidu
  const ox = Math.floor(cx + (0 - camX) * zoom + zoom / 2);
  const oy = Math.floor(cy + (0 - camY) * zoom + zoom / 2);
  ctx.strokeStyle = "rgba(220,80,80,0.7)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(ox - 4, oy);
  ctx.lineTo(ox + 4, oy);
  ctx.moveTo(ox, oy - 4);
  ctx.lineTo(ox, oy + 4);
  ctx.stroke();
}

function cellRawValue(cell) {
  if (ui.viz === "energy") return cell.energy;
  if (ui.viz === "memory_top") {
    const m = cell.memory;
    return m.length > 0 ? (m[m.length - 1] & 0xFF) : 0;
  }
  if (ui.viz === "memory_bottom") {
    const m = cell.memory;
    return m.length > 0 ? (m[0] & 0xFF) : 0;
  }
  return 0;
}

// Inferno-like paleta z prototypu 6
function colorForValue(v) {
  v = Math.max(0, Math.min(1, v));
  const t = Math.sqrt(v);
  const stops = [
    [0.00,  10,   5,  20],
    [0.25,  90,  20,  90],
    [0.50, 220,  60,  60],
    [0.75, 250, 160,  60],
    [1.00, 255, 240, 200],
  ];
  let i = 0;
  while (i < stops.length - 1 && t > stops[i + 1][0]) i++;
  const a = stops[i];
  const b = stops[Math.min(i + 1, stops.length - 1)];
  const span = b[0] - a[0];
  const lerp = span > 0 ? (t - a[0]) / span : 0;
  return [
    Math.round(a[1] + (b[1] - a[1]) * lerp),
    Math.round(a[2] + (b[2] - a[2]) * lerp),
    Math.round(a[3] + (b[3] - a[3]) * lerp),
  ];
}

// ===== HUD =====

function updateHud() {
  if (!ui.world) return;
  dom.tickValue.textContent = ui.world.tick;
  const size = ui.world.size();
  dom.cellCount.textContent = size;
  const total = ui.world.totalEnergy();
  dom.totalEnergy.textContent = total;
  const bb = ui.world.boundingBox();
  dom.bbox.textContent = bb
    ? `(${bb.xMin}..${bb.xMax}, ${bb.yMin}..${bb.yMax}) = ${bb.xMax - bb.xMin + 1}×${bb.yMax - bb.yMin + 1}`
    : "—";
  const c = ui.world.centroid();
  dom.centroid.textContent = c ? `(${c.x.toFixed(2)}, ${c.y.toFixed(2)})` : "—";
  // Cap = world.size / E_total. Klíčový invariant — pokud kdy překročí 100%, je to bug.
  const ratio = ui.initialETotal > 0 ? (size / ui.initialETotal * 100) : 0;
  dom.cap.textContent = `${size} / ${ui.initialETotal} (${ratio.toFixed(1)}%)`;
}

// ===== Loop =====

let animationFrame = null;
function tick() {
  if (!ui.running) return;
  const spf = clamp(parseInt(dom.spf.value, 10) || 1, 1, 50);
  for (let s = 0; s < spf; s++) ui.world.step();
  render();
  updateHud();
  animationFrame = requestAnimationFrame(tick);
}

// ===== Listenery =====

dom.run.addEventListener("click", () => {
  ui.running = !ui.running;
  dom.run.textContent = ui.running ? "Pauza" : "Spustit";
  if (ui.running) tick();
});

dom.step.addEventListener("click", () => {
  ui.world.step();
  render();
  updateHud();
});

dom.reset.addEventListener("click", reset);

dom.diffusion.addEventListener("input", () => {
  const v = parseFloat(dom.diffusion.value);
  dom.diffusionVal.textContent = v.toFixed(2);
  if (ui.world) ui.world.diffusionCoeff = v;
});

dom.moveThreshold.addEventListener("input", () => {
  const v = parseFloat(dom.moveThreshold.value);
  dom.moveThresholdVal.textContent = v.toFixed(1);
  if (ui.world) ui.world.moveThreshold = v;
});

dom.cpuK.addEventListener("change", () => {
  const v = clamp(parseInt(dom.cpuK.value, 10) || 1, 1, 64);
  dom.cpuK.value = v;
  if (ui.world) ui.world.cpuK = v;
});

dom.zoom.addEventListener("input", () => {
  ui.zoom = clamp(parseInt(dom.zoom.value, 10) || 8, 2, 32);
  dom.zoomVal.textContent = ui.zoom;
  render();
});

dom.viz.forEach(r => r.addEventListener("change", () => {
  if (r.checked) {
    ui.viz = r.value;
    render();
  }
}));

dom.preset.addEventListener("change", () => {
  const k = dom.preset.value;
  if (k && PRESETS[k]) {
    dom.program.value = PRESETS[k];
  }
});

// Konzervační batch test pro definici hotového prototypu.
dom.conservationBtn.addEventListener("click", () => {
  if (ui.running) {
    ui.running = false;
    dom.run.textContent = "Spustit";
  }
  const N_TICKS = 10000;
  const e = clamp(parseInt(dom.eTotal.value, 10) || 1, 1, 65536);
  const seed = parseInt(dom.seed.value, 10) || 1;
  const diffusionCoeff = parseFloat(dom.diffusion.value);
  const cpuK = clamp(parseInt(dom.cpuK.value, 10) || 1, 1, 64);
  const moveThreshold = parseFloat(dom.moveThreshold.value);

  const programs = ["pure_noise", "counter", "self_xp_replicator"];
  const results = [];
  const t0 = performance.now();

  for (const name of programs) {
    let progSlots = [];
    if (name === "counter") progSlots = parseProgram(PRESETS.counter).slots;
    else if (name === "self_xp_replicator") progSlots = parseProgram(PRESETS.self_xp).slots;
    const w = new SparseWorld({ seed, diffusionCoeff, cpuK, moveThreshold });
    w.bigBang(e, progSlots);
    let conservationFails = 0;
    let capFails = 0;
    let maxSize = 1;
    for (let t = 1; t <= N_TICKS; t++) {
      w.step();
      const sum = w.totalEnergy();
      const sz = w.size();
      if (sum !== e) conservationFails++;
      if (sz > e) capFails++;
      if (sz > maxSize) maxSize = sz;
    }
    const bb = w.boundingBox();
    const c = w.centroid();
    results.push({
      name,
      conservationFails,
      capFails,
      maxSize,
      finalSize: w.size(),
      finalE: w.totalEnergy(),
      bbox: bb ? `(${bb.xMin}..${bb.xMax}, ${bb.yMin}..${bb.yMax})` : "(empty)",
      centroid: c ? `(${c.x.toFixed(2)}, ${c.y.toFixed(2)})` : "(empty)",
    });
  }
  const t1 = performance.now();

  let out = `Batch test: 3 scénáře × ${N_TICKS} ticek (${((t1 - t0) / 1000).toFixed(1)} s)\n`;
  out += `E_total=${e}, seed=${seed}, coeff=${diffusionCoeff}, K=${cpuK}\n\n`;
  for (const r of results) {
    out += `[${r.name}]\n`;
    out += `  konzervace porušena: ${r.conservationFails} ticek\n`;
    out += `  cap porušen: ${r.capFails} ticek\n`;
    out += `  max world.size: ${r.maxSize} (cap = ${e})\n`;
    out += `  final E=${r.finalE}, size=${r.finalSize}, bbox=${r.bbox}, centroid=${r.centroid}\n`;
  }
  const totalFails = results.reduce((a, r) => a + r.conservationFails + r.capFails, 0);
  out += `\n${totalFails === 0 ? "VŠECHNY SCÉNÁŘE PROŠLY" : `${totalFails} TICKŮ S BUGEM`}\n`;
  dom.conservationOut.textContent = out;
});

function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

window.addEventListener("resize", () => {
  if (ui.world) render();
});

// ===== Init =====

dom.diffusionVal.textContent = parseFloat(dom.diffusion.value).toFixed(2);
dom.moveThresholdVal.textContent = parseFloat(dom.moveThreshold.value).toFixed(1);
dom.zoomVal.textContent = dom.zoom.value;
ui.zoom = parseInt(dom.zoom.value, 10);

reset();

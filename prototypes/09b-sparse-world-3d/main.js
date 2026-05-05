"use strict";

// Prototyp 9-B (3D) — UI a renderování přes three.js. Vlastní fyzika je v world.js.
//
// Renderer: WebGL (three.js r146 UMD), InstancedMesh boxů s kapacitou rovnou
// aktuálnímu E_total. Per-frame se nastavuje `count = world.cells.size` a pro
// každou živou buňku setMatrixAt + setColorAt. Bbox je drátový kvádr (LineSegments
// z EdgesGeometry), origin červený křížek, centroid bílá koule.
//
// THREE a THREE.OrbitControls jsou globály z UMD scriptů v index.html.

// Presety stejné jako v 9 + self_zp/self_omni (6 směrů).
const PRESETS = {
  counter: "loop:\n  inc 0x10\n  jmp loop",
  self_xp: "start:\n  setp xp, start\n  jmp start",
  self_zp: "start:\n  setp zp, start\n  jmp start",
  self_omni: "start:\n  setp xp, start\n  setp xn, start\n  setp yp, start\n  setp yn, start\n  setp zp, start\n  setp zn, start\n  jmp start",
  beacon: "start:\n  inc 0x20\n  setp xp, start\n  jmp start",
  quine_core: "; quine — písmena DEADBEEF\nstart:\n  set 0x10, 0xDE\n  set 0x11, 0xAD\n  set 0x12, 0xBE\n  set 0x13, 0xEF\n  setp xp, start\n  jmp start",
  projectile: "start:\n  setp xp, start\n  port xp, 0x20\n  jmp start",
};

// ===== DOM =====

const dom = {
  container: document.getElementById("canvasContainer"),
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
  voxelSize: document.getElementById("voxelSizeInput"),
  voxelSizeVal: document.getElementById("voxelSizeValue"),
  followCentroid: document.getElementById("followCentroidInput"),
  showBbox: document.getElementById("showBboxInput"),
  viz: Array.from(document.querySelectorAll("input[name='viz']")),
  preset: document.getElementById("presetInput"),
  program: document.getElementById("programInput"),
  tickValue: document.getElementById("tickValue"),
  cellCount: document.getElementById("cellCountValue"),
  totalEnergy: document.getElementById("totalEnergyValue"),
  bbox: document.getElementById("bboxValue"),
  centroid: document.getElementById("centroidValue"),
  cap: document.getElementById("capValue"),
  fps: document.getElementById("fpsValue"),
  conservationBtn: document.getElementById("conservationCheckButton"),
  conservationOut: document.getElementById("conservationOutput"),
};

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
  voxelSize: 0.85,
  followCentroid: true,
  showBbox: true,
  initialETotal: 0,
};

// ===== Three.js scéna =====

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x050507);

const camera = new THREE.PerspectiveCamera(
  60,
  Math.max(1, dom.container.clientWidth) / Math.max(1, dom.container.clientHeight),
  0.1,
  20000,
);
camera.position.set(40, 30, 40);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(dom.container.clientWidth, dom.container.clientHeight);
dom.container.appendChild(renderer.domElement);

const controls = new THREE.OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.07;
controls.target.set(0, 0, 0);

// Světla
scene.add(new THREE.AmbientLight(0xffffff, 0.55));
const dirLight = new THREE.DirectionalLight(0xffffff, 0.7);
dirLight.position.set(80, 120, 60);
scene.add(dirLight);
const fillLight = new THREE.DirectionalLight(0x88aaff, 0.25);
fillLight.position.set(-60, -40, -80);
scene.add(fillLight);

// Origin marker (3 osové úsečky, červené)
const ORIGIN_LEN = 1.5;
{
  const geo = new THREE.BufferGeometry();
  const verts = new Float32Array([
    -ORIGIN_LEN, 0, 0,  ORIGIN_LEN, 0, 0,
    0, -ORIGIN_LEN, 0,  0, ORIGIN_LEN, 0,
    0, 0, -ORIGIN_LEN,  0, 0, ORIGIN_LEN,
  ]);
  geo.setAttribute("position", new THREE.BufferAttribute(verts, 3));
  const mat = new THREE.LineBasicMaterial({ color: 0xdc5050, transparent: true, opacity: 0.85 });
  const cross = new THREE.LineSegments(geo, mat);
  scene.add(cross);
}

// Centroid marker (bílá koule)
const centroidMesh = (() => {
  const geo = new THREE.SphereGeometry(0.4, 12, 8);
  const mat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.9 });
  const m = new THREE.Mesh(geo, mat);
  scene.add(m);
  return m;
})();

// Bbox wireframe — přebudovává se per frame z aktuálního bbox.
let bboxMesh = null;
function updateBboxMesh(bb) {
  if (bboxMesh) {
    scene.remove(bboxMesh);
    bboxMesh.geometry.dispose();
    bboxMesh.material.dispose();
    bboxMesh = null;
  }
  if (!bb || !ui.showBbox) return;
  const w = bb.xMax - bb.xMin + 1;
  const h = bb.yMax - bb.yMin + 1;
  const d = bb.zMax - bb.zMin + 1;
  const cx = (bb.xMin + bb.xMax + 1) / 2 - 0.5;
  const cy = (bb.yMin + bb.yMax + 1) / 2 - 0.5;
  const cz = (bb.zMin + bb.zMax + 1) / 2 - 0.5;
  const geo = new THREE.EdgesGeometry(new THREE.BoxGeometry(w, h, d));
  const mat = new THREE.LineBasicMaterial({ color: 0x3a8c4f, transparent: true, opacity: 0.6 });
  const line = new THREE.LineSegments(geo, mat);
  line.position.set(cx, cy, cz);
  scene.add(line);
  bboxMesh = line;
}

// Voxel InstancedMesh — kapacita = aktuální E_total. Každá živá buňka má
// jeden instance slot. Při change E_total se realokuje (createVoxelMesh).
let voxelMesh = null;
let voxelCapacity = 0;
const _tmpMatrix = new THREE.Matrix4();
const _tmpQuat = new THREE.Quaternion();
const _tmpPos = new THREE.Vector3();
const _tmpScale = new THREE.Vector3();
const _tmpColor = new THREE.Color();

function createVoxelMesh(capacity) {
  if (voxelMesh) {
    scene.remove(voxelMesh);
    voxelMesh.geometry.dispose();
    voxelMesh.material.dispose();
  }
  voxelCapacity = Math.max(1, capacity);
  // Krychlové voxely 1×1×1 — sparse svět má diskrétní celočíselné souřadnice.
  const geo = new THREE.BoxGeometry(1, 1, 1);
  const mat = new THREE.MeshLambertMaterial({ vertexColors: false });
  voxelMesh = new THREE.InstancedMesh(geo, mat, voxelCapacity);
  voxelMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  // Inicializace: všechny instance scale=0 (skryté), barva černá.
  _tmpScale.set(0, 0, 0);
  _tmpPos.set(0, 0, 0);
  const black = new THREE.Color(0, 0, 0);
  for (let i = 0; i < voxelCapacity; i++) {
    _tmpMatrix.compose(_tmpPos, _tmpQuat, _tmpScale);
    voxelMesh.setMatrixAt(i, _tmpMatrix);
    voxelMesh.setColorAt(i, black);
  }
  voxelMesh.count = 0;
  scene.add(voxelMesh);
}

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

  // Realokace voxel meshe na novou kapacitu (E_total = horní mez počtu buněk).
  createVoxelMesh(eTotal);

  // Naprahnu kameru tak, aby viděla počáteční bbox z odstupu.
  recenterCamera(true);

  render();
  updateHud();
}

function recenterCamera(initial) {
  const c = ui.world ? ui.world.centroid() : null;
  if (!c) return;
  if (initial) {
    // Při resetu posunu kameru i target. Při běhu (followCentroid) jen target.
    const dist = 30;
    controls.target.set(c.x, c.y, c.z);
    camera.position.set(c.x + dist, c.y + dist * 0.7, c.z + dist);
  } else if (ui.followCentroid) {
    controls.target.set(c.x, c.y, c.z);
  }
}

// ===== Render =====

let visibleCount = 0;

function render() {
  if (!voxelMesh || !ui.world) return;

  // Pokud E_total v UI změnil a překračuje kapacitu, realokuj. Není ale
  // ideální to dělat za běhu — primární cesta je přes Reset. Tady jen safety.
  if (ui.world.size() > voxelCapacity) {
    createVoxelMesh(Math.max(ui.world.size(), ui.initialETotal));
  }

  // Předpočet maxValue pro paletu (relativní škála).
  let maxValue = 1;
  if (ui.viz === "energy") {
    for (const cell of ui.world.cells.values()) {
      if (cell.energy > maxValue) maxValue = cell.energy;
    }
  } else if (ui.viz === "memory_top" || ui.viz === "memory_bottom") {
    maxValue = 255; // byte rozsah
  } // origin_tag nepotřebuje scale

  let i = 0;
  const voxelScale = ui.voxelSize;
  for (const cell of ui.world.cells.values()) {
    const v = cellRawValue(cell);
    const t = ui.viz === "energy"
      ? Math.sqrt(Math.max(0, v) / maxValue)
      : (ui.viz === "origin_tag" ? 0 : Math.max(0, v) / maxValue);

    let r, g, b;
    if (ui.viz === "origin_tag") {
      const tag = (cell.originTag >>> 0);
      const hue = (tag % 360) / 360;
      const rgb = hsvToRgb(hue, 0.7, 0.9);
      r = rgb[0]; g = rgb[1]; b = rgb[2];
    } else {
      const rgb = heatColor(t);
      r = rgb[0]; g = rgb[1]; b = rgb[2];
    }

    _tmpPos.set(cell.x, cell.y, cell.z);
    _tmpScale.set(voxelScale, voxelScale, voxelScale);
    _tmpMatrix.compose(_tmpPos, _tmpQuat, _tmpScale);
    voxelMesh.setMatrixAt(i, _tmpMatrix);
    _tmpColor.setRGB(r, g, b);
    voxelMesh.setColorAt(i, _tmpColor);
    i++;
    if (i >= voxelCapacity) break; // safety
  }
  visibleCount = i;
  voxelMesh.count = i;
  voxelMesh.instanceMatrix.needsUpdate = true;
  if (voxelMesh.instanceColor) voxelMesh.instanceColor.needsUpdate = true;

  // Centroid + bbox
  const c = ui.world.centroid();
  if (c) {
    centroidMesh.position.set(c.x, c.y, c.z);
    centroidMesh.visible = true;
    if (ui.followCentroid) controls.target.set(c.x, c.y, c.z);
  } else {
    centroidMesh.visible = false;
  }

  updateBboxMesh(ui.world.boundingBox());

  renderer.render(scene, camera);
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
  if (ui.viz === "origin_tag") return 0;
  return 0;
}

// Heat paleta — adaptace inferno z prototypu 9 (RGB v 0..1).
function heatColor(v) {
  v = Math.max(0, Math.min(1, v));
  const stops = [
    [0.00, 0.04, 0.02, 0.08],
    [0.25, 0.35, 0.08, 0.35],
    [0.50, 0.86, 0.24, 0.24],
    [0.75, 0.98, 0.63, 0.24],
    [1.00, 1.00, 0.94, 0.78],
  ];
  let i = 0;
  while (i < stops.length - 1 && v > stops[i + 1][0]) i++;
  const a = stops[i];
  const b = stops[Math.min(i + 1, stops.length - 1)];
  const span = b[0] - a[0];
  const lerp = span > 0 ? (v - a[0]) / span : 0;
  return [
    a[1] + (b[1] - a[1]) * lerp,
    a[2] + (b[2] - a[2]) * lerp,
    a[3] + (b[3] - a[3]) * lerp,
  ];
}

function hsvToRgb(h, s, v) {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  switch (i % 6) {
    case 0: return [v, t, p];
    case 1: return [q, v, p];
    case 2: return [p, v, t];
    case 3: return [p, q, v];
    case 4: return [t, p, v];
    default: return [v, p, q];
  }
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
    ? `(${bb.xMin}..${bb.xMax}, ${bb.yMin}..${bb.yMax}, ${bb.zMin}..${bb.zMax}) = ${bb.xMax - bb.xMin + 1}×${bb.yMax - bb.yMin + 1}×${bb.zMax - bb.zMin + 1}`
    : "—";
  const c = ui.world.centroid();
  dom.centroid.textContent = c
    ? `(${c.x.toFixed(2)}, ${c.y.toFixed(2)}, ${c.z.toFixed(2)})`
    : "—";
  const ratio = ui.initialETotal > 0 ? (size / ui.initialETotal * 100) : 0;
  dom.cap.textContent = `${size} / ${ui.initialETotal} (${ratio.toFixed(1)}%)`;
}

// ===== Loop (animation frame) =====

const perf = { lastTs: 0, samples: [], lastHudUpdate: 0 };
function avgFps() {
  if (perf.samples.length === 0) return 0;
  let s = 0;
  for (const v of perf.samples) s += v;
  return s / perf.samples.length;
}

function frame(ts) {
  const dt = perf.lastTs > 0 ? (ts - perf.lastTs) : 16;
  perf.lastTs = ts;
  if (dt > 0 && dt < 1000) {
    perf.samples.push(1000 / dt);
    if (perf.samples.length > 30) perf.samples.shift();
  }

  applyWsad(dt);
  controls.update();

  if (ui.running && ui.world) {
    const spf = clamp(parseInt(dom.spf.value, 10) || 1, 1, 50);
    for (let s = 0; s < spf; s++) ui.world.step();
  }

  render();

  // HUD update jen ~10× za sekundu, ať nezahlcuje DOM
  if (ts - perf.lastHudUpdate > 100) {
    updateHud();
    dom.fps.textContent = avgFps().toFixed(1);
    perf.lastHudUpdate = ts;
  }

  requestAnimationFrame(frame);
}

// ===== WSAD pohyb kamery (přejaté z prototypu 8, lehce zjednodušené) =====
//
// W/S = dopředu/dozadu po směru pohledu
// A/D = vlevo/vpravo (perpendikulárně)
// Q/E = dolů/nahoru (world Y)
// Shift = sprint (3×)

const keyState = { w: false, a: false, s: false, d: false, q: false, e: false, shift: false };

function isInputFocused(target) {
  const tag = (target && target.tagName) || "";
  return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
}

window.addEventListener("keydown", (ev) => {
  if (isInputFocused(ev.target)) return;
  const k = ev.key.toLowerCase();
  if (k in keyState) keyState[k] = true;
  if (k === "shift") keyState.shift = true;
});
window.addEventListener("keyup", (ev) => {
  const k = ev.key.toLowerCase();
  if (k in keyState) keyState[k] = false;
  if (k === "shift") keyState.shift = false;
});
window.addEventListener("blur", () => {
  for (const k of Object.keys(keyState)) keyState[k] = false;
});

const _camForward = new THREE.Vector3();
const _camRight = new THREE.Vector3();
const _worldUp = new THREE.Vector3(0, 1, 0);
const _moveDelta = new THREE.Vector3();

function applyWsad(dtMs) {
  if (!keyState.w && !keyState.a && !keyState.s && !keyState.d && !keyState.q && !keyState.e) return;
  // Rychlost úměrná průměru bbox, aby pohyb škáloval s velikostí světa.
  const bb = ui.world ? ui.world.boundingBox() : null;
  const span = bb
    ? Math.max(bb.xMax - bb.xMin, bb.yMax - bb.yMin, bb.zMax - bb.zMin) + 4
    : 16;
  const baseSpeed = span * 0.025 * (dtMs / 16);
  const speed = baseSpeed * (keyState.shift ? 3 : 1);

  camera.getWorldDirection(_camForward);
  _camRight.crossVectors(_camForward, _worldUp).normalize();
  _moveDelta.set(0, 0, 0);
  if (keyState.w) _moveDelta.addScaledVector(_camForward, speed);
  if (keyState.s) _moveDelta.addScaledVector(_camForward, -speed);
  if (keyState.d) _moveDelta.addScaledVector(_camRight, speed);
  if (keyState.a) _moveDelta.addScaledVector(_camRight, -speed);
  if (keyState.e) _moveDelta.y += speed;
  if (keyState.q) _moveDelta.y -= speed;
  camera.position.add(_moveDelta);
  controls.target.add(_moveDelta);
}

// ===== Listenery =====

dom.run.addEventListener("click", () => {
  ui.running = !ui.running;
  dom.run.textContent = ui.running ? "Pauza" : "Spustit";
});

dom.step.addEventListener("click", () => {
  if (!ui.world) return;
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

dom.voxelSize.addEventListener("input", () => {
  ui.voxelSize = parseFloat(dom.voxelSize.value);
  dom.voxelSizeVal.textContent = ui.voxelSize.toFixed(2);
});

dom.followCentroid.addEventListener("change", () => {
  ui.followCentroid = dom.followCentroid.checked;
});

dom.showBbox.addEventListener("change", () => {
  ui.showBbox = dom.showBbox.checked;
  if (!ui.showBbox) updateBboxMesh(null);
});

dom.viz.forEach(r => r.addEventListener("change", () => {
  if (r.checked) ui.viz = r.value;
}));

dom.preset.addEventListener("change", () => {
  const k = dom.preset.value;
  if (k && PRESETS[k]) dom.program.value = PRESETS[k];
});

// Konzervační batch test (čistě výpočetní, bez rendering kroku).
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
      name, conservationFails, capFails, maxSize,
      finalSize: w.size(),
      finalE: w.totalEnergy(),
      bbox: bb ? `(${bb.xMin}..${bb.xMax}, ${bb.yMin}..${bb.yMax}, ${bb.zMin}..${bb.zMax})` : "(empty)",
      centroid: c ? `(${c.x.toFixed(2)}, ${c.y.toFixed(2)}, ${c.z.toFixed(2)})` : "(empty)",
    });
  }
  const t1 = performance.now();

  let out = `Batch test (3D): 3 scénáře × ${N_TICKS} ticek (${((t1 - t0) / 1000).toFixed(1)} s)\n`;
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
  const w = dom.container.clientWidth;
  const h = dom.container.clientHeight;
  if (w > 0 && h > 0) {
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h);
  }
});

// ===== Init =====

dom.diffusionVal.textContent = parseFloat(dom.diffusion.value).toFixed(2);
dom.moveThresholdVal.textContent = parseFloat(dom.moveThreshold.value).toFixed(1);
dom.voxelSizeVal.textContent = parseFloat(dom.voxelSize.value).toFixed(2);
ui.voxelSize = parseFloat(dom.voxelSize.value);
ui.followCentroid = dom.followCentroid.checked;
ui.showBbox = dom.showBbox.checked;

reset();
requestAnimationFrame(frame);

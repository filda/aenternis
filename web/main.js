// Aenternis — WASM 3D viewer (phase 4c).
//
// Render thread only. The WASM World instance and the per-tick step
// loop live in a dedicated Web Worker (`./worker.js`); the main thread
// receives `{ snap, stride, tick, ... }` snapshots over postMessage
// (Uint32Array transferred zero-copy) and renders the latest one each
// animation frame.
//
// This decoupling keeps the render thread at 60 FPS even when the
// simulation is heavy. Render and sim are independent: if the worker
// is slow, the renderer reuses the last snapshot; if the renderer is
// slow, intermediate snapshots are simply overwritten before they're
// drawn.

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { assemble } from "../src/asm.js";

// ----- Configuration ---------------------------------------------------------

const config = {
  seed: 1234,
  energy: 10_000_000,
  coeff: 0.15,
  k: 1,
  moveThreshold: 1.0,
  // "pcg" (Aenternis default) or "xorshift32" (matches JS prototype 9-B
  // bit-for-bit). Changing this only takes effect on Reset, since RNG
  // backend determines the big bang's initial state.
  rngKind: "pcg",
};

// ----- Worker setup ----------------------------------------------------------

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });

let latestSnapshot = null; // { snap, stride, tick, cellCount, totalEnergy }
let workerReady = false;

worker.onmessage = (ev) => {
  const msg = ev.data;
  if (msg.type === "ready") {
    workerReady = true;
    sendInit();
  } else if (msg.type === "snapshot") {
    latestSnapshot = msg;
  } else if (msg.type === "cellDetail") {
    renderInspector(msg);
  }
};

function sendInit() {
  let program = new Uint32Array(0);
  if (dom.programText) {
    const result = assemble(dom.programText.value);
    program = result.slots;
    if (dom.programStatus) {
      dom.programStatus.textContent = result.errors.length > 0
        ? `${result.errors.length} parse error(s): ${result.errors.join("; ")}`
        : `${program.length} slot(s) assembled`;
    }
  }
  worker.postMessage({
    type: "init",
    seed: config.seed,
    energy: config.energy,
    coeff: config.coeff,
    k: config.k,
    moveThreshold: config.moveThreshold,
    rngKind: config.rngKind,
    program,
  });
  cameraFitDirty = true;
  tracker.trail = [];
  tracker.current = null;
}

function sendConfig() {
  if (!workerReady) return;
  worker.postMessage({
    type: "config",
    coeff: config.coeff,
    k: config.k,
    moveThreshold: config.moveThreshold,
  });
}

function sendRunning(running) {
  if (!workerReady) return;
  worker.postMessage({ type: "running", running });
}

// ----- Three.js setup --------------------------------------------------------

const container = document.getElementById("canvasContainer");

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x05050a);

const camera = new THREE.PerspectiveCamera(60, window.innerWidth / window.innerHeight, 0.1, 5000);
camera.position.set(20, 20, 30);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(window.innerWidth, window.innerHeight);
container.appendChild(renderer.domElement);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.05;

window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

// Lights
scene.add(new THREE.AmbientLight(0xffffff, 0.4));
const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
dirLight.position.set(50, 80, 50);
scene.add(dirLight);

// ----- WSAD camera (FPS-style movement on top of OrbitControls) --------------

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
  if (!keyState.w && !keyState.a && !keyState.s && !keyState.d
      && !keyState.q && !keyState.e) return;

  const dist = camera.position.distanceTo(controls.target);
  const baseSpeed = Math.max(dist * 0.01, 0.1) * (dtMs / 16);
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

// ----- Voxel mesh (instanced, dynamic capacity) ------------------------------

const voxelGeometry = new THREE.SphereGeometry(0.45, 8, 6);
const voxelMaterial = new THREE.MeshLambertMaterial();

let voxelMesh = null;
let voxelCapacity = 0;
let lastUsedCount = 0;

function ensureCapacity(n) {
  if (voxelCapacity >= n) return;
  let cap = voxelCapacity > 0 ? voxelCapacity : 256;
  while (cap < n) cap *= 2;
  if (voxelMesh) {
    scene.remove(voxelMesh);
    voxelMesh.dispose();
  }
  voxelMesh = new THREE.InstancedMesh(voxelGeometry, voxelMaterial, cap);
  voxelMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  const initColor = new THREE.Color(0, 0, 0);
  for (let i = 0; i < cap; i++) {
    voxelMesh.setColorAt(i, initColor);
  }
  scene.add(voxelMesh);
  voxelCapacity = cap;
  lastUsedCount = 0;
}

ensureCapacity(1024);

// ----- Heat ramp -------------------------------------------------------------

const HEAT_STOPS = [
  [0.00, [0.00, 0.00, 0.00]],
  [0.10, [0.16, 0.24, 0.40]],
  [0.30, [0.32, 0.08, 0.24]],
  [0.55, [0.78, 0.28, 0.12]],
  [0.80, [0.94, 0.78, 0.32]],
  [1.00, [1.00, 1.00, 0.86]],
];

function heatColor(t) {
  t = Math.max(0, Math.min(1, t));
  let i = 0;
  while (i < HEAT_STOPS.length - 1 && t > HEAT_STOPS[i + 1][0]) i++;
  const a = HEAT_STOPS[i];
  const b = HEAT_STOPS[Math.min(i + 1, HEAT_STOPS.length - 1)];
  const span = b[0] - a[0];
  const lerp = span > 0 ? (t - a[0]) / span : 0;
  return [
    a[1][0] + (b[1][0] - a[1][0]) * lerp,
    a[1][1] + (b[1][1] - a[1][1]) * lerp,
    a[1][2] + (b[1][2] - a[1][2]) * lerp,
  ];
}

// ----- Tracker (highlight max-energy cell + trail) ---------------------------

const tracker = {
  enabled: false,
  trailLen: 60,
  trail: [],
  current: null,
  highlightMesh: null,
  trailLine: null,
};

function createHighlightMesh() {
  if (tracker.highlightMesh) {
    scene.remove(tracker.highlightMesh);
    tracker.highlightMesh.geometry.dispose();
    tracker.highlightMesh.material.dispose();
  }
  const geo = new THREE.BoxGeometry(1.4, 1.4, 1.4);
  const mat = new THREE.LineBasicMaterial({ color: 0xfff0c0, transparent: true, opacity: 0.95 });
  const wire = new THREE.LineSegments(new THREE.EdgesGeometry(geo), mat);
  wire.visible = false;
  scene.add(wire);
  tracker.highlightMesh = wire;
}

function createTrailLine() {
  if (tracker.trailLine) {
    scene.remove(tracker.trailLine);
    tracker.trailLine.geometry.dispose();
    tracker.trailLine.material.dispose();
  }
  const cap = Math.max(2, tracker.trailLen + 1);
  const geo = new THREE.BufferGeometry();
  const positions = new Float32Array(cap * 3);
  const colors = new Float32Array(cap * 3);
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  geo.setDrawRange(0, 0);
  const mat = new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.85 });
  const line = new THREE.Line(geo, mat);
  line.frustumCulled = false;
  scene.add(line);
  tracker.trailLine = line;
}

createHighlightMesh();
createTrailLine();

function updateTracker(maxCellIdx, snap, stride) {
  if (!tracker.enabled || maxCellIdx < 0) {
    if (tracker.highlightMesh) tracker.highlightMesh.visible = false;
    if (tracker.trailLine) tracker.trailLine.visible = false;
    if (dom.trackerPos) dom.trackerPos.textContent = "-";
    return;
  }

  const off = maxCellIdx * stride;
  const x = snap[off] | 0;
  const y = snap[off + 1] | 0;
  const z = snap[off + 2] | 0;
  const e = snap[off + 3];
  tracker.current = { x, y, z, energy: e };

  const last = tracker.trail.length > 0 ? tracker.trail[tracker.trail.length - 1] : null;
  if (!last || last.x !== x || last.y !== y || last.z !== z) {
    tracker.trail.push({ x, y, z, energy: e });
    while (tracker.trail.length > tracker.trailLen + 1) tracker.trail.shift();
  } else {
    last.energy = e;
  }

  if (tracker.highlightMesh) {
    tracker.highlightMesh.visible = true;
    tracker.highlightMesh.position.set(x, y, z);
    const pulse = 1.0 + 0.08 * Math.sin(performance.now() / 250);
    tracker.highlightMesh.scale.setScalar(pulse);
  }

  if (tracker.trailLine) {
    const len = tracker.trail.length;
    tracker.trailLine.visible = (tracker.trailLen > 0) && (len > 1);
    if (tracker.trailLine.visible) {
      const positions = tracker.trailLine.geometry.attributes.position.array;
      const colors = tracker.trailLine.geometry.attributes.color.array;
      const cap = positions.length / 3;
      const drawLen = Math.min(len, cap);
      for (let i = 0; i < drawLen; i++) {
        const p = tracker.trail[len - drawLen + i];
        positions[i * 3]     = p.x;
        positions[i * 3 + 1] = p.y;
        positions[i * 3 + 2] = p.z;
        const f = (i + 1) / drawLen;
        colors[i * 3]     = 1.00 * f;
        colors[i * 3 + 1] = 0.80 * f;
        colors[i * 3 + 2] = 0.20 * f;
      }
      tracker.trailLine.geometry.setDrawRange(0, drawLen);
      tracker.trailLine.geometry.attributes.position.needsUpdate = true;
      tracker.trailLine.geometry.attributes.color.needsUpdate = true;
    }
  }

  if (dom.trackerPos) {
    dom.trackerPos.textContent = `(${x},${y},${z}) E=${e.toLocaleString()}`;
  }
}

// ----- HUD --------------------------------------------------------------------

const dom = {
  tick: document.getElementById("tick"),
  cells: document.getElementById("cells"),
  energy: document.getElementById("energy"),
  fps: document.getElementById("fps"),
  msPerTick: document.getElementById("msPerTick"),
  ticksPerSec: document.getElementById("ticksPerSec"),
  pauseBtn: document.getElementById("pauseBtn"),
  resetBtn: document.getElementById("resetBtn"),
  seed: document.getElementById("seed"),
  energyIn: document.getElementById("energy_in"),
  coeff: document.getElementById("coeff"),
  coeffVal: document.getElementById("coeffVal"),
  k: document.getElementById("k"),
  kVal: document.getElementById("kVal"),
  moveThreshold: document.getElementById("moveThreshold"),
  moveThresholdVal: document.getElementById("moveThresholdVal"),
  trackerEnabled: document.getElementById("trackerEnabled"),
  trailLen: document.getElementById("trailLen"),
  trailLenVal: document.getElementById("trailLenVal"),
  trackerPos: document.getElementById("trackerPos"),
  programText: document.getElementById("programText"),
  programStatus: document.getElementById("programStatus"),
  sliceEnabled: document.getElementById("sliceEnabled"),
  rngXs32: document.getElementById("rngXs32"),
};

// ----- Slice (z = 0 only) — proto-9-style 2D view ----------------------------

const slice = { enabled: false };

dom.sliceEnabled.addEventListener("change", () => {
  slice.enabled = dom.sliceEnabled.checked;
  // Force a re-render even if no new snapshot has arrived.
  lastRenderedTick = -1;
});

let running = true;

dom.pauseBtn.addEventListener("click", () => {
  running = !running;
  dom.pauseBtn.textContent = running ? "Pause" : "Resume";
  sendRunning(running);
});
dom.resetBtn.addEventListener("click", () => {
  config.seed = parseInt(dom.seed.value, 10) || 0;
  config.energy = parseInt(dom.energyIn.value, 10) || 0;
  config.rngKind = dom.rngXs32.checked ? "xorshift32" : "pcg";
  running = true;
  dom.pauseBtn.textContent = "Pause";
  sendInit();
});
// RNG checkbox change is captured into config.rngKind only on Reset —
// switching backends mid-run would leave existing cells inconsistent
// (origin tags came from one hash, new cells would use the other).
dom.rngXs32.addEventListener("change", () => {
  // No live update; the user has to press Reset for the change to take
  // effect. Visual hint comes from the help text under the checkbox.
});
dom.coeff.addEventListener("input", () => {
  config.coeff = parseFloat(dom.coeff.value);
  dom.coeffVal.textContent = config.coeff.toFixed(2);
  sendConfig();
});
dom.k.addEventListener("input", () => {
  config.k = parseInt(dom.k.value, 10) || 1;
  dom.kVal.textContent = config.k;
  sendConfig();
});
dom.moveThreshold.addEventListener("input", () => {
  config.moveThreshold = parseFloat(dom.moveThreshold.value) || 2.0;
  dom.moveThresholdVal.textContent = config.moveThreshold.toFixed(1);
  sendConfig();
});
dom.trackerEnabled.addEventListener("change", () => {
  tracker.enabled = dom.trackerEnabled.checked;
});
dom.trailLen.addEventListener("input", () => {
  tracker.trailLen = parseInt(dom.trailLen.value, 10);
  dom.trailLenVal.textContent = tracker.trailLen;
  createTrailLine();
});

// ----- Camera initial fit ----------------------------------------------------

let cameraFitDirty = true;
function fitCameraToWorld(minX, maxX, minY, maxY, minZ, maxZ) {
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const cz = (minZ + maxZ) / 2;
  const span = Math.max(maxX - minX, maxY - minY, maxZ - minZ, 4);
  const dist = span * 2.5;
  controls.target.set(cx, cy, cz);
  camera.position.set(cx + dist, cy + dist * 0.6, cz + dist);
  camera.updateProjectionMatrix();
}

// ----- Frame loop (render only — sim runs in worker) -------------------------

const tempMatrix = new THREE.Matrix4();
const tempColor = new THREE.Color();
const tempPos = new THREE.Vector3();
const tempQuat = new THREE.Quaternion();
const tempScale = new THREE.Vector3(1, 1, 1);

let lastT = performance.now();
let fpsAvg = 0;
let msPerTickAvg = 0;
let ticksPerSecAvg = 0;
let lastRenderedTick = -1;
let lastTickStampT = 0;

function frame(now) {
  const dt = now - lastT;
  lastT = now;
  if (dt > 0) fpsAvg = 0.9 * fpsAvg + 0.1 * (1000 / dt);

  applyWsad(dt);

  // Render only when we've received at least one snapshot, and only
  // re-build instance data when the snapshot has actually advanced.
  if (latestSnapshot && latestSnapshot.tick !== lastRenderedTick) {
    renderSnapshot(latestSnapshot);

    // Smooth tick metrics from this newly-arrived snapshot.
    msPerTickAvg = 0.85 * msPerTickAvg + 0.15 * latestSnapshot.msPerTick;
    if (lastTickStampT > 0) {
      const ticksDelta = latestSnapshot.tick - lastRenderedTick;
      const wallDelta = now - lastTickStampT;
      if (ticksDelta > 0 && wallDelta > 0) {
        const observed = (ticksDelta * 1000) / wallDelta;
        ticksPerSecAvg = 0.85 * ticksPerSecAvg + 0.15 * observed;
      }
    }
    lastTickStampT = now;
    lastRenderedTick = latestSnapshot.tick;
  }

  controls.update();
  renderer.render(scene, camera);

  if (latestSnapshot) {
    dom.tick.textContent = latestSnapshot.tick;
    dom.cells.textContent = latestSnapshot.cellCount.toLocaleString();
    dom.energy.textContent = latestSnapshot.totalEnergy.toLocaleString();
  }
  dom.fps.textContent = fpsAvg.toFixed(1);
  dom.msPerTick.textContent = msPerTickAvg.toFixed(1);
  dom.ticksPerSec.textContent = ticksPerSecAvg.toFixed(1);

  maybeRefreshInspector();

  requestAnimationFrame(frame);
}

function renderSnapshot(state) {
  const { snap, stride, cellCount } = state;

  ensureCapacity(Math.max(cellCount, lastUsedCount));

  let minX = Infinity, maxX = -Infinity;
  let minY = Infinity, maxY = -Infinity;
  let minZ = Infinity, maxZ = -Infinity;
  let maxE = 0;
  let maxCellIdx = -1;
  for (let i = 0; i < cellCount; i++) {
    const off = i * stride;
    const z = snap[off + 2] | 0;
    if (slice.enabled && z !== 0) continue;
    const x = snap[off] | 0;
    const y = snap[off + 1] | 0;
    if (x < minX) minX = x; if (x > maxX) maxX = x;
    if (y < minY) minY = y; if (y > maxY) maxY = y;
    if (z < minZ) minZ = z; if (z > maxZ) maxZ = z;
    const e = snap[off + 3];
    if (e > maxE) { maxE = e; maxCellIdx = i; }
  }
  if (maxE < 1) maxE = 1;
  // Avoid bogus camera fit when slice mode hides everything.
  if (minX === Infinity) {
    minX = -1; maxX = 1; minY = -1; maxY = 1; minZ = -1; maxZ = 1;
  }

  if (cameraFitDirty && cellCount > 0) {
    fitCameraToWorld(minX, maxX, minY, maxY, minZ, maxZ);
    cameraFitDirty = false;
  }

  const zeroScale = new THREE.Vector3(0, 0, 0);
  for (let i = 0; i < cellCount; i++) {
    const off = i * stride;
    const x = snap[off] | 0;
    const y = snap[off + 1] | 0;
    const z = snap[off + 2] | 0;
    const e = snap[off + 3];

    if (slice.enabled && z !== 0) {
      // Hide instances outside the z=0 plane in slice mode.
      tempMatrix.compose(tempPos.set(0, 0, 0), tempQuat, zeroScale);
      voxelMesh.setMatrixAt(i, tempMatrix);
      continue;
    }

    tempPos.set(x, y, z);
    tempMatrix.compose(tempPos, tempQuat, tempScale);
    voxelMesh.setMatrixAt(i, tempMatrix);

    const t = Math.sqrt(e / maxE);
    const [r, g, b] = heatColor(t);
    tempColor.setRGB(r, g, b);
    voxelMesh.setColorAt(i, tempColor);
  }

  if (lastUsedCount > cellCount) {
    for (let i = cellCount; i < lastUsedCount; i++) {
      tempMatrix.compose(tempPos.set(0, 0, 0), tempQuat, zeroScale);
      voxelMesh.setMatrixAt(i, tempMatrix);
    }
  }
  lastUsedCount = cellCount;

  voxelMesh.instanceMatrix.needsUpdate = true;
  if (voxelMesh.instanceColor) voxelMesh.instanceColor.needsUpdate = true;

  // Three.js raycasts InstancedMesh by first testing the global
  // bounding sphere; without an explicit update it stays around the
  // single zero-instance default and the cursor never finds anything.
  // Update it from the world's bbox so clicks can hit live cells.
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const cz = (minZ + maxZ) / 2;
  const halfSpan = Math.max(maxX - minX, maxY - minY, maxZ - minZ, 2) / 2 + 1;
  if (!voxelMesh.boundingSphere) {
    voxelMesh.boundingSphere = new THREE.Sphere();
  }
  voxelMesh.boundingSphere.center.set(cx, cy, cz);
  voxelMesh.boundingSphere.radius = halfSpan;

  updateTracker(maxCellIdx, snap, stride);
}

// ----- Inspector (cell click → state dump panel) -----------------------------

const inspector = {
  panel: document.getElementById("inspector"),
  coord: null, // { x, y, z } or null when closed
  dom: {
    coord: document.getElementById("iCoord"),
    tick: document.getElementById("iTick"),
    pc: document.getElementById("iPc"),
    energy: document.getElementById("iEnergy"),
    originTag: document.getElementById("iOriginTag"),
    appearance: document.getElementById("iAppearance"),
    pointers: document.getElementById("iPointers"),
    rates: document.getElementById("iRates"),
    activeOutflow: document.getElementById("iActiveOutflow"),
    inflow: document.getElementById("iInflow"),
    memory: document.getElementById("iMemory"),
    close: document.getElementById("iClose"),
  },
};

inspector.dom.close.addEventListener("click", () => {
  inspector.coord = null;
  inspector.panel.classList.remove("visible");
});

const raycaster = new THREE.Raycaster();
const mouseNdc = new THREE.Vector2();

renderer.domElement.addEventListener("click", (ev) => {
  const rect = renderer.domElement.getBoundingClientRect();
  mouseNdc.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
  mouseNdc.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(mouseNdc, camera);

  if (!voxelMesh || !latestSnapshot) return;
  const hits = raycaster.intersectObject(voxelMesh, false);
  if (hits.length === 0) return;

  // Find the closest hit whose instanceId is within the live cell range
  // (instances beyond `cellCount` are scaled to zero but raycaster sees them).
  for (const hit of hits) {
    const idx = hit.instanceId;
    if (idx === undefined) continue;
    if (idx >= latestSnapshot.cellCount) continue;
    const off = idx * latestSnapshot.stride;
    const x = latestSnapshot.snap[off] | 0;
    const y = latestSnapshot.snap[off + 1] | 0;
    const z = latestSnapshot.snap[off + 2] | 0;
    inspector.coord = { x, y, z };
    inspector.panel.classList.add("visible");
    requestInspect(x, y, z);
    return;
  }
});

function requestInspect(x, y, z) {
  if (!workerReady) return;
  worker.postMessage({ type: "inspect", x, y, z });
}

const DIR_LABELS = ["xp", "xn", "yp", "yn", "zp", "zn"];

function fmtArr(arr) {
  return DIR_LABELS.map((d, i) => `${d}=${arr[i]}`).join("  ");
}

function fmtMemory(slots) {
  // Hex dump: 8 slots per row, each as 8-char hex.
  const lines = [];
  for (let i = 0; i < slots.length; i += 8) {
    const addr = i.toString(16).padStart(4, "0");
    const row = [];
    for (let j = 0; j < 8 && i + j < slots.length; j++) {
      row.push(slots[i + j].toString(16).padStart(8, "0"));
    }
    lines.push(`${addr}: ${row.join(" ")}`);
  }
  return lines.join("\n");
}

function renderInspector(msg) {
  if (!inspector.coord) return;
  const { x, y, z } = inspector.coord;
  if (msg.x !== x || msg.y !== y || msg.z !== z) return; // stale
  const data = msg.data;
  const prefix = msg.prefix;
  const dom = inspector.dom;

  if (data.length === 0) {
    dom.coord.textContent = `(${x}, ${y}, ${z}) — no cell`;
    dom.tick.textContent = msg.tick;
    dom.pc.textContent = "-";
    dom.energy.textContent = "-";
    dom.originTag.textContent = "-";
    dom.appearance.textContent = "-";
    dom.pointers.textContent = "";
    dom.rates.textContent = "";
    dom.activeOutflow.textContent = "";
    dom.inflow.textContent = "";
    dom.memory.textContent = "";
    return;
  }

  dom.coord.textContent = `(${x}, ${y}, ${z})`;
  dom.tick.textContent = msg.tick;
  dom.pc.textContent = data[0];
  dom.energy.textContent = data[1].toLocaleString();
  dom.originTag.textContent = `0x${data[2].toString(16).padStart(8, "0")}`;
  dom.appearance.textContent = `0x${data[3].toString(16).padStart(8, "0")}`;
  dom.pointers.textContent = fmtArr(data.slice(4, 10));
  dom.rates.textContent = fmtArr(data.slice(10, 16));
  dom.activeOutflow.textContent = fmtArr(data.slice(16, 22));
  dom.inflow.textContent = fmtArr(data.slice(22, 28));
  dom.memory.textContent = fmtMemory(data.slice(prefix));
}

// Auto-refresh inspector every ~5 frames while it's open and the world
// is running, so the panel reflects live state without flooding the
// worker with messages.
let inspectorRefreshCounter = 0;
function maybeRefreshInspector() {
  if (!inspector.coord || !running) return;
  inspectorRefreshCounter += 1;
  if (inspectorRefreshCounter >= 5) {
    inspectorRefreshCounter = 0;
    requestInspect(inspector.coord.x, inspector.coord.y, inspector.coord.z);
  }
}

requestAnimationFrame(frame);

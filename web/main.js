// Aenternis — WASM 3D viewer (phase 4a).
//
// Rust core compiled to WASM, rendered as Three.js InstancedMesh of
// spheres. One instance per cell; positions and colors are updated
// every frame from `world.cellsSnapshot()`. Capacity grows on demand
// (powers of two) so we don't pre-allocate megabytes for tiny worlds.

import init, { World } from "/crates/aenternis-wasm/pkg/aenternis_wasm.js";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

await init();

// ----- Configuration ---------------------------------------------------------

const config = {
  seed: 42,
  energy: 500,
  coeff: 0.20,
  k: 1,
};

let world = new World(config.seed, config.energy);
let running = true;

function rebuild() {
  world.free();
  world = new World(config.seed, config.energy);
  cameraFitDirty = true;
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
  // Pre-fill colors so .setColorAt is valid for any index.
  const initColor = new THREE.Color(0, 0, 0);
  for (let i = 0; i < cap; i++) {
    voxelMesh.setColorAt(i, initColor);
  }
  scene.add(voxelMesh);
  voxelCapacity = cap;
  lastUsedCount = 0;
}

ensureCapacity(1024);

// ----- Heat ramp (energy → color) --------------------------------------------

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

// ----- HUD --------------------------------------------------------------------

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

// ----- Frame loop ------------------------------------------------------------

const tempMatrix = new THREE.Matrix4();
const tempColor = new THREE.Color();
const tempPos = new THREE.Vector3();
const tempQuat = new THREE.Quaternion();
const tempScale = new THREE.Vector3(1, 1, 1);

let lastT = performance.now();
let fpsAvg = 0;

function frame(now) {
  const dt = now - lastT;
  lastT = now;
  if (dt > 0) fpsAvg = 0.9 * fpsAvg + 0.1 * (1000 / dt);

  if (running) {
    world.step(config.coeff, config.k);
  }

  const snap = world.cellsSnapshot();
  const stride = world.snapshotStride;
  const cellCount = (snap.length / stride) | 0;

  ensureCapacity(Math.max(cellCount, lastUsedCount));

  // First pass: bbox + max energy.
  let minX = Infinity, maxX = -Infinity;
  let minY = Infinity, maxY = -Infinity;
  let minZ = Infinity, maxZ = -Infinity;
  let maxE = 0;
  for (let i = 0; i < snap.length; i += stride) {
    const x = snap[i] | 0;
    const y = snap[i + 1] | 0;
    const z = snap[i + 2] | 0;
    if (x < minX) minX = x; if (x > maxX) maxX = x;
    if (y < minY) minY = y; if (y > maxY) maxY = y;
    if (z < minZ) minZ = z; if (z > maxZ) maxZ = z;
    if (snap[i + 3] > maxE) maxE = snap[i + 3];
  }
  if (maxE < 1) maxE = 1;

  // Camera initial fit (only on first frame after rebuild).
  if (cameraFitDirty && cellCount > 0) {
    fitCameraToWorld(minX, maxX, minY, maxY, minZ, maxZ);
    cameraFitDirty = false;
  }

  // Second pass: write instance matrix + color for each cell.
  for (let i = 0; i < cellCount; i++) {
    const off = i * stride;
    const x = snap[off] | 0;
    const y = snap[off + 1] | 0;
    const z = snap[off + 2] | 0;
    const e = snap[off + 3];

    tempPos.set(x, y, z);
    tempMatrix.compose(tempPos, tempQuat, tempScale);
    voxelMesh.setMatrixAt(i, tempMatrix);

    const t = Math.sqrt(e / maxE);
    const [r, g, b] = heatColor(t);
    tempColor.setRGB(r, g, b);
    voxelMesh.setColorAt(i, tempColor);
  }

  // Hide previously-used instances that are now beyond the live count.
  if (lastUsedCount > cellCount) {
    const zeroScale = new THREE.Vector3(0, 0, 0);
    for (let i = cellCount; i < lastUsedCount; i++) {
      tempMatrix.compose(tempPos.set(0, 0, 0), tempQuat, zeroScale);
      voxelMesh.setMatrixAt(i, tempMatrix);
    }
  }
  lastUsedCount = cellCount;

  voxelMesh.instanceMatrix.needsUpdate = true;
  if (voxelMesh.instanceColor) voxelMesh.instanceColor.needsUpdate = true;

  controls.update();
  renderer.render(scene, camera);

  // HUD.
  dom.tick.textContent = world.tick();
  dom.cells.textContent = cellCount.toLocaleString();
  dom.energy.textContent = world.totalEnergy().toLocaleString();
  dom.fps.textContent = fpsAvg.toFixed(1);

  requestAnimationFrame(frame);
}

requestAnimationFrame(frame);

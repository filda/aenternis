"use strict";
// THREE a THREE.OrbitControls dostupné jako globály z UMD scripts v index.html

// ===== Konstanty Aenternis =====

const DIRS = 6;
const OPPOSITE = [1, 0, 3, 2, 5, 4];
const DIR_OFFSET = [
  [+1, 0, 0], [-1, 0, 0],
  [0, +1, 0], [0, -1, 0],
  [0, 0, +1], [0, 0, -1],
];
const LAYOUT_ORDER_FROM_END = [5, 4, 3, 2, 1, 0];
const MAX_MEMORY = 16777216;

// ===== Aenternis simulation (zjednodušená z prototypu 7) =====

const sim = {
  N: 32,
  step: 0,
  cells: [],
  coeff: 0.15,
};

// Alternativní pohled na stav, když simulace běží ve workeru.
// Worker neposílá plné cells (memory atd.), jen energy a origin tagy.
const workerView = {
  energies: new Uint32Array(0),
  originTags: new Uint32Array(0),
};

function idx(x, y, z) { return x + y * sim.N + z * sim.N * sim.N; }
function neighborIdx(x, y, z, d) {
  const N = sim.N;
  const [dx, dy, dz] = DIR_OFFSET[d];
  return idx((x+dx+N)%N, (y+dy+N)%N, (z+dz+N)%N);
}

function stochasticFloor(v) {
  if (v <= 0) return 0;
  const w = Math.floor(v);
  return w + (Math.random() < (v - w) ? 1 : 0);
}

function makeCell() {
  return {
    energy: 0,
    memory: new Uint32Array(0),
    pointers: [0,0,0,0,0,0],
    rates: [0,0,0,0,0,0],
    originTag: Math.floor(Math.random() * 0x100000000) >>> 0,
  };
}

function randomSlots(n) {
  const a = new Uint32Array(n);
  for (let i = 0; i < n; i++) a[i] = Math.floor(Math.random() * 0x100000000) >>> 0;
  return a;
}

function resetWorld() {
  const N = sim.N;
  sim.cells = new Array(N*N*N);
  for (let i = 0; i < sim.cells.length; i++) sim.cells[i] = makeCell();
  sim.step = 0;
}

function initScenario(scenario) {
  const N = sim.N;
  const c = Math.floor(N / 2);
  if (scenario === "point") {
    const totalE = N * N * N;
    sim.cells[idx(c, c, c)].energy = totalE;
    sim.cells[idx(c, c, c)].memory = randomSlots(totalE);
  } else if (scenario === "ball") {
    const r = Math.max(2, Math.floor(N / 6));
    const maxE = 64;
    for (let z = 0; z < N; z++)
      for (let y = 0; y < N; y++)
        for (let x = 0; x < N; x++) {
          const dx = x-c, dy = y-c, dz = z-c;
          const dist = Math.sqrt(dx*dx + dy*dy + dz*dz);
          if (dist <= r) {
            const e = Math.floor(maxE * (1 - dist/(r+1)));
            if (e > 0) {
              sim.cells[idx(x,y,z)].energy = e;
              sim.cells[idx(x,y,z)].memory = randomSlots(e);
            }
          }
        }
  } else if (scenario === "random") {
    for (let i = 0; i < sim.cells.length; i++) {
      const e = Math.floor(Math.random() * 3);
      if (e > 0) {
        sim.cells[i].energy = e;
        sim.cells[i].memory = randomSlots(e);
      }
    }
  }
  recomputeLayouts();
}

function recomputeLayouts() {
  const N = sim.N;
  const cells = sim.cells;
  for (let z = 0; z < N; z++)
    for (let y = 0; y < N; y++)
      for (let x = 0; x < N; x++) {
        const i = idx(x, y, z);
        const cell = cells[i];
        const myE = cell.energy;
        let totalRate = 0;
        for (let d = 0; d < DIRS; d++) {
          const nE = cells[neighborIdx(x,y,z,d)].energy;
          cell.rates[d] = (myE - nE > 0) ? stochasticFloor((myE - nE) * sim.coeff) : 0;
          totalRate += cell.rates[d];
        }
        const memSize = cell.memory.length;
        if (totalRate > memSize && totalRate > 0) {
          const scale = memSize / totalRate;
          let newTotal = 0;
          for (let d = 0; d < DIRS; d++) {
            cell.rates[d] = Math.floor(cell.rates[d] * scale);
            newTotal += cell.rates[d];
          }
          let leftover = memSize - newTotal;
          while (leftover > 0) {
            let added = false;
            for (let d = 0; d < DIRS; d++) {
              if (cell.rates[d] > 0 && leftover > 0) {
                cell.rates[d] += 1;
                leftover -= 1;
                added = true;
                break;
              }
            }
            if (!added) break;
          }
        }
        let cursor = cell.memory.length;
        for (const d of LAYOUT_ORDER_FROM_END) {
          cursor -= cell.rates[d];
          cell.pointers[d] = Math.max(0, cursor);
        }
      }
}

// Yield helper - umožní browseru render mezi chunky
const yield0 = () => new Promise(r => setTimeout(r, 0));
const CHUNK_SIZE = 4000;  // počet cel zpracovaných před yield

// Flat neighbor index (i = flat index, vrací sousedův flat index)
function flatNeighborIdx(i, d) {
  const N = sim.N;
  const N2 = N * N;
  const x = i % N;
  const y = Math.floor(i / N) % N;
  const z = Math.floor(i / N2);
  const [dx, dy, dz] = DIR_OFFSET[d];
  return ((x + dx + N) % N) + ((y + dy + N) % N) * N + ((z + dz + N) % N) * N2;
}

async function step() {
  const N = sim.N;
  const total = N*N*N;
  const cells = sim.cells;

  // Phase 1: Outflow snapshot (chunked)
  const outflows = new Array(total);
  for (let chunkStart = 0; chunkStart < total; chunkStart += CHUNK_SIZE) {
    const chunkEnd = Math.min(chunkStart + CHUNK_SIZE, total);
    for (let i = chunkStart; i < chunkEnd; i++) {
      const cell = cells[i];
      const out = new Array(DIRS);
      const memSize = cell.memory.length;
      for (let d = 0; d < DIRS; d++) {
        const rate = cell.rates[d];
        const ptr = cell.pointers[d];
        const slots = new Uint32Array(rate);
        if (memSize > 0) {
          for (let k = 0; k < rate; k++) slots[k] = cell.memory[(ptr+k) % memSize];
        }
        out[d] = slots;
      }
      outflows[i] = out;
    }
    await yield0();
  }

  // Phase 2: Apply (chunked)
  for (let chunkStart = 0; chunkStart < total; chunkStart += CHUNK_SIZE) {
    const chunkEnd = Math.min(chunkStart + CHUNK_SIZE, total);
    for (let i = chunkStart; i < chunkEnd; i++) {
      const cell = cells[i];
      let totalOut = 0;
      for (let d = 0; d < DIRS; d++) totalOut += cell.rates[d];

      let totalIn = 0;
      const inflows = new Array(DIRS);
      for (let d = 0; d < DIRS; d++) {
        const inSlots = outflows[flatNeighborIdx(i, d)][OPPOSITE[d]];
        inflows[d] = inSlots;
        totalIn += inSlots.length;
      }

      const oldMem = cell.memory;
      const sizeAfterOut = Math.max(0, oldMem.length - totalOut);
      let newSize = sizeAfterOut + totalIn;
      if (newSize > MAX_MEMORY) newSize = MAX_MEMORY;
      const newMem = new Uint32Array(newSize);
      const preserveLen = Math.min(sizeAfterOut, newSize);
      for (let k = 0; k < preserveLen; k++) newMem[k] = oldMem[k];
      let pos = preserveLen;
      for (let d = 0; d < DIRS && pos < newSize; d++) {
        const inS = inflows[d];
        for (let k = 0; k < inS.length && pos < newSize; k++) newMem[pos++] = inS[k];
      }
      cell.memory = newMem;
      cell.energy = newSize;
    }
    await yield0();
  }

  // Phase 3: Recompute layouts (chunked)
  await recomputeLayoutsChunked();
  sim.step += 1;
}

// Chunked verze recomputeLayouts
async function recomputeLayoutsChunked() {
  const N = sim.N;
  const total = N*N*N;
  const cells = sim.cells;
  for (let chunkStart = 0; chunkStart < total; chunkStart += CHUNK_SIZE) {
    const chunkEnd = Math.min(chunkStart + CHUNK_SIZE, total);
    for (let i = chunkStart; i < chunkEnd; i++) {
      const cell = cells[i];
      const myE = cell.energy;
      let totalRate = 0;
      for (let d = 0; d < DIRS; d++) {
        const nE = cells[flatNeighborIdx(i, d)].energy;
        cell.rates[d] = (myE - nE > 0) ? stochasticFloor((myE - nE) * sim.coeff) : 0;
        totalRate += cell.rates[d];
      }
      const memSize = cell.memory.length;
      if (totalRate > memSize && totalRate > 0) {
        const scale = memSize / totalRate;
        let newTotal = 0;
        for (let d = 0; d < DIRS; d++) {
          cell.rates[d] = Math.floor(cell.rates[d] * scale);
          newTotal += cell.rates[d];
        }
        let leftover = memSize - newTotal;
        while (leftover > 0) {
          let added = false;
          for (let d = 0; d < DIRS; d++) {
            if (cell.rates[d] > 0 && leftover > 0) {
              cell.rates[d] += 1;
              leftover -= 1;
              added = true;
              break;
            }
          }
          if (!added) break;
        }
      }
      let cursor = cell.memory.length;
      for (const d of LAYOUT_ORDER_FROM_END) {
        cursor -= cell.rates[d];
        cell.pointers[d] = Math.max(0, cursor);
      }
    }
    await yield0();
  }
}

// Univerzální gettery - fungují jak v "main thread" módu, tak ve "worker" módu.
function getEnergyAt(i) {
  if (state && state.workerMode) {
    return workerView.energies[i] || 0;
  }
  const cell = sim.cells[i];
  return cell ? cell.energy : 0;
}
function getOriginTagAt(i) {
  if (state && state.workerMode) {
    return workerView.originTags[i] || 0;
  }
  const cell = sim.cells[i];
  return cell ? cell.originTag : 0;
}

// ===== Three.js setup =====

const container = document.getElementById("canvasContainer");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x080810);

const camera = new THREE.PerspectiveCamera(60, window.innerWidth / window.innerHeight, 0.1, 5000);
camera.position.set(50, 50, 80);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(window.innerWidth, window.innerHeight);
container.appendChild(renderer.domElement);

window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

const controls = new THREE.OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.05;

// Lights
scene.add(new THREE.AmbientLight(0xffffff, 0.4));
const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
dirLight.position.set(100, 150, 100);
scene.add(dirLight);

// ===== WSAD kamera (FPS-style movement, doplněk k OrbitControls) =====
// W/S = dopředu/dozadu po směru pohledu
// A/D = doleva/doprava (perpendikulárně)
// Q/E = dolů/nahoru (v world Y)
// Shift = sprint (3x rychleji)

const keyState = {
  w: false, a: false, s: false, d: false,
  q: false, e: false, shift: false,
};

function isInputFocused(target) {
  const tag = (target && target.tagName) || "";
  return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
}

window.addEventListener("keydown", (ev) => {
  if (isInputFocused(ev.target)) return;
  const k = ev.key.toLowerCase();
  if (k === "w") keyState.w = true;
  else if (k === "a") keyState.a = true;
  else if (k === "s") keyState.s = true;
  else if (k === "d") keyState.d = true;
  else if (k === "q") keyState.q = true;
  else if (k === "e") keyState.e = true;
  else if (k === "shift") keyState.shift = true;
});

window.addEventListener("keyup", (ev) => {
  const k = ev.key.toLowerCase();
  if (k === "w") keyState.w = false;
  else if (k === "a") keyState.a = false;
  else if (k === "s") keyState.s = false;
  else if (k === "d") keyState.d = false;
  else if (k === "q") keyState.q = false;
  else if (k === "e") keyState.e = false;
  else if (k === "shift") keyState.shift = false;
});

// Když okno ztratí fokus, resetuj všechny klávesy (jinak by zůstaly "stisknuté")
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

  // Rychlost je úměrná velikosti světa, aby pohyb byl použitelný i při N=100
  const baseSpeed = sim.N * 0.025 * (dtMs / 16);
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

  // Posun kamery i targetu - OrbitControls tak zachová vztah
  camera.position.add(_moveDelta);
  controls.target.add(_moveDelta);
}

// Boundary box (wireframe outline of world)
let worldBoundsMesh = null;
function createWorldBounds() {
  if (worldBoundsMesh) scene.remove(worldBoundsMesh);
  const N = sim.N;
  const geo = new THREE.BoxGeometry(N, N, N);
  const mat = new THREE.LineBasicMaterial({ color: 0x444466, transparent: true, opacity: 0.3 });
  const wire = new THREE.LineSegments(new THREE.EdgesGeometry(geo), mat);
  wire.position.set(N/2 - 0.5, N/2 - 0.5, N/2 - 0.5);
  scene.add(wire);
  worldBoundsMesh = wire;

  // Center camera on world (jen při resetu, ne při toggle workeru)
  const center = new THREE.Vector3(N/2, N/2, N/2);
  controls.target.copy(center);
  const dist = N * 1.8;
  camera.position.set(center.x + dist, center.y + dist * 0.6, center.z + dist);
}

// Voxel InstancedMesh
let voxelMesh = null;
const tempMatrix = new THREE.Matrix4();
const tempColor = new THREE.Color();
const tempPosition = new THREE.Vector3();
const tempQuat = new THREE.Quaternion();
const tempScale = new THREE.Vector3();

function createVoxelMesh() {
  if (voxelMesh) {
    scene.remove(voxelMesh);
    voxelMesh.geometry.dispose();
    voxelMesh.material.dispose();
  }
  const N = sim.N;
  const total = N * N * N;
  // Low-poly sféra pro voxely - 8x6 segmentů = ~48 vertices per instance
  const geo = new THREE.SphereGeometry(0.5, 8, 6);
  const mat = new THREE.MeshLambertMaterial();
  voxelMesh = new THREE.InstancedMesh(geo, mat, total);
  voxelMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  const initColor = new THREE.Color(0, 0, 0);
  for (let i = 0; i < total; i++) {
    voxelMesh.setColorAt(i, initColor);
  }
  scene.add(voxelMesh);
}

// HSV to RGB helper
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

// Heat paleta: černá → světle modrá → tmavě červená → červená → oranžová → žlutá → bílá
const HEAT_STOPS = [
  [0.00, 0.00, 0.00, 0.00],   // černá
  [0.08, 0.30, 0.55, 0.85],   // světle modrá (cold plasma)
  [0.20, 0.40, 0.10, 0.40],   // tmavě fialová/červená přechod
  [0.40, 0.85, 0.15, 0.10],   // červená
  [0.65, 1.00, 0.50, 0.05],   // oranžová
  [0.85, 1.00, 0.85, 0.30],   // žlutá
  [1.00, 1.00, 1.00, 0.95],   // bílá (nejvíc rozpálená)
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
    a[1] + (b[1] - a[1]) * lerp,
    a[2] + (b[2] - a[2]) * lerp,
    a[3] + (b[3] - a[3]) * lerp,
  ];
}

// ===== Tracker (highlight + trail) =====
// Sleduje cellu s nejvyšší energií. Trail = poslední N pozic.

const tracker = {
  current: null,        // { x, y, z, energy, idx }
  trail: [],            // pole { x, y, z, energy }
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
  const mat = new THREE.LineBasicMaterial({
    color: 0xfff0c0,
    transparent: true,
    opacity: 0.95,
    linewidth: 2,
  });
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
  const cap = Math.max(2, state.trailLen + 1);
  const geo = new THREE.BufferGeometry();
  const positions = new Float32Array(cap * 3);
  const colors = new Float32Array(cap * 3);
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  geo.setDrawRange(0, 0);
  const mat = new THREE.LineBasicMaterial({
    vertexColors: true,
    transparent: true,
    opacity: 0.85,
    linewidth: 2,
  });
  const line = new THREE.Line(geo, mat);
  line.frustumCulled = false;
  scene.add(line);
  tracker.trailLine = line;
}

function findMaxCell() {
  const N = sim.N;
  const total = N * N * N;
  let maxIdx = 0;
  let maxE = getEnergyAt(0);
  for (let i = 1; i < total; i++) {
    const e = getEnergyAt(i);
    if (e > maxE) { maxE = e; maxIdx = i; }
  }
  const x = maxIdx % N;
  const y = Math.floor(maxIdx / N) % N;
  const z = Math.floor(maxIdx / (N * N));
  return { x, y, z, energy: maxE, idx: maxIdx };
}

function updateTracker() {
  const enabled = state.trackerEnabled;
  if (!enabled) {
    if (tracker.highlightMesh) tracker.highlightMesh.visible = false;
    if (tracker.trailLine) tracker.trailLine.visible = false;
    if (dom.trackerPos) dom.trackerPos.textContent = "vypnuto";
    return;
  }

  const top = findMaxCell();
  if (top.energy <= 0) {
    if (tracker.highlightMesh) tracker.highlightMesh.visible = false;
    if (tracker.trailLine) tracker.trailLine.visible = false;
    if (dom.trackerPos) dom.trackerPos.textContent = "-";
    return;
  }

  tracker.current = top;

  // Append to trail (jen když se cell posunula)
  const last = tracker.trail.length > 0 ? tracker.trail[tracker.trail.length - 1] : null;
  if (!last || last.x !== top.x || last.y !== top.y || last.z !== top.z) {
    tracker.trail.push({ x: top.x, y: top.y, z: top.z, energy: top.energy });
    // Ořež historii na povolenou délku
    while (tracker.trail.length > state.trailLen + 1) tracker.trail.shift();
  } else {
    last.energy = top.energy;
  }

  // Highlight (wireframe kostka okolo aktivní cely)
  if (tracker.highlightMesh) {
    tracker.highlightMesh.visible = true;
    tracker.highlightMesh.position.set(top.x, top.y, top.z);
    // Lehká pulsace velikosti aby šel highlight víc vidět
    const pulse = 1.0 + 0.08 * Math.sin(performance.now() / 250);
    tracker.highlightMesh.scale.setScalar(pulse);
  }

  // Trail
  if (tracker.trailLine) {
    const len = tracker.trail.length;
    tracker.trailLine.visible = (state.trailLen > 0) && (len > 1);
    if (tracker.trailLine.visible) {
      const positions = tracker.trailLine.geometry.attributes.position.array;
      const colors = tracker.trailLine.geometry.attributes.color.array;
      const cap = positions.length / 3;
      const drawLen = Math.min(len, cap);
      for (let i = 0; i < drawLen; i++) {
        const p = tracker.trail[len - drawLen + i];
        positions[i*3]     = p.x;
        positions[i*3 + 1] = p.y;
        positions[i*3 + 2] = p.z;
        // Fade: starší pozice tmavší/červenější, novější jasnější/žluté
        const f = (i + 1) / drawLen;  // 0..1, 1 = nejnovější
        colors[i*3]     = 1.00 * f;
        colors[i*3 + 1] = 0.80 * f;
        colors[i*3 + 2] = 0.20 * f;
      }
      tracker.trailLine.geometry.setDrawRange(0, drawLen);
      tracker.trailLine.geometry.attributes.position.needsUpdate = true;
      tracker.trailLine.geometry.attributes.color.needsUpdate = true;
    }
  }

  if (dom.trackerPos) {
    dom.trackerPos.textContent = `(${top.x},${top.y},${top.z}) E=${top.energy.toLocaleString()}`;
  }
}

// Update voxel positions and colors based on simulation state
let visibleCount = 0;
function updateVoxels() {
  const N = sim.N;
  const minE = state.minEnergy;
  const colorMode = state.colorMode;
  const voxelSize = state.voxelSize;
  const total = N * N * N;

  // Find max energy + total energy for normalization
  let maxE = 0;
  let totalE = 0;
  for (let i = 0; i < total; i++) {
    const e = getEnergyAt(i);
    if (e > maxE) maxE = e;
    totalE += e;
  }
  if (maxE < 1) maxE = 1;

  // Předpočítej škálovací funkci pro energy → t [0,1]
  // Relativní: t = sqrt(e / maxE) - vždycky někdo bude bílý
  // Absolutní: t = log(1+e) / log(1+totalE) - bílá jen když cela drží
  //            významnou frakci celkové energie světa. Po equilibraci pole zhasne.
  let energyToT;
  if (state.energyScale === "absolute") {
    const denom = Math.log(1 + Math.max(1, totalE));
    energyToT = (e) => (e <= 0 ? 0 : Math.log(1 + e) / denom);
  } else {
    energyToT = (e) => Math.sqrt(e / maxE);
  }

  // Tracker fade: když je tracker aktivní a má vybranou cellu, ostatní cely ztlumíme
  // Multiplikátor pro RGB. 1.0 = žádné ztlumení, 0.0 = ostatní úplně černé.
  const dimActive = state.trackerEnabled && tracker.current && tracker.current.energy > 0;
  const trackedIdx = dimActive ? tracker.current.idx : -1;
  const fadeMul = dimActive ? (1 - state.dimOthers) : 1.0;

  visibleCount = 0;
  for (let z = 0; z < N; z++) {
    for (let y = 0; y < N; y++) {
      for (let x = 0; x < N; x++) {
        const i = idx(x, y, z);
        const e = getEnergyAt(i);
        let scale;
        let r, g, b;
        if (e <= minE) {
          scale = 0;
          r = g = b = 0;
        } else {
          scale = voxelSize;
          const t = energyToT(e);
          if (colorMode === "energy") {
            const rgb = heatColor(t);
            r = rgb[0]; g = rgb[1]; b = rgb[2];
          } else {
            const tag = getOriginTagAt(i) >>> 0;
            const hue = (tag % 360) / 360;
            const v = 0.4 + 0.6 * t;
            const rgb = hsvToRgb(hue, 0.7, v);
            r = rgb[0]; g = rgb[1]; b = rgb[2];
          }
          // Tracker: ztlum všechny cely kromě sledované
          if (dimActive && i !== trackedIdx) {
            r *= fadeMul;
            g *= fadeMul;
            b *= fadeMul;
          }
          visibleCount++;
        }

        tempPosition.set(x, y, z);
        tempScale.set(scale, scale, scale);
        tempMatrix.compose(tempPosition, tempQuat, tempScale);
        voxelMesh.setMatrixAt(i, tempMatrix);
        tempColor.setRGB(r, g, b);
        voxelMesh.setColorAt(i, tempColor);
      }
    }
  }
  voxelMesh.instanceMatrix.needsUpdate = true;
  if (voxelMesh.instanceColor) voxelMesh.instanceColor.needsUpdate = true;
}

// ===== UI state =====

const state = {
  running: false,
  minEnergy: 0,
  colorMode: "energy",
  energyScale: "relative",
  voxelSize: 0.85,
  trackerEnabled: true,
  trailLen: 60,
  dimOthers: 0.82,   // 0 = bez ztlumení, 1 = ostatní cely úplně černé
  workerMode: false,
  workerInstance: null,
  dirty: false,
};

const dom = {
  cellCount: document.getElementById("cellCount"),
  visibleCount: document.getElementById("visibleCount"),
  fps: document.getElementById("fps"),
  msPerTick: document.getElementById("msPerTick"),
  step: document.getElementById("step"),
  totalE: document.getElementById("totalE"),
  runBtn: document.getElementById("runBtn"),
  stepBtn: document.getElementById("stepBtn"),
  resetBtn: document.getElementById("resetBtn"),
  sizeInput: document.getElementById("sizeInput"),
  scenarioInput: document.getElementById("scenarioInput"),
  coeffInput: document.getElementById("coeffInput"),
  coeffVal: document.getElementById("coeffVal"),
  colorMode: document.getElementById("colorMode"),
  energyScale: document.getElementById("energyScale"),
  minEnergy: document.getElementById("minEnergy"),
  minEnergyVal: document.getElementById("minEnergyVal"),
  voxelSize: document.getElementById("voxelSize"),
  trackerEnabled: document.getElementById("trackerEnabled"),
  trailLen: document.getElementById("trailLen"),
  trailLenVal: document.getElementById("trailLenVal"),
  dimOthers: document.getElementById("dimOthers"),
  dimOthersVal: document.getElementById("dimOthersVal"),
  trackerPos: document.getElementById("trackerPos"),
  useWorker: document.getElementById("useWorker"),
  workerStatus: document.getElementById("workerStatus"),
};

// Perf tracking
const perfRing = { ms: [], fps: [], lastFrame: 0, ringSize: 30, lastHudUpdate: 0 };
function avgRing(ring) {
  if (ring.length === 0) return 0;
  let s = 0;
  for (const v of ring) s += v;
  return s / ring.length;
}

function updateHud() {
  let total = 0;
  if (state.workerMode) {
    for (let i = 0; i < workerView.energies.length; i++) total += workerView.energies[i];
    dom.cellCount.textContent = workerView.energies.length.toLocaleString();
  } else {
    for (const c of sim.cells) total += c.energy;
    dom.cellCount.textContent = sim.cells.length.toLocaleString();
  }
  dom.visibleCount.textContent = visibleCount.toLocaleString();
  dom.fps.textContent = avgRing(perfRing.fps).toFixed(1);
  dom.msPerTick.textContent = avgRing(perfRing.ms).toFixed(1);
  dom.step.textContent = sim.step;
  dom.totalE.textContent = total.toLocaleString();
}

// ===== Web Worker integration =====

// Pokus o new Worker. Chrome blokuje file:// → file:// jako "null origin",
// fallback: načti worker.js přes XHR a obal do Blob URL.
function tryConstructWorker() {
  try {
    return new Worker("worker.js");
  } catch (e1) {
    console.warn("Přímý Worker selhal, zkouším Blob fallback:", e1.message);
    try {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", "worker.js", false); // synchronní, jednorazově při startu
      xhr.send();
      if (xhr.status !== 0 && xhr.status !== 200) {
        throw new Error("XHR status " + xhr.status);
      }
      const blob = new Blob([xhr.responseText], { type: "text/javascript" });
      return new Worker(URL.createObjectURL(blob));
    } catch (e2) {
      throw new Error("file:// fallback selhal: " + e2.message);
    }
  }
}

function startWorker() {
  if (state.workerInstance) return true;
  try {
    const w = tryConstructWorker();
    w.onmessage = (ev) => {
      const msg = ev.data;
      if (msg.type === "state") {
        workerView.energies = msg.energies;
        sim.step = msg.step;
        if (typeof msg.msPerTick === "number") {
          perfRing.ms.push(msg.msPerTick);
          if (perfRing.ms.length > perfRing.ringSize) perfRing.ms.shift();
        }
        state.dirty = true;
      } else if (msg.type === "origins") {
        workerView.originTags = msg.tags;
        state.dirty = true;
      }
    };
    w.onerror = (err) => {
      console.error("Worker error:", err);
      dom.workerStatus.textContent = "chyba: " + (err.message || "?");
    };
    state.workerInstance = w;
    dom.workerStatus.textContent = "aktivní";
    return true;
  } catch (e) {
    console.warn("Worker se nepodařilo spustit:", e);
    dom.workerStatus.textContent = "nedostupný (" + (e.message || "?") + ")";
    return false;
  }
}

function stopWorker() {
  if (!state.workerInstance) return;
  try {
    state.workerInstance.postMessage({ type: "run", running: false });
    state.workerInstance.terminate();
  } catch (e) { /* ignore */ }
  state.workerInstance = null;
  dom.workerStatus.textContent = "vypnut";
}

function workerReset() {
  if (!state.workerInstance) return;
  state.workerInstance.postMessage({
    type: "reset",
    N: sim.N,
    coeff: sim.coeff,
    scenario: dom.scenarioInput.value,
  });
}

// ===== Reset and run =====

function fullReset() {
  sim.N = Math.max(8, Math.min(100, parseInt(dom.sizeInput.value, 10) || 32));
  sim.coeff = parseFloat(dom.coeffInput.value);

  if (state.workerMode) {
    if (!state.workerInstance) startWorker();
    if (state.workerInstance) {
      // Worker bude posílat origins + první state samostatně.
      // Lokální sim.cells nepotřebujeme držet plné, ale pro fallback inicializujeme.
      sim.cells = [];
      sim.step = 0;
      workerView.energies = new Uint32Array(sim.N * sim.N * sim.N);
      workerView.originTags = new Uint32Array(sim.N * sim.N * sim.N);
      workerReset();
    } else {
      // Worker selhal - fallback na main-thread
      state.workerMode = false;
      dom.useWorker.checked = false;
      resetWorld();
      initScenario(dom.scenarioInput.value);
    }
  } else {
    resetWorld();
    initScenario(dom.scenarioInput.value);
  }

  createWorldBounds();
  createVoxelMesh();
  createHighlightMesh();
  createTrailLine();
  tracker.trail = [];
  tracker.current = null;
  state.dirty = true;
  updateHud();
}

dom.runBtn.addEventListener("click", () => {
  state.running = !state.running;
  dom.runBtn.textContent = state.running ? "Pauza" : "Spustit";
  if (state.workerMode && state.workerInstance) {
    state.workerInstance.postMessage({ type: "run", running: state.running });
  }
});
dom.stepBtn.addEventListener("click", async () => {
  if (state.workerMode) {
    if (state.workerInstance) {
      state.workerInstance.postMessage({ type: "step" });
    }
  } else {
    const t0 = performance.now();
    await step();
    const t1 = performance.now();
    perfRing.ms.push(t1 - t0);
    if (perfRing.ms.length > perfRing.ringSize) perfRing.ms.shift();
    state.dirty = true;
  }
});
dom.resetBtn.addEventListener("click", () => {
  state.running = false;
  dom.runBtn.textContent = "Spustit";
  fullReset();
});

dom.coeffInput.addEventListener("input", () => {
  sim.coeff = parseFloat(dom.coeffInput.value);
  dom.coeffVal.textContent = sim.coeff.toFixed(2);
  if (state.workerMode && state.workerInstance) {
    state.workerInstance.postMessage({ type: "setCoeff", coeff: sim.coeff });
  } else {
    recomputeLayouts();
  }
});
dom.colorMode.addEventListener("change", () => {
  state.colorMode = dom.colorMode.value;
  state.dirty = true;
});
dom.energyScale.addEventListener("change", () => {
  state.energyScale = dom.energyScale.value;
  state.dirty = true;
});
dom.minEnergy.addEventListener("input", () => {
  state.minEnergy = parseInt(dom.minEnergy.value, 10);
  dom.minEnergyVal.textContent = state.minEnergy;
  state.dirty = true;
});
dom.voxelSize.addEventListener("input", () => {
  state.voxelSize = parseFloat(dom.voxelSize.value);
  state.dirty = true;
});

dom.trackerEnabled.addEventListener("change", () => {
  state.trackerEnabled = dom.trackerEnabled.checked;
  state.dirty = true;
});
dom.trailLen.addEventListener("input", () => {
  state.trailLen = parseInt(dom.trailLen.value, 10);
  dom.trailLenVal.textContent = state.trailLen;
  // Realokovat buffer trail line - geometrie má fixní kapacitu
  createTrailLine();
  state.dirty = true;
});
dom.dimOthers.addEventListener("input", () => {
  state.dimOthers = parseFloat(dom.dimOthers.value);
  dom.dimOthersVal.textContent = state.dimOthers.toFixed(2);
  state.dirty = true;
});

dom.useWorker.addEventListener("change", () => {
  const wantWorker = dom.useWorker.checked;
  if (wantWorker === state.workerMode) return;
  state.running = false;
  dom.runBtn.textContent = "Spustit";
  if (wantWorker) {
    state.workerMode = true;
    if (!startWorker()) {
      // startWorker už vrátí stav zpátky na false
      state.workerMode = false;
      dom.useWorker.checked = false;
    }
  } else {
    stopWorker();
    state.workerMode = false;
  }
  fullReset();
});

// ===== Animation loop =====

state.dirty = false;

function renderLoop(now) {
  // FPS tracking
  if (perfRing.lastFrame > 0) {
    const dt = now - perfRing.lastFrame;
    perfRing.fps.push(1000 / dt);
    if (perfRing.fps.length > perfRing.ringSize) perfRing.fps.shift();
  }
  const dtMs = perfRing.lastFrame > 0 ? (now - perfRing.lastFrame) : 16;
  perfRing.lastFrame = now;

  // WSAD pohyb kamery (per frame)
  applyWsad(dtMs);

  // Update voxely jen když je nový stav.
  // Tracker MUSÍ proběhnout dřív než voxely, protože updateVoxels používá
  // tracker.current.idx pro ztlumení netrackovaných cel.
  if (state.dirty) {
    updateTracker();
    updateVoxels();
    state.dirty = false;
  }

  controls.update();
  renderer.render(scene, camera);

  // HUD update sampling
  if (Math.floor(now / 100) !== perfRing.lastHudUpdate) {
    perfRing.lastHudUpdate = Math.floor(now / 100);
    updateHud();
  }
  requestAnimationFrame(renderLoop);
}

async function simLoop() {
  while (true) {
    if (state.running && !state.workerMode) {
      const t0 = performance.now();
      await step();
      const t1 = performance.now();
      perfRing.ms.push(t1 - t0);
      if (perfRing.ms.length > perfRing.ringSize) perfRing.ms.shift();
      state.dirty = true;
    } else {
      await new Promise(r => setTimeout(r, 50));
    }
  }
}

// ===== Init =====

// Načti počáteční hodnoty z UI checkboxů
state.workerMode = dom.useWorker.checked;
state.trackerEnabled = dom.trackerEnabled.checked;
state.trailLen = parseInt(dom.trailLen.value, 10);
dom.trailLenVal.textContent = state.trailLen;
state.dimOthers = parseFloat(dom.dimOthers.value);
dom.dimOthersVal.textContent = state.dimOthers.toFixed(2);

if (state.workerMode) {
  if (!startWorker()) {
    state.workerMode = false;
    dom.useWorker.checked = false;
  }
} else {
  dom.workerStatus.textContent = "vypnut";
}

fullReset();
state.dirty = true;
requestAnimationFrame(renderLoop);
simLoop();

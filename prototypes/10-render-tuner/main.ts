// Aenternis - prototyp 10: Render tuner.
//
// Tournament-style chooser pro parametry render bloku v /web/. Statický
// svět se vygeneruje jednou na startu (WASM, deterministic seed) a pak
// se z něj renderuje 5x2 mřížka náhledů — každý s jinou hodnotou
// aktuálně laděného parametru. Uživatel klikne na variantu, hodnota se
// zafixuje, prototyp přejde do dalšího kola pro další parametr.
//
// Render pipeline replikuje /web/main.ts (ambient + directional light,
// baked space env, instanced voxel mesh s heat-color a jitter, SSAO +
// UnrealBloom + ACES tonemap). 10 tiles sdílí jeden WebGLRenderer +
// jednu kameru s OrbitControls; každý tile má vlastní Scene + Composer
// s vlastní instancí všech passes.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { BokehPass } from 'three/addons/postprocessing/BokehPass.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { SSAOPass } from 'three/addons/postprocessing/SSAOPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';

import init, { World } from '../../crates/aenternis-wasm/pkg/aenternis_wasm.js';
import { fitCamera } from '../../src/camera-fit.ts';
import { heatColor, meanRelativeT, voxelSizeFactor } from '../../src/heat.ts';
import { JITTER_AMPLITUDE, gridJitter } from '../../src/jitter.ts';
import type { SnapshotBbox } from '../../src/snapshot.ts';

// ----- Konfigurace -----------------------------------------------------------

const CAPTURE_SEED = 1234;
const CAPTURE_ENERGY = 1_000_000;
const CAPTURE_TICKS = 250;
const CAPTURE_COEFF = 0.15;
const CAPTURE_K = 1;
const CAPTURE_MOVE_THRESHOLD = 1.0;

const GRID_COLS = 5;
const GRID_ROWS = 2;
const TILES_PER_ROUND = GRID_COLS * GRID_ROWS;

interface ParamDef {
  readonly name: keyof RenderParams;
  readonly label: string;
  readonly min: number;
  readonly max: number;
  readonly step: number;
  readonly default: number;
  readonly precision: number;
}

// Pořadí kol — od největšího vizuálního dopadu k nejmenšímu.
// voxelSize range/default zrcadlí produkční /web/index.html (krystaly do 4.5×);
// DoF kola (focus/aperture/maxblur) odpovídají BokehPass sliderům ve vieweru.
const PARAM_DEFS: readonly ParamDef[] = [
  { name: 'exposure',       label: 'tonemap exposure', min: 0.3,  max: 2.5,  step: 0.05,  default: 1.00,  precision: 2 },
  { name: 'emissive',       label: 'emissive boost',   min: 0.0,  max: 2.0,  step: 0.05,  default: 0.50,  precision: 2 },
  { name: 'roughness',      label: 'roughness',        min: 0.0,  max: 1.0,  step: 0.05,  default: 0.60,  precision: 2 },
  { name: 'bloomStrength',  label: 'bloom strength',   min: 0.0,  max: 2.0,  step: 0.05,  default: 0.80,  precision: 2 },
  { name: 'bloomThreshold', label: 'bloom threshold',  min: 0.0,  max: 1.0,  step: 0.05,  default: 0.70,  precision: 2 },
  { name: 'bloomRadius',    label: 'bloom radius',     min: 0.0,  max: 1.0,  step: 0.05,  default: 0.40,  precision: 2 },
  { name: 'fogDensity',     label: 'fog density',      min: 0.0,  max: 0.03, step: 0.001, default: 0.005, precision: 3 },
  { name: 'ssaoRadius',     label: 'SSAO radius',      min: 1.0,  max: 20.0, step: 0.5,   default: 8.0,   precision: 1 },
  { name: 'voxelSize',      label: 'voxel size',       min: 0.2,  max: 4.5,  step: 0.05,  default: 4.50,  precision: 2 },
  { name: 'minLuma',        label: 'min luma (cull)',  min: 0.0,  max: 1.00, step: 0.02,  default: 0.42,  precision: 2 },
  { name: 'dofFocus',       label: 'DoF focus',        min: 0.0,  max: 400,  step: 5,     default: 50,    precision: 0 },
  { name: 'dofAperture',    label: 'DoF aperture',     min: 0.0,  max: 0.1,  step: 0.001, default: 0.025, precision: 3 },
  { name: 'dofMaxblur',     label: 'DoF maxblur',      min: 0.0,  max: 0.05, step: 0.001, default: 0.010, precision: 3 },
];

interface RenderParams {
  exposure: number;
  emissive: number;
  roughness: number;
  bloomStrength: number;
  bloomThreshold: number;
  bloomRadius: number;
  fogDensity: number;
  ssaoRadius: number;
  voxelSize: number;
  minLuma: number;
  dofFocus: number;
  dofAperture: number;
  dofMaxblur: number;
}

function defaultParams(): RenderParams {
  const out: Partial<Record<keyof RenderParams, number>> = {};
  for (const def of PARAM_DEFS) {
    out[def.name] = def.default;
  }
  return out as RenderParams;
}

// ----- History (persistent across sessions) -----------------------------------

const HISTORY_KEY = 'aenternis.tuner.history';
const HISTORY_MAX = 10;

function loadHistory(): RenderParams[] {
  try {
    const raw = window.localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    // Backfill — staré entries z localStorage nemají nově přidané klíče (např.
    // minLuma); jinak by .toFixed() na undefined spadlo. Default doplní díru.
    const defaults = defaultParams();
    return parsed.map((h) => ({ ...defaults, ...(h as Partial<RenderParams>) }));
  } catch {
    return [];
  }
}

function saveHistory(history: RenderParams[]): void {
  try {
    window.localStorage.setItem(HISTORY_KEY, JSON.stringify(history));
  } catch {
    // Ignore quota errors etc. — history je nice-to-have, ne kritický stav.
  }
}

function appendHistory(history: RenderParams[], params: RenderParams): RenderParams[] {
  // Dedupe: pokud je poslední záznam identický, nepřidávej.
  const last = history[history.length - 1];
  if (last && JSON.stringify(last) === JSON.stringify(params)) return history;
  const next = [...history, params];
  if (next.length > HISTORY_MAX) next.splice(0, next.length - HISTORY_MAX);
  saveHistory(next);
  return next;
}

function medianOf(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[mid - 1]! + sorted[mid]!) / 2
    : sorted[mid]!;
}

function medianParams(history: RenderParams[]): RenderParams {
  const result: Partial<Record<keyof RenderParams, number>> = {};
  for (const def of PARAM_DEFS) {
    const values = history.map((h) => h[def.name]);
    const m = medianOf(values);
    const stepped = Math.round(m / def.step) * def.step;
    const clamped = Math.max(def.min, Math.min(def.max, stepped));
    result[def.name] = Number(clamped.toFixed(def.precision));
  }
  return result as RenderParams;
}

// Voxel geometrie konstanty (z /web/main.ts). Viewer teď defaultně renderuje
// flat-shaded krystaly, takže tuner ladí proti stejnému povrchu.
const VOXEL_CRYSTAL_RADIUS = 0.6;
const TWO_PI = Math.PI * 2;

// ----- Snapshot data ---------------------------------------------------------

interface SnapshotData {
  readonly snap: Uint32Array;
  readonly stride: number;
  readonly cellCount: number;
  readonly totalEnergy: number;
  readonly bbox: SnapshotBbox;
  readonly tick: number;
  readonly seed: number;
  readonly ticks: number;
  readonly energyIn: number;
}

interface SnapshotMeta {
  readonly stride: number;
  readonly cellCount: number;
  readonly totalEnergy: number;
  readonly bbox: SnapshotBbox;
  readonly tick: number;
  readonly seed: number;
  readonly ticks: number;
  readonly energyIn: number;
}

function setStatus(text: string): void {
  const el = document.getElementById('status');
  if (el) el.textContent = text;
}

function hideStatus(): void {
  const el = document.getElementById('status');
  if (el) el.style.display = 'none';
}

async function tryLoadStaticSnapshot(): Promise<SnapshotData | null> {
  try {
    const [binResp, metaResp] = await Promise.all([
      fetch('./snapshot.bin'),
      fetch('./snapshot.meta.json'),
    ]);
    if (!binResp.ok || !metaResp.ok) return null;
    const [binBuf, meta] = await Promise.all([
      binResp.arrayBuffer(),
      metaResp.json() as Promise<SnapshotMeta>,
    ]);
    const snap = new Uint32Array(binBuf);
    return { ...meta, snap };
  } catch {
    return null;
  }
}

async function captureSnapshot(): Promise<SnapshotData> {
  setStatus('Načítám WASM…');
  await init();

  setStatus('Inicializuji svět…');
  const world = World.newWithProgram(CAPTURE_SEED, CAPTURE_ENERGY, new Uint32Array(0));
  world.setMoveThreshold(CAPTURE_MOVE_THRESHOLD);

  // Tickujeme po dávkách, aby browser nezamrzl a status mohl tickovat.
  const BATCH = 50;
  for (let done = 0; done < CAPTURE_TICKS; done += BATCH) {
    const end = Math.min(CAPTURE_TICKS, done + BATCH);
    for (let i = done; i < end; i += 1) {
      world.step(CAPTURE_COEFF, CAPTURE_K);
    }
    setStatus(`Generuji svět… tick ${end}/${CAPTURE_TICKS}`);
    await new Promise((r) => requestAnimationFrame(() => r(null)));
  }

  const view = world.cellsSnapshotView();
  const snap = new Uint32Array(view); // Kopie z WASM linear memory.
  const bboxArr = world.boundingBox();
  if (bboxArr.length < 6) {
    throw new Error('World produced empty bbox after capture ticks');
  }
  const bbox: SnapshotBbox = {
    minX: bboxArr[0]!, maxX: bboxArr[1]!,
    minY: bboxArr[2]!, maxY: bboxArr[3]!,
    minZ: bboxArr[4]!, maxZ: bboxArr[5]!,
  };
  const data: SnapshotData = {
    snap,
    stride: world.snapshotStride,
    cellCount: world.cellCount(),
    totalEnergy: world.totalEnergy(),
    bbox,
    tick: world.tick(),
    seed: CAPTURE_SEED,
    ticks: CAPTURE_TICKS,
    energyIn: CAPTURE_ENERGY,
  };
  world.free();
  return data;
}

function downloadSnapshot(snapshot: SnapshotData): void {
  // Uint32Array's .buffer is typed as ArrayBufferLike (could be SharedArrayBuffer);
  // we constructed snap from a non-shared ArrayBuffer so the cast is safe.
  const binBlob = new Blob([snapshot.snap.buffer as ArrayBuffer], { type: 'application/octet-stream' });
  const meta: SnapshotMeta = {
    stride: snapshot.stride,
    cellCount: snapshot.cellCount,
    totalEnergy: snapshot.totalEnergy,
    bbox: snapshot.bbox,
    tick: snapshot.tick,
    seed: snapshot.seed,
    ticks: snapshot.ticks,
    energyIn: snapshot.energyIn,
  };
  const metaBlob = new Blob([JSON.stringify(meta, null, 2)], { type: 'application/json' });
  const triggerDownload = (blob: Blob, name: string): void => {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };
  triggerDownload(binBlob, 'snapshot.bin');
  triggerDownload(metaBlob, 'snapshot.meta.json');
}

// ----- THREE setup -----------------------------------------------------------

interface Tile {
  readonly scene: THREE.Scene;
  readonly fog: THREE.FogExp2;
  readonly material: THREE.MeshStandardMaterial;
  materialShader: THREE.WebGLProgramParametersWithUniforms | null;
  readonly mesh: THREE.InstancedMesh;
  readonly composer: EffectComposer;
  readonly ssaoPass: SSAOPass;
  readonly bokehPass: BokehPass;
  readonly bloomPass: UnrealBloomPass;
  params: RenderParams;
}

const SHARED_GEOMETRY = new THREE.IcosahedronGeometry(VOXEL_CRYSTAL_RADIUS, 0);

function buildSpaceEnvironment(renderer: THREE.WebGLRenderer): THREE.Texture {
  // Kopie /web/main.ts:buildSpaceEnvironment — gradient sphere + 800 stars,
  // baked přes PMREMGenerator do cubemap textury.
  const skyScene = new THREE.Scene();
  const skyGeom = new THREE.SphereGeometry(500, 32, 16);
  skyGeom.scale(-1, 1, 1);
  const skyMat = new THREE.ShaderMaterial({
    uniforms: {
      topColor: { value: new THREE.Color(0x101830) },
      bottomColor: { value: new THREE.Color(0x000005) },
    },
    vertexShader: `
      varying vec3 vWorldPos;
      void main() {
        vec4 wp = modelMatrix * vec4(position, 1.0);
        vWorldPos = wp.xyz;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: `
      uniform vec3 topColor;
      uniform vec3 bottomColor;
      varying vec3 vWorldPos;
      void main() {
        float h = normalize(vWorldPos).y * 0.5 + 0.5;
        gl_FragColor = vec4(mix(bottomColor, topColor, h), 1.0);
      }
    `,
    side: THREE.BackSide,
    depthWrite: false,
  });
  skyScene.add(new THREE.Mesh(skyGeom, skyMat));

  const starCount = 800;
  const starGeom = new THREE.BufferGeometry();
  const positions = new Float32Array(starCount * 3);
  const colors = new Float32Array(starCount * 3);
  for (let i = 0; i < starCount; i += 1) {
    const u = Math.random() * 2 - 1;
    const t = Math.random() * Math.PI * 2;
    const r = 480;
    const sinPhi = Math.sqrt(1 - u * u);
    positions[i * 3] = r * sinPhi * Math.cos(t);
    positions[i * 3 + 1] = r * u;
    positions[i * 3 + 2] = r * sinPhi * Math.sin(t);
    const warm = Math.random() < 0.15 ? 1.0 : 0.0;
    const c = 0.6 + Math.random() * 0.4;
    colors[i * 3] = c * (0.8 + warm * 0.2);
    colors[i * 3 + 1] = c * (0.8 + warm * 0.05);
    colors[i * 3 + 2] = c * (1.0 - warm * 0.2);
  }
  starGeom.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  starGeom.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  const starMat = new THREE.PointsMaterial({
    size: 2.5,
    sizeAttenuation: false,
    vertexColors: true,
    transparent: true,
    opacity: 0.9,
    depthWrite: false,
  });
  skyScene.add(new THREE.Points(starGeom, starMat));

  const pmrem = new THREE.PMREMGenerator(renderer);
  pmrem.compileEquirectangularShader();
  const rt = pmrem.fromScene(skyScene);
  pmrem.dispose();
  skyGeom.dispose();
  skyMat.dispose();
  starGeom.dispose();
  starMat.dispose();
  return rt.texture;
}

function populateMesh(
  mesh: THREE.InstancedMesh,
  snap: Uint32Array,
  stride: number,
  cellCount: number,
  totalEnergy: number,
  voxelScale: number,
): void {
  const tempMatrix = new THREE.Matrix4();
  const tempPos = new THREE.Vector3();
  const tempQuat = new THREE.Quaternion();
  const tempEuler = new THREE.Euler();
  const tempScale = new THREE.Vector3();
  const tempColor = new THREE.Color();
  for (let i = 0; i < cellCount; i += 1) {
    const off = i * stride;
    const x = snap[off]! | 0;
    const y = snap[off + 1]! | 0;
    const z = snap[off + 2]! | 0;
    const e = snap[off + 3]!;
    const t = meanRelativeT(e, totalEnergy, cellCount);
    const perScale = voxelScale * voxelSizeFactor(t);
    tempScale.set(perScale, perScale, perScale);
    tempPos.set(
      x + gridJitter(x, y, z, 0) * JITTER_AMPLITUDE,
      y + gridJitter(x, y, z, 1) * JITTER_AMPLITUDE,
      z + gridJitter(x, y, z, 2) * JITTER_AMPLITUDE,
    );
    // Deterministic per-cell crystal orientation (matches /web/main.ts) — the
    // same jitter hash as the position offset, so facets don't align into moire.
    tempEuler.set(
      gridJitter(x, y, z, 0) * TWO_PI,
      gridJitter(x, y, z, 1) * TWO_PI,
      gridJitter(x, y, z, 2) * TWO_PI,
    );
    tempQuat.setFromEuler(tempEuler);
    tempMatrix.compose(tempPos, tempQuat, tempScale);
    mesh.setMatrixAt(i, tempMatrix);
    const [r, g, b] = heatColor(t);
    tempColor.setRGB(r, g, b);
    mesh.setColorAt(i, tempColor);
  }
  mesh.instanceMatrix.needsUpdate = true;
  if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
}

function createTile(
  renderer: THREE.WebGLRenderer,
  camera: THREE.PerspectiveCamera,
  env: THREE.Texture,
  snapshot: SnapshotData,
  tileWidth: number,
  tileHeight: number,
  params: RenderParams,
): Tile {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x05050a);
  scene.environment = env;

  const fog = new THREE.FogExp2(0x05050a, params.fogDensity);
  scene.fog = fog;

  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
  dirLight.position.set(50, 80, 50);
  scene.add(dirLight);

  const material = new THREE.MeshStandardMaterial({
    metalness: 0.0,
    roughness: params.roughness,
    flatShading: true, // hard crystal facets (viewer default shape).
  });
  let materialShader: THREE.WebGLProgramParametersWithUniforms | null = null;
  material.onBeforeCompile = (shader) => {
    shader.uniforms['uEmissiveBoost'] = { value: params.emissive };
    shader.uniforms['uMinLuma'] = { value: params.minLuma };
    shader.fragmentShader = shader.fragmentShader
      .replace(
        'uniform vec3 emissive;',
        `uniform vec3 emissive;
uniform float uEmissiveBoost;
uniform float uMinLuma;`,
      )
      // Alpha-test discard pro low-energy entities: po color_fragment chunk-u
      // už diffuseColor.rgb obsahuje per-instance heat color, takže luminanci
      // z něj porovnáme s prahem. discard běží před lighting passes, takže
      // se ušetří i fragment work.
      .replace(
        '#include <color_fragment>',
        `#include <color_fragment>
if (dot(diffuseColor.rgb, vec3(0.299, 0.587, 0.114)) < uMinLuma) discard;`,
      )
      .replace(
        '#include <emissivemap_fragment>',
        `#include <emissivemap_fragment>
totalEmissiveRadiance += diffuseColor.rgb * uEmissiveBoost;`,
      );
    materialShader = shader;
  };

  const mesh = new THREE.InstancedMesh(SHARED_GEOMETRY, material, snapshot.cellCount);
  mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  // Inicializace color bufferu na 0 (jinak by ho three pre-fillnul bílou).
  const initColor = new THREE.Color(0, 0, 0);
  for (let i = 0; i < snapshot.cellCount; i += 1) {
    mesh.setColorAt(i, initColor);
  }
  scene.add(mesh);

  populateMesh(mesh, snapshot.snap, snapshot.stride, snapshot.cellCount, snapshot.totalEnergy, params.voxelSize);

  const composer = new EffectComposer(renderer);
  composer.setSize(tileWidth, tileHeight);
  composer.addPass(new RenderPass(scene, camera));
  const ssaoPass = new SSAOPass(scene, camera, tileWidth, tileHeight);
  ssaoPass.kernelRadius = params.ssaoRadius;
  composer.addPass(ssaoPass);
  // DoF before bloom (same order as /web/main.ts). Always enabled here — the
  // tournament tunes focus/aperture/maxblur, so the pass must be live; at the
  // default maxblur the blur is mild, so the earlier rounds stay readable.
  const bokehPass = new BokehPass(scene, camera, {
    focus: params.dofFocus,
    aperture: params.dofAperture,
    maxblur: params.dofMaxblur,
  });
  composer.addPass(bokehPass);
  const bloomPass = new UnrealBloomPass(
    new THREE.Vector2(tileWidth, tileHeight),
    params.bloomStrength,
    params.bloomRadius,
    params.bloomThreshold,
  );
  composer.addPass(bloomPass);
  composer.addPass(new OutputPass());

  return {
    scene, fog, material, mesh, composer, ssaoPass, bokehPass, bloomPass,
    params: { ...params },
    get materialShader() { return materialShader; },
    set materialShader(s) { materialShader = s; },
  } as Tile;
}

function applyTileParams(
  tile: Tile,
  next: RenderParams,
  snapshot: SnapshotData,
): void {
  const prev = tile.params;
  tile.fog.density = next.fogDensity;
  tile.material.roughness = next.roughness;
  if (tile.materialShader) {
    tile.materialShader.uniforms['uEmissiveBoost']!.value = next.emissive;
    tile.materialShader.uniforms['uMinLuma']!.value = next.minLuma;
  }
  tile.ssaoPass.kernelRadius = next.ssaoRadius;
  // three types BokehPass.uniforms as an opaque {}, so cast to poke values.
  const bokeh = tile.bokehPass.uniforms as Record<string, THREE.IUniform>;
  bokeh['focus']!.value = next.dofFocus;
  bokeh['aperture']!.value = next.dofAperture;
  bokeh['maxblur']!.value = next.dofMaxblur;
  tile.bloomPass.threshold = next.bloomThreshold;
  tile.bloomPass.strength = next.bloomStrength;
  tile.bloomPass.radius = next.bloomRadius;
  if (prev.voxelSize !== next.voxelSize) {
    populateMesh(
      tile.mesh, snapshot.snap, snapshot.stride, snapshot.cellCount,
      snapshot.totalEnergy, next.voxelSize,
    );
  }
  tile.params = { ...next };
}

function resizeTile(tile: Tile, w: number, h: number): void {
  tile.composer.setSize(w, h);
  tile.ssaoPass.setSize(w, h);
  tile.bokehPass.setSize(w, h);
  // UnrealBloomPass nemá veřejné setSize, ale composer.setSize na něj propaguje.
}

// ----- Round controller ------------------------------------------------------

interface RoundLayout {
  readonly cols: number;
  readonly rows: number;
  readonly tileWidth: number;
  readonly tileHeight: number;
  readonly canvasWidth: number;
  readonly canvasHeight: number;
}

function computeLayout(canvas: HTMLCanvasElement, cols: number, rows: number): RoundLayout {
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  return {
    cols, rows,
    canvasWidth: w,
    canvasHeight: h,
    tileWidth: Math.floor(w / cols),
    tileHeight: Math.floor(h / rows),
  };
}

function tileValues(def: ParamDef, n: number): number[] {
  const out: number[] = [];
  for (let i = 0; i < n; i += 1) {
    const t = n === 1 ? 0.5 : i / (n - 1);
    const raw = def.min + (def.max - def.min) * t;
    const stepped = Math.round(raw / def.step) * def.step;
    const clamped = Math.max(def.min, Math.min(def.max, stepped));
    // Zaokrouhli na precision podle stepu — vyhne se FP špíně typu 0.35000000000000003
    // v lockednutém JSONu (rendering by si nevšiml, ale výstupní hodnoty jsou jinak ošklivé).
    out.push(Number(clamped.toFixed(def.precision)));
  }
  return out;
}

function formatValue(v: number, precision: number): string {
  return v.toFixed(precision);
}

// ----- Bootstrap -------------------------------------------------------------

async function bootstrap(): Promise<void> {
  // 1) Snapshot — try static, fall back to capture.
  const snapshot: SnapshotData = (await tryLoadStaticSnapshot()) ?? (await captureSnapshot());

  setStatus('Inicializuji renderer…');

  // 2) Renderer + canvas.
  const canvas = document.getElementById('canvas') as HTMLCanvasElement;
  const overlay = document.getElementById('overlay') as HTMLDivElement;
  const topbar = document.getElementById('topbar') as HTMLElement;
  const gridEl = document.getElementById('grid') as HTMLElement;
  const summary = document.getElementById('summary') as HTMLElement;
  topbar.hidden = false;
  gridEl.hidden = false;

  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;
  renderer.autoClear = false;
  renderer.setScissorTest(true);

  function syncCanvasSize(): void {
    const rect = canvas.getBoundingClientRect();
    renderer.setSize(rect.width, rect.height, false);
  }
  syncCanvasSize();

  // 3) Shared camera + OrbitControls (jeden pohled pro všechny tiles).
  const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 5000);
  const fit = fitCamera(snapshot.bbox);
  camera.position.set(fit.eye[0], fit.eye[1], fit.eye[2]);
  const controls = new OrbitControls(camera, canvas);
  controls.target.set(fit.target[0], fit.target[1], fit.target[2]);
  controls.enableDamping = true;
  controls.dampingFactor = 0.06;
  controls.update();

  const env = buildSpaceEnvironment(renderer);

  // 4) Vytvoříme TILES_PER_ROUND tile bundles; resize se aktualizuje při změně round layoutu.
  let layout = computeLayout(canvas, GRID_COLS, GRID_ROWS);
  const initialParams = defaultParams();
  const tiles: Tile[] = [];
  for (let i = 0; i < TILES_PER_ROUND; i += 1) {
    tiles.push(createTile(renderer, camera, env, snapshot, layout.tileWidth, layout.tileHeight, initialParams));
  }
  // Po prvním render-cyklu se onBeforeCompile zavolá a materialShader se zachytí; do té doby emissive zůstává defaultní.

  // 5) Round controller state.
  type Mode = 'round' | 'final' | 'compare';
  let mode: Mode = 'round';
  let roundIdx = 0;
  let activeTileCount = TILES_PER_ROUND;
  let compareCandidates: RenderParams[] = [];
  let history = loadHistory();
  const locked: Partial<RenderParams> = {};

  function currentParamsFor(value: number): RenderParams {
    const base = defaultParams();
    Object.assign(base, locked);
    if (roundIdx < PARAM_DEFS.length) {
      const name = PARAM_DEFS[roundIdx]!.name;
      base[name] = value;
    }
    return base;
  }

  function compareLayoutFor(n: number): { cols: number; rows: number } {
    if (n <= 1) return { cols: 1, rows: 1 };
    if (n <= 5) return { cols: n, rows: 1 };
    return { cols: Math.ceil(n / 2), rows: 2 };
  }

  function rebuildOverlay(
    n: number,
    labelHtmlFor: (i: number) => string,
    onClickAt: (i: number) => void,
  ): void {
    overlay.innerHTML = '';
    for (let i = 0; i < n; i += 1) {
      const col = i % layout.cols;
      const row = Math.floor(i / layout.cols);
      const tile = document.createElement('div');
      tile.className = 'tile';
      tile.style.left = `${col * layout.tileWidth}px`;
      tile.style.top = `${row * layout.tileHeight}px`;
      tile.style.width = `${layout.tileWidth}px`;
      tile.style.height = `${layout.tileHeight}px`;
      const label = document.createElement('div');
      label.className = 'label';
      label.innerHTML = labelHtmlFor(i);
      tile.appendChild(label);
      tile.addEventListener('click', () => onClickAt(i));
      overlay.appendChild(tile);
    }
  }

  function syncCameraAspect(): void {
    camera.aspect = layout.tileWidth / Math.max(1, layout.tileHeight);
    camera.updateProjectionMatrix();
  }

  function renderLockedBreadcrumb(): void {
    const panel = document.getElementById('lockedPanel') as HTMLDivElement;
    panel.innerHTML = '';
    for (const def of PARAM_DEFS) {
      const v = locked[def.name];
      if (v === undefined) continue;
      const item = document.createElement('span');
      item.className = 'item';
      item.innerHTML = `<strong>${def.name}</strong> ${formatValue(v, def.precision)}`;
      panel.appendChild(item);
    }
  }

  function enterRound(idx: number): void {
    mode = 'round';
    roundIdx = idx;
    activeTileCount = TILES_PER_ROUND;
    if (idx >= PARAM_DEFS.length) {
      enterFinal();
      return;
    }
    const def = PARAM_DEFS[idx]!;
    (document.getElementById('roundIdx') as HTMLSpanElement).textContent = String(idx + 1);
    (document.getElementById('paramName') as HTMLElement).textContent = def.label;
    renderLockedBreadcrumb();

    layout = computeLayout(canvas, GRID_COLS, GRID_ROWS);
    for (const t of tiles) resizeTile(t, layout.tileWidth, layout.tileHeight);
    syncCameraAspect();

    const values = tileValues(def, TILES_PER_ROUND);
    rebuildOverlay(
      TILES_PER_ROUND,
      (i) => `${def.label}: <span class="v">${formatValue(values[i]!, def.precision)}</span>`,
      (i) => onPickRound(values[i]!),
    );
    for (let i = 0; i < TILES_PER_ROUND; i += 1) {
      applyTileParams(tiles[i]!, currentParamsFor(values[i]!), snapshot);
    }
    summary.hidden = true;
  }

  function enterFinal(): void {
    mode = 'final';
    activeTileCount = 1;
    // Po posledním kole zobrazíme jen 1 tile (1x1 layout), na něm finální kombinace.
    (document.getElementById('paramName') as HTMLElement).textContent = 'final preview';
    (document.getElementById('roundIdx') as HTMLSpanElement).textContent = String(PARAM_DEFS.length);
    renderLockedBreadcrumb();

    layout = computeLayout(canvas, 1, 1);
    resizeTile(tiles[0]!, layout.tileWidth, layout.tileHeight);
    syncCameraAspect();

    const finalParams: RenderParams = { ...defaultParams(), ...locked };
    applyTileParams(tiles[0]!, finalParams, snapshot);
    overlay.innerHTML = '';

    history = appendHistory(history, finalParams);

    const json = JSON.stringify(finalParams, null, 2);
    (document.getElementById('summaryJson') as HTMLPreElement).textContent = json;
    renderHistoryPanel();
    summary.hidden = false;
  }

  function enterCompare(): void {
    if (history.length < 2) return;
    mode = 'compare';
    compareCandidates = history.slice(-HISTORY_MAX);
    activeTileCount = compareCandidates.length;
    const { cols, rows } = compareLayoutFor(activeTileCount);
    layout = computeLayout(canvas, cols, rows);

    (document.getElementById('paramName') as HTMLElement).textContent =
      `compare (${activeTileCount} kandidátů)`;
    (document.getElementById('roundIdx') as HTMLSpanElement).textContent = '★';
    renderLockedBreadcrumb();

    syncCameraAspect();
    for (let i = 0; i < activeTileCount; i += 1) {
      resizeTile(tiles[i]!, layout.tileWidth, layout.tileHeight);
      applyTileParams(tiles[i]!, compareCandidates[i]!, snapshot);
    }
    rebuildOverlay(
      activeTileCount,
      (i) => {
        const c = compareCandidates[i]!;
        return `<span class="v">run ${i + 1}</span>`
          + `<br>exp:${formatValue(c.exposure, 2)} `
          + `bloom:${formatValue(c.bloomStrength, 2)} `
          + `vox:${formatValue(c.voxelSize, 2)} `
          + `lum:${formatValue(c.minLuma, 2)}`;
      },
      (i) => onPickCompare(i),
    );
    summary.hidden = true;
  }

  function enterRefine(): void {
    // Druhý průchod: locked zůstává, jen restartujeme od kola 0.
    // currentParamsFor() přepíše hodnotu aktuálního kola, ostatní vychází z locked.
    summary.hidden = true;
    enterRound(0);
  }

  function onPickRound(value: number): void {
    if (mode !== 'round') return;
    if (roundIdx >= PARAM_DEFS.length) return;
    const def = PARAM_DEFS[roundIdx]!;
    locked[def.name] = value;
    enterRound(roundIdx + 1);
  }

  function onPickCompare(idx: number): void {
    if (mode !== 'compare') return;
    const winner = compareCandidates[idx];
    if (!winner) return;
    for (const key of Object.keys(locked)) delete (locked as Record<string, number>)[key];
    Object.assign(locked, winner);
    enterFinal();
  }

  function renderHistoryPanel(): void {
    const list = document.getElementById('historyList') as HTMLDivElement;
    list.innerHTML = '';
    if (history.length === 0) {
      list.textContent = 'Žádný předchozí run.';
    } else {
      for (let i = 0; i < history.length; i += 1) {
        const h = history[i]!;
        const item = document.createElement('div');
        item.className = 'history-item';
        item.innerHTML = `<strong>run ${i + 1}</strong>`
          + ` <span class="snip">exp:${formatValue(h.exposure, 2)} `
          + `emis:${formatValue(h.emissive, 2)} `
          + `bloom:${formatValue(h.bloomStrength, 2)} `
          + `vox:${formatValue(h.voxelSize, 2)} `
          + `lum:${formatValue(h.minLuma, 2)}</span>`;
        list.appendChild(item);
      }
    }
    const compareBtn = document.getElementById('compareBtn') as HTMLButtonElement;
    compareBtn.disabled = history.length < 2;

    const medianPanel = document.getElementById('medianPanel') as HTMLElement;
    const medianJson = document.getElementById('medianJson') as HTMLPreElement;
    if (history.length >= 2) {
      medianPanel.hidden = false;
      medianJson.textContent = JSON.stringify(medianParams(history), null, 2);
    } else {
      medianPanel.hidden = true;
    }
  }

  // 6) Render smyčka.
  function renderAll(): void {
    if (mode === 'final') {
      const tile = tiles[0]!;
      renderer.toneMappingExposure = tile.params.exposure;
      renderer.setViewport(0, 0, layout.canvasWidth, layout.canvasHeight);
      renderer.setScissor(0, 0, layout.canvasWidth, layout.canvasHeight);
      tile.composer.render();
      return;
    }
    // round nebo compare: grid podle aktuálního layoutu a počtu aktivních tiles.
    for (let i = 0; i < activeTileCount; i += 1) {
      const col = i % layout.cols;
      const row = Math.floor(i / layout.cols);
      const x = col * layout.tileWidth;
      // WebGL viewport má y od spodu; DOM má y od shora.
      const yWebgl = layout.canvasHeight - (row + 1) * layout.tileHeight;
      renderer.setViewport(x, yWebgl, layout.tileWidth, layout.tileHeight);
      renderer.setScissor(x, yWebgl, layout.tileWidth, layout.tileHeight);
      renderer.toneMappingExposure = tiles[i]!.params.exposure;
      tiles[i]!.composer.render();
    }
  }

  function frame(): void {
    controls.update();
    renderAll();
    requestAnimationFrame(frame);
  }

  // 7) Resize handler — recompute layout on window resize.
  window.addEventListener('resize', () => {
    syncCanvasSize();
    if (mode === 'final') {
      layout = computeLayout(canvas, 1, 1);
      resizeTile(tiles[0]!, layout.tileWidth, layout.tileHeight);
    } else if (mode === 'compare') {
      const { cols, rows } = compareLayoutFor(activeTileCount);
      layout = computeLayout(canvas, cols, rows);
      for (let i = 0; i < activeTileCount; i += 1) {
        resizeTile(tiles[i]!, layout.tileWidth, layout.tileHeight);
      }
    } else {
      layout = computeLayout(canvas, GRID_COLS, GRID_ROWS);
      for (const t of tiles) resizeTile(t, layout.tileWidth, layout.tileHeight);
    }
    // Re-position overlay tiles to match new layout.
    const cells = overlay.querySelectorAll<HTMLDivElement>('.tile');
    for (let i = 0; i < cells.length; i += 1) {
      const col = i % layout.cols;
      const row = Math.floor(i / layout.cols);
      cells[i]!.style.left = `${col * layout.tileWidth}px`;
      cells[i]!.style.top = `${row * layout.tileHeight}px`;
      cells[i]!.style.width = `${layout.tileWidth}px`;
      cells[i]!.style.height = `${layout.tileHeight}px`;
    }
    syncCameraAspect();
  });

  // 8) Buttons.
  (document.getElementById('saveSnapshotBtn') as HTMLButtonElement)
    .addEventListener('click', () => downloadSnapshot(snapshot));
  const restart = (): void => {
    for (const key of Object.keys(locked)) delete (locked as Record<string, number>)[key];
    enterRound(0);
  };
  (document.getElementById('restartBtn') as HTMLButtonElement).addEventListener('click', restart);
  (document.getElementById('restartFinalBtn') as HTMLButtonElement).addEventListener('click', restart);
  (document.getElementById('refineBtn') as HTMLButtonElement).addEventListener('click', enterRefine);
  (document.getElementById('compareBtn') as HTMLButtonElement).addEventListener('click', enterCompare);
  (document.getElementById('copyBtn') as HTMLButtonElement).addEventListener('click', () => {
    const txt = (document.getElementById('summaryJson') as HTMLPreElement).textContent ?? '';
    void navigator.clipboard.writeText(txt);
  });
  (document.getElementById('copyMedianBtn') as HTMLButtonElement).addEventListener('click', () => {
    const txt = (document.getElementById('medianJson') as HTMLPreElement).textContent ?? '';
    void navigator.clipboard.writeText(txt);
  });
  (document.getElementById('clearHistoryBtn') as HTMLButtonElement).addEventListener('click', () => {
    history = [];
    saveHistory(history);
    renderHistoryPanel();
  });

  // 9) Start — enterRound() interně volá syncCameraAspect().
  enterRound(0);
  hideStatus();
  requestAnimationFrame(frame);
}

void bootstrap().catch((err) => {
  console.error(err);
  setStatus(`Chyba: ${err instanceof Error ? err.message : String(err)}`);
});

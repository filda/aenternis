// Aenternis — WASM 3D viewer (phase 4c).
//
// Render thread only. The WASM World instance and the per-tick step
// loop live in a dedicated Web Worker (`./worker.ts`); the main thread
// receives `{ snap, stride, tick, ... }` snapshots over postMessage
// (Uint32Array transferred zero-copy) and renders the latest one each
// animation frame.
//
// This decoupling keeps the render thread at 60 FPS even when the
// simulation is heavy. Render and sim are independent: if the worker
// is slow, the renderer reuses the last snapshot; if the renderer is
// slow, intermediate snapshots are simply overwritten before they're
// drawn.
//
// Pure logic (heat ramp, formatters, snapshot analysis, tracker
// reducer, camera-fit math, program-text parser) lives in `src/` and
// is exercised by the full vitest+stryker gate. This file is the
// glue that wires that logic into THREE / DOM / Worker. The `bootstrap`
// export is invoked from `index.html` so importing this module in a
// test does not actually start anything.

import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { SSAOPass } from 'three/addons/postprocessing/SSAOPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';

import { fitCamera } from '../src/camera-fit.ts';
import { fmtBbox, fmtDirArr, fmtMemoryHexDump } from '../src/format.ts';
import { heatColor, meanRelativeT, voxelSizeFactor } from '../src/heat.ts';
import { JITTER_AMPLITUDE, gridJitter } from '../src/jitter.ts';
import { parseProgramText } from '../src/program-text.ts';
import type {
  CellDetailMsg,
  ConfigMsg,
  InitMsg,
  InspectMsg,
  RunningMsg,
  SnapshotMsg,
  WorkerToMainMsg,
} from '../src/protocol.ts';
import { analyzeSnapshot, type SnapshotBbox } from '../src/snapshot.ts';
import {
  EMPTY_TRACKER_STATE,
  pushTrackerSample,
  resetTrackerState,
  type TrackerState,
} from '../src/tracker.ts';

interface RuntimeConfig {
  seed: number;
  energy: number;
  coeff: number;
  k: number;
  moveThreshold: number;
  rngKind: 'pcg' | 'xorshift32';
  legacyTickOffset: boolean;
  legacyFullPrecision: boolean;
  legacyPortWrap: boolean;
  legacyOpcodeSet: boolean;
}

/** Look up an element by id and assert its concrete type, throwing a
 *  loud error early if the HTML and the code drifted apart. */
function requireEl<T extends Element>(id: string, ctor: { new (...args: never[]): T }): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} not found in DOM`);
  if (!(el instanceof ctor)) {
    throw new Error(`#${id} is not a ${ctor.name}`);
  }
  return el;
}

/** Entry point — invoked from `index.html` after the DOM is parsed.
 *  Wires the THREE scene, DOM listeners, and the simulation worker
 *  into one coherent runtime. Does not return; the frame loop keeps
 *  running on `requestAnimationFrame`. */
export function bootstrap(): void {
  // ----- Configuration -------------------------------------------------------
  const config: RuntimeConfig = {
    seed: 1234,
    energy: 10_000_000,
    coeff: 0.15,
    k: 1,
    moveThreshold: 1.0,
    rngKind: 'pcg',
    legacyTickOffset: false,
    legacyFullPrecision: false,
    legacyPortWrap: false,
    legacyOpcodeSet: false,
  };

  // ----- DOM lookup ----------------------------------------------------------
  // Resolved once; if any id is missing we want to see the error before
  // wiring up listeners that would silently fail.
  const dom = {
    container: requireEl('canvasContainer', HTMLDivElement),
    tick: requireEl('tick', HTMLSpanElement),
    cells: requireEl('cells', HTMLSpanElement),
    energy: requireEl('energy', HTMLSpanElement),
    bbox: requireEl('bbox', HTMLSpanElement),
    fps: requireEl('fps', HTMLSpanElement),
    msPerTick: requireEl('msPerTick', HTMLSpanElement),
    ticksPerSec: requireEl('ticksPerSec', HTMLSpanElement),
    pauseBtn: requireEl('pauseBtn', HTMLButtonElement),
    resetBtn: requireEl('resetBtn', HTMLButtonElement),
    seed: requireEl('seed', HTMLInputElement),
    energyIn: requireEl('energy_in', HTMLInputElement),
    coeff: requireEl('coeff', HTMLInputElement),
    coeffVal: requireEl('coeffVal', HTMLSpanElement),
    k: requireEl('k', HTMLInputElement),
    kVal: requireEl('kVal', HTMLSpanElement),
    moveThreshold: requireEl('moveThreshold', HTMLInputElement),
    moveThresholdVal: requireEl('moveThresholdVal', HTMLSpanElement),
    trackerEnabled: requireEl('trackerEnabled', HTMLInputElement),
    trailLen: requireEl('trailLen', HTMLInputElement),
    trailLenVal: requireEl('trailLenVal', HTMLSpanElement),
    trackerPos: requireEl('trackerPos', HTMLSpanElement),
    programText: requireEl('programText', HTMLTextAreaElement),
    programStatus: requireEl('programStatus', HTMLDivElement),
    sliceEnabled: requireEl('sliceEnabled', HTMLInputElement),
    voxelSize: requireEl('voxelSize', HTMLInputElement),
    voxelSizeVal: requireEl('voxelSizeVal', HTMLSpanElement),
    shapeOcta: requireEl('shapeOcta', HTMLInputElement),
    bloomEnabled: requireEl('bloomEnabled', HTMLInputElement),
    bloomThreshold: requireEl('bloomThreshold', HTMLInputElement),
    bloomThresholdVal: requireEl('bloomThresholdVal', HTMLSpanElement),
    bloomStrength: requireEl('bloomStrength', HTMLInputElement),
    bloomStrengthVal: requireEl('bloomStrengthVal', HTMLSpanElement),
    bloomRadius: requireEl('bloomRadius', HTMLInputElement),
    bloomRadiusVal: requireEl('bloomRadiusVal', HTMLSpanElement),
    exposure: requireEl('exposure', HTMLInputElement),
    exposureVal: requireEl('exposureVal', HTMLSpanElement),
    fogEnabled: requireEl('fogEnabled', HTMLInputElement),
    fogDensity: requireEl('fogDensity', HTMLInputElement),
    fogDensityVal: requireEl('fogDensityVal', HTMLSpanElement),
    emissive: requireEl('emissive', HTMLInputElement),
    emissiveVal: requireEl('emissiveVal', HTMLSpanElement),
    roughness: requireEl('roughness', HTMLInputElement),
    roughnessVal: requireEl('roughnessVal', HTMLSpanElement),
    ssaoEnabled: requireEl('ssaoEnabled', HTMLInputElement),
    ssaoRadius: requireEl('ssaoRadius', HTMLInputElement),
    ssaoRadiusVal: requireEl('ssaoRadiusVal', HTMLSpanElement),
    envEnabled: requireEl('envEnabled', HTMLInputElement),
    envBackground: requireEl('envBackground', HTMLInputElement),
    rngXs32: requireEl('rngXs32', HTMLInputElement),
    legacyTickOffset: requireEl('legacyTickOffset', HTMLInputElement),
    legacyFullPrecision: requireEl('legacyFullPrecision', HTMLInputElement),
    legacyPortWrap: requireEl('legacyPortWrap', HTMLInputElement),
    legacyOpcodeSet: requireEl('legacyOpcodeSet', HTMLInputElement),
    inspector: requireEl('inspector', HTMLElement),
  };

  // ----- Worker setup --------------------------------------------------------
  // `./worker.js` resolves to the tsc-emitted file in production and to
  // `./worker.ts` source in Vite dev (Vite rewrites `.js` → `.ts`).
  const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
  let latestSnapshot: SnapshotMsg | null = null;
  let workerReady = false;

  worker.onmessage = (ev: MessageEvent<WorkerToMainMsg>) => {
    const msg = ev.data;
    if (msg.type === 'ready') {
      workerReady = true;
      sendInit();
    } else if (msg.type === 'snapshot') {
      latestSnapshot = msg;
    } else if (msg.type === 'cellDetail') {
      renderInspector(msg);
    }
  };

  function sendInit(): void {
    const { program, status } = parseProgramText(dom.programText.value);
    dom.programStatus.textContent = status;
    const init: InitMsg = {
      type: 'init',
      seed: config.seed,
      energy: config.energy,
      coeff: config.coeff,
      k: config.k,
      moveThreshold: config.moveThreshold,
      rngKind: config.rngKind,
      legacyTickOffset: config.legacyTickOffset,
      legacyFullPrecision: config.legacyFullPrecision,
      legacyPortWrap: config.legacyPortWrap,
      legacyOpcodeSet: config.legacyOpcodeSet,
      program,
    };
    worker.postMessage(init);
    cameraFitDirty = true;
    trackerState = resetTrackerState();
  }

  function sendConfig(): void {
    if (!workerReady) return;
    const cfg: ConfigMsg = {
      type: 'config',
      coeff: config.coeff,
      k: config.k,
      moveThreshold: config.moveThreshold,
      legacyTickOffset: config.legacyTickOffset,
      legacyFullPrecision: config.legacyFullPrecision,
      legacyPortWrap: config.legacyPortWrap,
      legacyOpcodeSet: config.legacyOpcodeSet,
    };
    worker.postMessage(cfg);
  }

  function sendRunning(running: boolean): void {
    if (!workerReady) return;
    const msg: RunningMsg = { type: 'running', running };
    worker.postMessage(msg);
  }

  function requestInspect(x: number, y: number, z: number): void {
    if (!workerReady) return;
    const msg: InspectMsg = { type: 'inspect', x, y, z };
    worker.postMessage(msg);
  }

  // ----- Three.js setup ------------------------------------------------------
  const scene = new THREE.Scene();
  const defaultBackground = new THREE.Color(0x05050a);
  scene.background = defaultBackground;
  // Exponential depth fog matching the background color — distant
  // cells fade into the dark, which dramatically improves 3D depth
  // perception in dense fields. Density and on/off live behind the
  // sliders below; we hold the same FogExp2 instance and just toggle
  // `scene.fog` between it and null.
  const fog = new THREE.FogExp2(0x05050a, parseFloat(dom.fogDensity.value));
  scene.fog = dom.fogEnabled.checked ? fog : null;

  const camera = new THREE.PerspectiveCamera(60, window.innerWidth / window.innerHeight, 0.1, 5000);
  camera.position.set(20, 20, 30);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(window.innerWidth, window.innerHeight);
  // ACES filmic tone mapping for the postprocessing pipeline. Applied
  // by `OutputPass` at the very end of the composer chain so bloom
  // operates in linear HDR space and only the final composite gets
  // tone-mapped to sRGB. `toneMappingExposure` is a multiplier on
  // pre-tonemap luminance and is exposed as a slider.
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = parseFloat(dom.exposure.value);
  dom.container.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.05;

  // Postprocessing pipeline: scene render → unreal bloom. Bloom picks
  // up pixels brighter than `threshold` and bleeds them into a glow,
  // which combined with the warm-end heat ramp makes hot cells feel
  // like they're radiating energy. Strength / radius / threshold are
  // exposed via sliders; the whole pass can be disabled to compare.
  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(scene, camera));
  // Screen-space ambient occlusion. Adds contact shadows between
  // densely packed cells so the field stops looking like a flat
  // particle cloud and gains real 3D structure. Costs an extra
  // depth/normal pass — toggled off the cheap way.
  const ssaoPass = new SSAOPass(scene, camera, window.innerWidth, window.innerHeight);
  ssaoPass.kernelRadius = parseFloat(dom.ssaoRadius.value);
  ssaoPass.enabled = dom.ssaoEnabled.checked;
  composer.addPass(ssaoPass);
  const bloomPass = new UnrealBloomPass(
    new THREE.Vector2(window.innerWidth, window.innerHeight),
    parseFloat(dom.bloomStrength.value),
    parseFloat(dom.bloomRadius.value),
    parseFloat(dom.bloomThreshold.value),
  );
  composer.addPass(bloomPass);
  // Final pass: tone-map the linear HDR composite to sRGB. Without
  // this, bloom highlights would be hard-clipped and the ACES setting
  // on the renderer wouldn't be applied to the post-processed output.
  composer.addPass(new OutputPass());

  window.addEventListener('resize', () => {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
    composer.setSize(window.innerWidth, window.innerHeight);
    ssaoPass.setSize(window.innerWidth, window.innerHeight);
  });

  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
  dirLight.position.set(50, 80, 50);
  scene.add(dirLight);

  // ----- Procedural deep-space environment ----------------------------------
  // Builds a small skybox scene (gradient sphere + ~800 stars) and
  // bakes it once via PMREMGenerator into a pre-filtered cubemap.
  // The cubemap is what Three uses for PBR indirect-light reflections
  // — without it, MeshStandardMaterial only sees the directional +
  // ambient lights and looks chalky. With it, voxels pick up subtle
  // blue ambient from the "sky" and faint star highlights, all
  // procedural so no asset shipping.
  function buildSpaceEnvironment(): THREE.Texture {
    const skyScene = new THREE.Scene();

    // Gradient sphere rendered from the inside.
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

    // Star field — uniform random points on a sphere, slight color
    // variation so it doesn't read as a regular grid.
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
      // Blue-white tint, occasional warm star.
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

    const pmremGenerator = new THREE.PMREMGenerator(renderer);
    pmremGenerator.compileEquirectangularShader();
    const envRT = pmremGenerator.fromScene(skyScene);
    pmremGenerator.dispose();
    // Dispose source meshes — we only need the baked cubemap now.
    skyGeom.dispose();
    skyMat.dispose();
    starGeom.dispose();
    starMat.dispose();
    return envRT.texture;
  }

  const spaceEnv = buildSpaceEnvironment();
  scene.environment = dom.envEnabled.checked ? spaceEnv : null;
  if (dom.envBackground.checked) scene.background = spaceEnv;

  // ----- WSAD camera ---------------------------------------------------------
  const keyState: Record<string, boolean> = {
    w: false, a: false, s: false, d: false, q: false, e: false, shift: false,
  };

  function isInputFocused(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    const tag = target.tagName;
    return tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA';
  }

  window.addEventListener('keydown', (ev) => {
    if (isInputFocused(ev.target)) return;
    const k = ev.key.toLowerCase();
    if (k in keyState) keyState[k] = true;
    if (k === 'shift') keyState['shift'] = true;
  });
  window.addEventListener('keyup', (ev) => {
    const k = ev.key.toLowerCase();
    if (k in keyState) keyState[k] = false;
    if (k === 'shift') keyState['shift'] = false;
  });
  window.addEventListener('blur', () => {
    for (const k of Object.keys(keyState)) keyState[k] = false;
  });

  const _camForward = new THREE.Vector3();
  const _camRight = new THREE.Vector3();
  const _worldUp = new THREE.Vector3(0, 1, 0);
  const _moveDelta = new THREE.Vector3();

  function applyWsad(dtMs: number): void {
    if (!keyState['w'] && !keyState['a'] && !keyState['s'] && !keyState['d']
        && !keyState['q'] && !keyState['e']) return;

    const dist = camera.position.distanceTo(controls.target);
    const baseSpeed = Math.max(dist * 0.01, 0.1) * (dtMs / 16);
    const speed = baseSpeed * (keyState['shift'] ? 3 : 1);

    camera.getWorldDirection(_camForward);
    _camRight.crossVectors(_camForward, _worldUp).normalize();

    _moveDelta.set(0, 0, 0);
    if (keyState['w']) _moveDelta.addScaledVector(_camForward, speed);
    if (keyState['s']) _moveDelta.addScaledVector(_camForward, -speed);
    if (keyState['d']) _moveDelta.addScaledVector(_camRight, speed);
    if (keyState['a']) _moveDelta.addScaledVector(_camRight, -speed);
    if (keyState['e']) _moveDelta.y += speed;
    if (keyState['q']) _moveDelta.y -= speed;

    camera.position.add(_moveDelta);
    controls.target.add(_moveDelta);
  }

  // ----- Voxel mesh (instanced, dynamic capacity) ----------------------------
  // Two shapes available: the default 8×6 sphere (~80 triangles) and a
  // detail-0 octahedron (8 triangles). The octahedron renders ~10×
  // faster per cell — useful when the field grows past a few hundred
  // thousand cells. Visually a small octahedron with jitter applied
  // reads as a slightly faceted ball.
  const VOXEL_SPHERE_RADIUS = 0.45;
  const VOXEL_OCTA_RADIUS = 0.55;
  function makeVoxelGeometry(octahedron: boolean): THREE.BufferGeometry {
    return octahedron
      ? new THREE.OctahedronGeometry(VOXEL_OCTA_RADIUS, 0)
      : new THREE.SphereGeometry(VOXEL_SPHERE_RADIUS, 8, 6);
  }
  let useOctahedron = false;
  let voxelGeometry: THREE.BufferGeometry = makeVoxelGeometry(useOctahedron);
  // PBR material — metalness 0 (no specular highlights from a metallic
  // surface) and a soft roughness keep the diffuse shading subtle so
  // the heat-ramp color stays the dominant visual signal. Per-cell
  // emissive contribution is injected via onBeforeCompile below: the
  // surface color is added to the emissive radiance, so a hot (white)
  // cell radiates strongly while a cold (near-black) cell barely glows.
  const voxelMaterial = new THREE.MeshStandardMaterial({
    metalness: 0.0,
    roughness: parseFloat(dom.roughness.value),
  });
  // Captured shader handle so the emissive slider can poke at the
  // uniform after the material has been compiled. Three.js compiles
  // lazily on first render, so we read it inside `onBeforeCompile`.
  let voxelMaterialShader: THREE.WebGLProgramParametersWithUniforms | null = null;
  voxelMaterial.onBeforeCompile = (shader) => {
    shader.uniforms['uEmissiveBoost'] = { value: parseFloat(dom.emissive.value) };
    shader.fragmentShader = shader.fragmentShader
      .replace(
        'uniform vec3 emissive;',
        `uniform vec3 emissive;
uniform float uEmissiveBoost;`,
      )
      .replace(
        '#include <emissivemap_fragment>',
        `#include <emissivemap_fragment>
totalEmissiveRadiance += diffuseColor.rgb * uEmissiveBoost;`,
      );
    voxelMaterialShader = shader;
  };
  let voxelMesh: THREE.InstancedMesh | null = null;
  let voxelCapacity = 0;
  let lastUsedCount = 0;

  function rebuildVoxelMesh(cap: number): void {
    if (voxelMesh) {
      scene.remove(voxelMesh);
      voxelMesh.dispose();
    }
    voxelMesh = new THREE.InstancedMesh(voxelGeometry, voxelMaterial, cap);
    voxelMesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    const initColor = new THREE.Color(0, 0, 0);
    for (let i = 0; i < cap; i += 1) {
      voxelMesh.setColorAt(i, initColor);
    }
    scene.add(voxelMesh);
    voxelCapacity = cap;
    lastUsedCount = 0;
  }

  function ensureCapacity(n: number): void {
    if (voxelCapacity >= n) return;
    let cap = voxelCapacity > 0 ? voxelCapacity : 256;
    while (cap < n) cap *= 2;
    rebuildVoxelMesh(cap);
  }

  function setVoxelShape(octahedron: boolean): void {
    if (useOctahedron === octahedron) return;
    useOctahedron = octahedron;
    voxelGeometry.dispose();
    voxelGeometry = makeVoxelGeometry(useOctahedron);
    rebuildVoxelMesh(Math.max(voxelCapacity, 256));
    lastRenderedTick = -1;
  }
  ensureCapacity(1024);

  // ----- Tracker meshes ------------------------------------------------------
  let trackerState: TrackerState = EMPTY_TRACKER_STATE;
  let trackerEnabled = false;
  let trailLen = 60;
  let highlightMesh: THREE.LineSegments | null = null;
  let trailLine: THREE.Line | null = null;

  function createHighlightMesh(): void {
    if (highlightMesh) {
      scene.remove(highlightMesh);
      highlightMesh.geometry.dispose();
      (highlightMesh.material as THREE.Material).dispose();
    }
    const geo = new THREE.BoxGeometry(1.4, 1.4, 1.4);
    const mat = new THREE.LineBasicMaterial({ color: 0xfff0c0, transparent: true, opacity: 0.95 });
    const wire = new THREE.LineSegments(new THREE.EdgesGeometry(geo), mat);
    wire.visible = false;
    scene.add(wire);
    highlightMesh = wire;
  }

  function createTrailLine(): void {
    if (trailLine) {
      scene.remove(trailLine);
      trailLine.geometry.dispose();
      (trailLine.material as THREE.Material).dispose();
    }
    const cap = Math.max(2, trailLen + 1);
    const geo = new THREE.BufferGeometry();
    const positions = new Float32Array(cap * 3);
    const colors = new Float32Array(cap * 3);
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    geo.setDrawRange(0, 0);
    const mat = new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.85 });
    const line = new THREE.Line(geo, mat);
    line.frustumCulled = false;
    scene.add(line);
    trailLine = line;
  }
  createHighlightMesh();
  createTrailLine();

  function updateTrackerVisuals(maxCellIdx: number, snap: Uint32Array, stride: number): void {
    if (!trackerEnabled || maxCellIdx < 0) {
      if (highlightMesh) highlightMesh.visible = false;
      if (trailLine) trailLine.visible = false;
      dom.trackerPos.textContent = '-';
      return;
    }

    const off = maxCellIdx * stride;
    const x = snap[off]! | 0;
    const y = snap[off + 1]! | 0;
    const z = snap[off + 2]! | 0;
    const e = snap[off + 3]!;
    trackerState = pushTrackerSample(trackerState, { x, y, z, energy: e }, trailLen);

    if (highlightMesh) {
      highlightMesh.visible = true;
      // Match the same jitter applied to the voxel mesh so the
      // wireframe stays glued to its cell instead of floating to the
      // grid center.
      highlightMesh.position.set(
        x + gridJitter(x, y, z, 0) * JITTER_AMPLITUDE,
        y + gridJitter(x, y, z, 1) * JITTER_AMPLITUDE,
        z + gridJitter(x, y, z, 2) * JITTER_AMPLITUDE,
      );
      const pulse = 1.0 + 0.08 * Math.sin(performance.now() / 250);
      highlightMesh.scale.setScalar(pulse);
    }

    if (trailLine) {
      const len = trackerState.trail.length;
      trailLine.visible = (trailLen > 0) && (len > 1);
      if (trailLine.visible) {
        const positions = trailLine.geometry.attributes['position']!.array as Float32Array;
        const colors = trailLine.geometry.attributes['color']!.array as Float32Array;
        const cap = positions.length / 3;
        const drawLen = Math.min(len, cap);
        for (let i = 0; i < drawLen; i += 1) {
          const p = trackerState.trail[len - drawLen + i]!;
          positions[i * 3] = p.x;
          positions[i * 3 + 1] = p.y;
          positions[i * 3 + 2] = p.z;
          const f = (i + 1) / drawLen;
          colors[i * 3] = 1.00 * f;
          colors[i * 3 + 1] = 0.80 * f;
          colors[i * 3 + 2] = 0.20 * f;
        }
        trailLine.geometry.setDrawRange(0, drawLen);
        trailLine.geometry.attributes['position']!.needsUpdate = true;
        trailLine.geometry.attributes['color']!.needsUpdate = true;
      }
    }
    dom.trackerPos.textContent = `(${x},${y},${z}) E=${e.toLocaleString()}`;
  }

  // ----- Render settings -----------------------------------------------------
  let voxelScale = 1.0;
  dom.voxelSize.addEventListener('input', () => {
    voxelScale = parseFloat(dom.voxelSize.value);
    dom.voxelSizeVal.textContent = voxelScale.toFixed(2);
    lastRenderedTick = -1; // force a re-render even if no new snapshot.
  });
  dom.shapeOcta.addEventListener('change', () => {
    setVoxelShape(dom.shapeOcta.checked);
  });
  dom.bloomEnabled.addEventListener('change', () => {
    bloomPass.enabled = dom.bloomEnabled.checked;
  });
  dom.bloomThreshold.addEventListener('input', () => {
    const v = parseFloat(dom.bloomThreshold.value);
    bloomPass.threshold = v;
    dom.bloomThresholdVal.textContent = v.toFixed(2);
  });
  dom.bloomStrength.addEventListener('input', () => {
    const v = parseFloat(dom.bloomStrength.value);
    bloomPass.strength = v;
    dom.bloomStrengthVal.textContent = v.toFixed(2);
  });
  dom.bloomRadius.addEventListener('input', () => {
    const v = parseFloat(dom.bloomRadius.value);
    bloomPass.radius = v;
    dom.bloomRadiusVal.textContent = v.toFixed(2);
  });
  dom.exposure.addEventListener('input', () => {
    const v = parseFloat(dom.exposure.value);
    renderer.toneMappingExposure = v;
    dom.exposureVal.textContent = v.toFixed(2);
  });
  dom.fogEnabled.addEventListener('change', () => {
    scene.fog = dom.fogEnabled.checked ? fog : null;
  });
  dom.fogDensity.addEventListener('input', () => {
    const v = parseFloat(dom.fogDensity.value);
    fog.density = v;
    dom.fogDensityVal.textContent = v.toFixed(3);
  });
  dom.emissive.addEventListener('input', () => {
    const v = parseFloat(dom.emissive.value);
    if (voxelMaterialShader) {
      voxelMaterialShader.uniforms['uEmissiveBoost']!.value = v;
    }
    dom.emissiveVal.textContent = v.toFixed(2);
  });
  dom.roughness.addEventListener('input', () => {
    const v = parseFloat(dom.roughness.value);
    voxelMaterial.roughness = v;
    dom.roughnessVal.textContent = v.toFixed(2);
  });
  dom.ssaoEnabled.addEventListener('change', () => {
    ssaoPass.enabled = dom.ssaoEnabled.checked;
  });
  dom.ssaoRadius.addEventListener('input', () => {
    const v = parseFloat(dom.ssaoRadius.value);
    ssaoPass.kernelRadius = v;
    dom.ssaoRadiusVal.textContent = v.toFixed(1);
  });
  dom.envEnabled.addEventListener('change', () => {
    scene.environment = dom.envEnabled.checked ? spaceEnv : null;
  });
  dom.envBackground.addEventListener('change', () => {
    scene.background = dom.envBackground.checked ? spaceEnv : defaultBackground;
  });

  // ----- Slice (z = 0 only) — proto-9-style 2D view --------------------------
  let sliceEnabled = false;
  dom.sliceEnabled.addEventListener('change', () => {
    sliceEnabled = dom.sliceEnabled.checked;
    lastRenderedTick = -1; // force a re-render even if no new snapshot.
  });

  // ----- Pause / Reset / config listeners ------------------------------------
  let running = true;
  dom.pauseBtn.addEventListener('click', () => {
    running = !running;
    dom.pauseBtn.textContent = running ? 'Pause' : 'Resume';
    sendRunning(running);
  });
  dom.resetBtn.addEventListener('click', () => {
    config.seed = parseInt(dom.seed.value, 10) || 0;
    config.energy = parseInt(dom.energyIn.value, 10) || 0;
    config.rngKind = dom.rngXs32.checked ? 'xorshift32' : 'pcg';
    config.legacyTickOffset = dom.legacyTickOffset.checked;
    config.legacyFullPrecision = dom.legacyFullPrecision.checked;
    config.legacyPortWrap = dom.legacyPortWrap.checked;
    config.legacyOpcodeSet = dom.legacyOpcodeSet.checked;
    running = true;
    dom.pauseBtn.textContent = 'Pause';
    sendInit();
  });
  // RNG checkbox change is captured into config.rngKind only on Reset —
  // switching backends mid-run would leave existing cells inconsistent.
  dom.legacyTickOffset.addEventListener('change', () => {
    config.legacyTickOffset = dom.legacyTickOffset.checked;
    sendConfig();
  });
  dom.legacyFullPrecision.addEventListener('change', () => {
    config.legacyFullPrecision = dom.legacyFullPrecision.checked;
    sendConfig();
  });
  dom.legacyPortWrap.addEventListener('change', () => {
    config.legacyPortWrap = dom.legacyPortWrap.checked;
    sendConfig();
  });
  dom.legacyOpcodeSet.addEventListener('change', () => {
    config.legacyOpcodeSet = dom.legacyOpcodeSet.checked;
    sendConfig();
  });
  dom.coeff.addEventListener('input', () => {
    config.coeff = parseFloat(dom.coeff.value);
    dom.coeffVal.textContent = config.coeff.toFixed(2);
    sendConfig();
  });
  dom.k.addEventListener('input', () => {
    config.k = parseInt(dom.k.value, 10) || 1;
    dom.kVal.textContent = String(config.k);
    sendConfig();
  });
  dom.moveThreshold.addEventListener('input', () => {
    config.moveThreshold = parseFloat(dom.moveThreshold.value) || 2.0;
    dom.moveThresholdVal.textContent = config.moveThreshold.toFixed(1);
    sendConfig();
  });
  dom.trackerEnabled.addEventListener('change', () => {
    trackerEnabled = dom.trackerEnabled.checked;
  });
  dom.trailLen.addEventListener('input', () => {
    trailLen = parseInt(dom.trailLen.value, 10);
    dom.trailLenVal.textContent = String(trailLen);
    createTrailLine();
  });

  // ----- Camera initial fit --------------------------------------------------
  let cameraFitDirty = true;
  function fitCameraToBbox(bbox: SnapshotBbox): void {
    const { target, eye } = fitCamera(bbox);
    controls.target.set(target[0], target[1], target[2]);
    camera.position.set(eye[0], eye[1], eye[2]);
    camera.updateProjectionMatrix();
  }

  // ----- Frame loop ----------------------------------------------------------
  const tempMatrix = new THREE.Matrix4();
  const tempColor = new THREE.Color();
  const tempPos = new THREE.Vector3();
  const tempQuat = new THREE.Quaternion();
  const tempScale = new THREE.Vector3(1, 1, 1);
  const zeroScale = new THREE.Vector3(0, 0, 0);

  let lastT = performance.now();
  let fpsAvg = 0;
  let msPerTickAvg = 0;
  let ticksPerSecAvg = 0;
  let lastRenderedTick = -1;
  let lastTickStampT = 0;

  function frame(now: number): void {
    const dt = now - lastT;
    lastT = now;
    if (dt > 0) fpsAvg = 0.9 * fpsAvg + 0.1 * (1000 / dt);

    applyWsad(dt);

    if (latestSnapshot && latestSnapshot.tick !== lastRenderedTick) {
      renderSnapshot(latestSnapshot);
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
    composer.render();

    if (latestSnapshot) {
      dom.tick.textContent = String(latestSnapshot.tick);
      dom.cells.textContent = latestSnapshot.cellCount.toLocaleString();
      dom.energy.textContent = latestSnapshot.totalEnergy.toLocaleString();
      const formatted = fmtBbox(latestSnapshot.bbox);
      dom.bbox.textContent = formatted ?? '-';
    }
    dom.fps.textContent = fpsAvg.toFixed(1);
    dom.msPerTick.textContent = msPerTickAvg.toFixed(1);
    dom.ticksPerSec.textContent = ticksPerSecAvg.toFixed(1);

    maybeRefreshInspector();
    requestAnimationFrame(frame);
  }

  function renderSnapshot(state: SnapshotMsg): void {
    const { snap, stride, cellCount } = state;
    ensureCapacity(Math.max(cellCount, lastUsedCount));
    if (!voxelMesh) return;

    const analysis = analyzeSnapshot(snap, stride, cellCount, sliceEnabled);
    const totalE = state.totalEnergy;
    // Fall back to a small box around the origin so the camera-fit math
    // stays well-defined when slice mode hides everything.
    const bbox: SnapshotBbox = analysis.bbox ?? {
      minX: -1, maxX: 1, minY: -1, maxY: 1, minZ: -1, maxZ: 1,
    };
    const liveCellCount = state.cellCount;

    if (cameraFitDirty && cellCount > 0) {
      fitCameraToBbox(bbox);
      cameraFitDirty = false;
    }

    for (let i = 0; i < cellCount; i += 1) {
      const off = i * stride;
      const x = snap[off]! | 0;
      const y = snap[off + 1]! | 0;
      const z = snap[off + 2]! | 0;
      const e = snap[off + 3]!;

      if (sliceEnabled && z !== 0) {
        tempMatrix.compose(tempPos.set(0, 0, 0), tempQuat, zeroScale);
        voxelMesh.setMatrixAt(i, tempMatrix);
        continue;
      }

      // Compute the heat-ramp t once per cell — both the color and the
      // per-cell size factor depend on it.
      const t = meanRelativeT(e, totalE, liveCellCount);
      const perScale = voxelScale * voxelSizeFactor(t);
      tempScale.set(perScale, perScale, perScale);
      // Tiny deterministic per-cell offset breaks grid-aligned moire
      // in dense fields. Stable across frames so cells don't shimmer.
      tempPos.set(
        x + gridJitter(x, y, z, 0) * JITTER_AMPLITUDE,
        y + gridJitter(x, y, z, 1) * JITTER_AMPLITUDE,
        z + gridJitter(x, y, z, 2) * JITTER_AMPLITUDE,
      );
      tempMatrix.compose(tempPos, tempQuat, tempScale);
      voxelMesh.setMatrixAt(i, tempMatrix);

      const [r, g, b] = heatColor(t);
      tempColor.setRGB(r, g, b);
      voxelMesh.setColorAt(i, tempColor);
    }

    if (lastUsedCount > cellCount) {
      for (let i = cellCount; i < lastUsedCount; i += 1) {
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
    const cx = (bbox.minX + bbox.maxX) / 2;
    const cy = (bbox.minY + bbox.maxY) / 2;
    const cz = (bbox.minZ + bbox.maxZ) / 2;
    const halfSpan = Math.max(bbox.maxX - bbox.minX, bbox.maxY - bbox.minY, bbox.maxZ - bbox.minZ, 2) / 2 + 1;
    if (!voxelMesh.boundingSphere) voxelMesh.boundingSphere = new THREE.Sphere();
    voxelMesh.boundingSphere.center.set(cx, cy, cz);
    // Add a small slack to cover the jitter-displaced voxels so the
    // raycaster doesn't lose hits at the field's edge.
    voxelMesh.boundingSphere.radius = halfSpan + JITTER_AMPLITUDE;

    updateTrackerVisuals(analysis.maxCellIdx, snap, stride);
  }

  // ----- Inspector -----------------------------------------------------------
  const inspector = {
    panel: dom.inspector,
    coord: null as { readonly x: number; readonly y: number; readonly z: number } | null,
    iCoord: requireEl('iCoord', HTMLSpanElement),
    iTick: requireEl('iTick', HTMLSpanElement),
    iPc: requireEl('iPc', HTMLSpanElement),
    iEnergy: requireEl('iEnergy', HTMLSpanElement),
    iOriginTag: requireEl('iOriginTag', HTMLSpanElement),
    iAppearance: requireEl('iAppearance', HTMLSpanElement),
    iPointers: requireEl('iPointers', HTMLSpanElement),
    iRates: requireEl('iRates', HTMLSpanElement),
    iActiveOutflow: requireEl('iActiveOutflow', HTMLSpanElement),
    iInflow: requireEl('iInflow', HTMLSpanElement),
    iMemory: requireEl('iMemory', HTMLPreElement),
    iClose: requireEl('iClose', HTMLButtonElement),
  };
  inspector.iClose.addEventListener('click', () => {
    inspector.coord = null;
    inspector.panel.classList.remove('visible');
  });

  const raycaster = new THREE.Raycaster();
  const mouseNdc = new THREE.Vector2();

  renderer.domElement.addEventListener('click', (ev) => {
    const rect = renderer.domElement.getBoundingClientRect();
    mouseNdc.x = ((ev.clientX - rect.left) / rect.width) * 2 - 1;
    mouseNdc.y = -((ev.clientY - rect.top) / rect.height) * 2 + 1;
    raycaster.setFromCamera(mouseNdc, camera);

    if (!voxelMesh || !latestSnapshot) return;
    const hits = raycaster.intersectObject(voxelMesh, false);
    if (hits.length === 0) return;

    for (const hit of hits) {
      const idx = hit.instanceId;
      if (idx === undefined) continue;
      if (idx >= latestSnapshot.cellCount) continue;
      const off = idx * latestSnapshot.stride;
      const x = latestSnapshot.snap[off]! | 0;
      const y = latestSnapshot.snap[off + 1]! | 0;
      const z = latestSnapshot.snap[off + 2]! | 0;
      inspector.coord = { x, y, z };
      inspector.panel.classList.add('visible');
      requestInspect(x, y, z);
      return;
    }
  });

  function renderInspector(msg: CellDetailMsg): void {
    if (!inspector.coord) return;
    const { x, y, z } = inspector.coord;
    if (msg.x !== x || msg.y !== y || msg.z !== z) return; // stale
    const data = msg.data;
    const prefix = msg.prefix;

    if (data.length === 0) {
      inspector.iCoord.textContent = `(${x}, ${y}, ${z}) — no cell`;
      inspector.iTick.textContent = String(msg.tick);
      inspector.iPc.textContent = '-';
      inspector.iEnergy.textContent = '-';
      inspector.iOriginTag.textContent = '-';
      inspector.iAppearance.textContent = '-';
      inspector.iPointers.textContent = '';
      inspector.iRates.textContent = '';
      inspector.iActiveOutflow.textContent = '';
      inspector.iInflow.textContent = '';
      inspector.iMemory.textContent = '';
      return;
    }
    inspector.iCoord.textContent = `(${x}, ${y}, ${z})`;
    inspector.iTick.textContent = String(msg.tick);
    inspector.iPc.textContent = String(data[0]!);
    inspector.iEnergy.textContent = data[1]!.toLocaleString();
    inspector.iOriginTag.textContent = `0x${data[2]!.toString(16).padStart(8, '0')}`;
    inspector.iAppearance.textContent = `0x${data[3]!.toString(16).padStart(8, '0')}`;
    inspector.iPointers.textContent = fmtDirArr(data.slice(4, 10));
    inspector.iRates.textContent = fmtDirArr(data.slice(10, 16));
    inspector.iActiveOutflow.textContent = fmtDirArr(data.slice(16, 22));
    inspector.iInflow.textContent = fmtDirArr(data.slice(22, 28));
    inspector.iMemory.textContent = fmtMemoryHexDump(data.slice(prefix));
  }

  // Auto-refresh the inspector every ~5 frames while it's open and the
  // world is running, so the panel reflects live state without flooding
  // the worker with messages.
  let inspectorRefreshCounter = 0;
  function maybeRefreshInspector(): void {
    if (!inspector.coord || !running) return;
    inspectorRefreshCounter += 1;
    if (inspectorRefreshCounter >= 5) {
      inspectorRefreshCounter = 0;
      requestInspect(inspector.coord.x, inspector.coord.y, inspector.coord.z);
    }
  }

  requestAnimationFrame(frame);
}

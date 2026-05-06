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

import { fitCamera } from '../src/camera-fit.ts';
import { fmtBbox, fmtDirArr, fmtMemoryHexDump } from '../src/format.ts';
import { heatColor } from '../src/heat.ts';
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
  scene.background = new THREE.Color(0x05050a);

  const camera = new THREE.PerspectiveCamera(60, window.innerWidth / window.innerHeight, 0.1, 5000);
  camera.position.set(20, 20, 30);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(window.innerWidth, window.innerHeight);
  dom.container.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.05;

  window.addEventListener('resize', () => {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
  });

  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const dirLight = new THREE.DirectionalLight(0xffffff, 0.8);
  dirLight.position.set(50, 80, 50);
  scene.add(dirLight);

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
  const voxelGeometry = new THREE.SphereGeometry(0.45, 8, 6);
  const voxelMaterial = new THREE.MeshLambertMaterial();
  let voxelMesh: THREE.InstancedMesh | null = null;
  let voxelCapacity = 0;
  let lastUsedCount = 0;

  function ensureCapacity(n: number): void {
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
    for (let i = 0; i < cap; i += 1) {
      voxelMesh.setColorAt(i, initColor);
    }
    scene.add(voxelMesh);
    voxelCapacity = cap;
    lastUsedCount = 0;
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
      highlightMesh.position.set(x, y, z);
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
    renderer.render(scene, camera);

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
    const maxE = analysis.maxEnergy < 1 ? 1 : analysis.maxEnergy;
    // Fall back to a small box around the origin so the camera-fit math
    // stays well-defined when slice mode hides everything.
    const bbox: SnapshotBbox = analysis.bbox ?? {
      minX: -1, maxX: 1, minY: -1, maxY: 1, minZ: -1, maxZ: 1,
    };

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
      tempPos.set(x, y, z);
      tempMatrix.compose(tempPos, tempQuat, tempScale);
      voxelMesh.setMatrixAt(i, tempMatrix);

      const t = Math.sqrt(e / maxE);
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
    voxelMesh.boundingSphere.radius = halfSpan;

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

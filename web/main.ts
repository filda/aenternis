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
import { BokehPass } from 'three/addons/postprocessing/BokehPass.js';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { SSAOPass } from 'three/addons/postprocessing/SSAOPass.js';
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';

import { fitCamera } from '../src/camera-fit.ts';
import { fmtBbox, fmtDirArr, fmtMemoryHexDump } from '../src/format.ts';
import { disassemble } from '../src/disasm.ts';
import { meanRelativeT, voxelSizeFactor } from '../src/heat.ts';
import { cellColorInto, type ColorMode } from '../src/color.ts';
import { JITTER_AMPLITUDE, gridJitter } from '../src/jitter.ts';
import type { SimChannel } from '../src/native-client.ts';
import { PRESETS, findPreset } from '../src/presets.ts';
import { parseProgramText } from '../src/program-text.ts';
import { DEFAULT_PROGRAM_TEXT, DEFAULT_SIM_CONFIG, type SimConfig } from '../src/sim-defaults.ts';
import type {
  CellDetailMsg,
  ConfigMsg,
  InitMsg,
  InspectMsg,
  MetricsMsg,
  ProgramStartedMsg,
  RunProgramMsg,
  RunningMsg,
  SnapshotMsg,
  StepMsg,
  WorkerToMainMsg,
} from '../src/protocol.ts';
import { assemble } from '../src/asm.ts';
import { openNativeClient } from './native-client.ts';
import {
  analyzeLineage,
  analyzeSnapshot,
  type LineageStats,
  type SnapshotBbox,
} from '../src/snapshot.ts';
import {
  EMPTY_TRACKER_STATE,
  pushTrackerSample,
  resetTrackerState,
  type TrackerState,
} from '../src/tracker.ts';

interface BackendChoice {
  readonly backend: 'wasm' | 'native';
  /** Used only when `backend === 'native'`. */
  readonly server: string;
}

/** localStorage keys for the backend choice. Exported as constants
 *  so a future settings page can read/write them by name. */
const BACKEND_KEY = 'aenternis.backend';
const SERVER_KEY = 'aenternis.server';

/** Default WS endpoint when none is configured. Picks `location.hostname`
 *  so a viewer opened from a LAN IP (e.g. via `vite --host`) connects
 *  back to the same host's `aenternis-server`. */
function defaultServerUrl(): string {
  return `ws://${window.location.hostname || 'localhost'}:8765/sim`;
}

/** Resolve the backend choice from URL query, then localStorage,
 *  then defaults. URL flags (`?backend=...`, `?server=...`) override
 *  *and persist* — once you visit the URL the choice survives the
 *  next plain reload. */
function resolveBackendChoice(): BackendChoice {
  const params = new URLSearchParams(window.location.search);
  const urlBackend = params.get('backend');
  if (urlBackend === 'native' || urlBackend === 'wasm') {
    window.localStorage.setItem(BACKEND_KEY, urlBackend);
  }
  const urlServer = params.get('server');
  if (urlServer) {
    window.localStorage.setItem(SERVER_KEY, urlServer);
  }
  const stored = window.localStorage.getItem(BACKEND_KEY);
  const backend: 'wasm' | 'native' = stored === 'native' ? 'native' : 'wasm';
  const server = window.localStorage.getItem(SERVER_KEY) ?? defaultServerUrl();
  return { backend, server };
}

/** Build the SimChannel for the chosen backend. */
function createSimChannel(choice: BackendChoice): SimChannel {
  if (choice.backend === 'native') {
    return openNativeClient(choice.server);
  }
  // The Web Worker shape (postMessage / onmessage / terminate) is a
  // structural superset of SimChannel; the cast is just to satisfy
  // the type checker.
  const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
  return worker as unknown as SimChannel;
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
  // Mutable copy of the shared production defaults — UI listeners write back
  // into this as the user drags sliders. The starting values (the "Cauldron"
  // preset) live in src/sim-defaults.ts so the render-tuner captures against
  // the exact same world. See docs/mechanics.md for the preset rationale.
  const config: SimConfig = { ...DEFAULT_SIM_CONFIG };

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
    tickBtn: requireEl('tickBtn', HTMLButtonElement),
    resetBtn: requireEl('resetBtn', HTMLButtonElement),
    seed: requireEl('seed', HTMLInputElement),
    energyIn: requireEl('energy_in', HTMLInputElement),
    genesisWindow: requireEl('genesisWindow', HTMLInputElement),
    genesisFertility: requireEl('genesisFertility', HTMLInputElement),
    coeff: requireEl('coeff', HTMLInputElement),
    coeffVal: requireEl('coeffVal', HTMLSpanElement),
    k: requireEl('k', HTMLInputElement),
    kVal: requireEl('kVal', HTMLSpanElement),
    moveThreshold: requireEl('moveThreshold', HTMLInputElement),
    moveThresholdVal: requireEl('moveThresholdVal', HTMLSpanElement),
    gravity: requireEl('gravity', HTMLInputElement),
    gravityVal: requireEl('gravityVal', HTMLSpanElement),
    gravityAlpha: requireEl('gravityAlpha', HTMLInputElement),
    gravityAlphaVal: requireEl('gravityAlphaVal', HTMLSpanElement),
    gravityRadius: requireEl('gravityRadius', HTMLInputElement),
    gravityRadiusVal: requireEl('gravityRadiusVal', HTMLSpanElement),
    pressure: requireEl('pressure', HTMLInputElement),
    pressureVal: requireEl('pressureVal', HTMLSpanElement),
    pressureGamma: requireEl('pressureGamma', HTMLInputElement),
    pressureGammaVal: requireEl('pressureGammaVal', HTMLSpanElement),
    pressureEref: requireEl('pressureEref', HTMLInputElement),
    pressureErefVal: requireEl('pressureErefVal', HTMLSpanElement),
    mutationStrength: requireEl('mutationStrength', HTMLInputElement),
    mutationStrengthVal: requireEl('mutationStrengthVal', HTMLSpanElement),
    mutationHalfDensity: requireEl('mutationHalfDensity', HTMLInputElement),
    mutationHalfDensityVal: requireEl('mutationHalfDensityVal', HTMLSpanElement),
    metricsEvery: requireEl('metricsEvery', HTMLInputElement),
    mTick: requireEl('mTick', HTMLSpanElement),
    mCells: requireEl('mCells', HTMLSpanElement),
    mEntropy: requireEl('mEntropy', HTMLSpanElement),
    mDiversity: requireEl('mDiversity', HTMLSpanElement),
    mUnique: requireEl('mUnique', HTMLSpanElement),
    mHistCanvas: requireEl('mHistCanvas', HTMLCanvasElement),
    mSeriesCanvas: requireEl('mSeriesCanvas', HTMLCanvasElement),
    trackerEnabled: requireEl('trackerEnabled', HTMLInputElement),
    trailLen: requireEl('trailLen', HTMLInputElement),
    trailLenVal: requireEl('trailLenVal', HTMLSpanElement),
    trackerPos: requireEl('trackerPos', HTMLSpanElement),
    programText: requireEl('programText', HTMLTextAreaElement),
    programStatus: requireEl('programStatus', HTMLDivElement),
    programPreset: requireEl('programPreset', HTMLSelectElement),
    runPilgrimBtn: requireEl('runPilgrimBtn', HTMLButtonElement),
    colorMode: requireEl('colorMode', HTMLSelectElement),
    sliceEnabled: requireEl('sliceEnabled', HTMLInputElement),
    voxelSize: requireEl('voxelSize', HTMLInputElement),
    voxelSizeVal: requireEl('voxelSizeVal', HTMLSpanElement),
    minLuma: requireEl('minLuma', HTMLInputElement),
    minLumaVal: requireEl('minLumaVal', HTMLSpanElement),
    shapeSel: requireEl('shapeSel', HTMLSelectElement),
    bloomEnabled: requireEl('bloomEnabled', HTMLInputElement),
    bloomThreshold: requireEl('bloomThreshold', HTMLInputElement),
    bloomThresholdVal: requireEl('bloomThresholdVal', HTMLSpanElement),
    bloomStrength: requireEl('bloomStrength', HTMLInputElement),
    bloomStrengthVal: requireEl('bloomStrengthVal', HTMLSpanElement),
    bloomRadius: requireEl('bloomRadius', HTMLInputElement),
    bloomRadiusVal: requireEl('bloomRadiusVal', HTMLSpanElement),
    dofEnabled: requireEl('dofEnabled', HTMLInputElement),
    dofFocus: requireEl('dofFocus', HTMLInputElement),
    dofFocusVal: requireEl('dofFocusVal', HTMLSpanElement),
    dofAperture: requireEl('dofAperture', HTMLInputElement),
    dofApertureVal: requireEl('dofApertureVal', HTMLSpanElement),
    dofMaxblur: requireEl('dofMaxblur', HTMLInputElement),
    dofMaxblurVal: requireEl('dofMaxblurVal', HTMLSpanElement),
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
    inspector: requireEl('inspector', HTMLElement),
    backendNative: requireEl('backendNative', HTMLInputElement),
    backendUrl: requireEl('backendUrl', HTMLSpanElement),
  };

  // ----- Backend channel setup ----------------------------------------------
  // Two backends live behind the same `SimChannel` interface:
  //   - WASM Web Worker (default): `./worker.js` resolves to the
  //     tsc-emitted file in production and `./worker.ts` source in
  //     Vite dev (Vite rewrites `.js` → `.ts`).
  //   - Native dev backend: WebSocket to `aenternis-server`. Picked
  //     when the URL query or localStorage selects `backend=native`.
  // The Worker shape is structurally compatible with `SimChannel`
  // (postMessage / onmessage / terminate), so the rest of the file
  // doesn't care which backend it talks to.
  const backendChoice = resolveBackendChoice();
  const channel: SimChannel = createSimChannel(backendChoice);
  const isNativeBackend = backendChoice.backend === 'native';
  let latestSnapshot: SnapshotMsg | null = null;
  let workerReady = false;

  // ----- Backend UI binding --------------------------------------------------
  // Show the current backend in the side panel. The checkbox toggles
  // `localStorage` and reloads — we don't hot-swap the channel. The
  // user expects "switch backend" to mean "talk to the other one
  // from a clean slate"; reload makes that contract obvious.
  dom.backendNative.checked = isNativeBackend;
  dom.backendUrl.textContent = isNativeBackend ? backendChoice.server : '(WASM Worker)';
  dom.backendNative.addEventListener('change', () => {
    window.localStorage.setItem(
      BACKEND_KEY,
      dom.backendNative.checked ? 'native' : 'wasm',
    );
    window.location.reload();
  });

  channel.onmessage = (ev) => {
    const msg = ev.data as WorkerToMainMsg;
    if (msg.type === 'ready') {
      workerReady = true;
      if (isNativeBackend) {
        // Server already owns the shared world; don't reset it on
        // connect. Wait for the Welcome frame that follows to learn
        // the current `running` state. Reset stays available via
        // the explicit Reset button (which is a global action — it
        // resets the world for every connected client).
        return;
      }
      // Page load lands in the same paused state as Reset — user
      // sees tick 0 of the snapshot and decides when to start with
      // Pause/Resume or Tick.
      initPaused();
    } else if (msg.type === 'welcome') {
      // Native-only: server tells a fresh client whether the shared
      // world is currently ticking. Sync the Pause/Resume button so
      // late-joiners don't show a stale "Resume" while ticks fly.
      running = msg.running;
      dom.pauseBtn.textContent = running ? 'Pause' : 'Resume';
    } else if (msg.type === 'snapshot') {
      latestSnapshot = msg;
      // Render this snapshot even if its tick matches the last one — used
      // right after a possession so the tagged cell appears at once.
      if (forceRenderNext) {
        lastRenderedTick = -1;
        forceRenderNext = false;
      }
    } else if (msg.type === 'cellDetail') {
      renderInspector(msg);
    } else if (msg.type === 'programStarted') {
      onProgramStarted(msg);
    } else if (msg.type === 'programRejected') {
      dom.programStatus.textContent = `odmítnuto: ${msg.reason}`;
    } else if (msg.type === 'metrics') {
      updateMetrics(msg);
    }
  };

  /** Send an `init` to the worker and immediately follow with
   *  `running:false` so the new world is held on tick 0. Shared by the
   *  initial page-load handshake and the Reset button. */
  function initPaused(): void {
    running = false;
    dom.pauseBtn.textContent = 'Resume';
    sendInit();
    // Worker init flips its own running=true and schedules a loop
    // callback; that callback is queued behind this running:false
    // message, so the running guard short-circuits it before any tick.
    sendRunning(false);
  }

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
      gravity: config.gravity,
      gravityAlpha: config.gravityAlpha,
      gravityRadius: config.gravityRadius,
      pressure: config.pressure,
      pressureGamma: config.pressureGamma,
      pressureEref: config.pressureEref,
      mutationStrength: config.mutationStrength,
      mutationHalfDensity: config.mutationHalfDensity,
      genesisWindow: config.genesisWindow,
      genesisFertility: config.genesisFertility,
      metricsEvery: readMetricsEvery(),
      program,
    };
    channel.postMessage(init);
    cameraFitDirty = true;
    trackerState = resetTrackerState();
    // A new big-bang is a fresh run — drop the old metrics time series so the
    // sparkline doesn't splice two unrelated worlds together.
    entropySeries.length = 0;
    diversitySeries.length = 0;
  }

  /** Metrics sampling cadence (ticks) from the Config input; `0` = off.
   *  Viewer-only — not part of `SimConfig` (the tuner must never sample). */
  function readMetricsEvery(): number {
    return Math.max(0, parseInt(dom.metricsEvery.value, 10) || 0);
  }

  function sendConfig(): void {
    if (!workerReady) return;
    const cfg: ConfigMsg = {
      type: 'config',
      coeff: config.coeff,
      k: config.k,
      moveThreshold: config.moveThreshold,
      gravity: config.gravity,
      gravityAlpha: config.gravityAlpha,
      gravityRadius: config.gravityRadius,
      pressure: config.pressure,
      pressureGamma: config.pressureGamma,
      pressureEref: config.pressureEref,
      mutationStrength: config.mutationStrength,
      mutationHalfDensity: config.mutationHalfDensity,
      metricsEvery: readMetricsEvery(),
    };
    channel.postMessage(cfg);
  }

  // Code-metrics time series (entropy + diversity), capped so a long run
  // doesn't grow unbounded. Each `metrics` message pushes one sample.
  const METRICS_SERIES_CAP = 300;
  const entropySeries: number[] = [];
  const diversitySeries: number[] = [];
  // Max opcode entropy is log2(bin count); the histogram length tells us the
  // count, so we normalize the sparkline against it once we've seen a sample.
  let metricsMaxEntropy = Math.log2(31);

  /** Apply one metrics sample: numeric readouts, opcode histogram bars, and
   *  the entropy/diversity sparkline. Pure rendering — no world mutation. */
  function updateMetrics(msg: MetricsMsg): void {
    dom.mTick.textContent = msg.tick.toLocaleString('en-US');
    dom.mCells.textContent = msg.cells.toLocaleString('en-US');
    dom.mEntropy.textContent = msg.entropy.toFixed(3);
    dom.mDiversity.textContent = msg.diversity.toFixed(4);
    dom.mUnique.textContent = String(msg.uniqueTypes);

    const bins = msg.opcodeHist.length;
    if (bins > 0) metricsMaxEntropy = Math.log2(bins);
    drawHistogram(dom.mHistCanvas, msg.opcodeHist);

    entropySeries.push(msg.entropy);
    diversitySeries.push(msg.diversity);
    if (entropySeries.length > METRICS_SERIES_CAP) entropySeries.shift();
    if (diversitySeries.length > METRICS_SERIES_CAP) diversitySeries.shift();
    drawSeries(dom.mSeriesCanvas);
  }

  /** Draw a normalized bar-per-opcode histogram (tallest bar = full height). */
  function drawHistogram(canvas: HTMLCanvasElement, hist: Float64Array): void {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const { width: w, height: h } = canvas;
    ctx.clearRect(0, 0, w, h);
    let max = 0;
    for (const v of hist) if (v > max) max = v;
    if (max <= 0) return;
    const n = hist.length;
    const bw = w / n;
    ctx.fillStyle = '#5ad1c8';
    for (let i = 0; i < n; i += 1) {
      const bh = (hist[i]! / max) * (h - 1);
      ctx.fillRect(i * bw, h - bh, Math.max(1, bw - 0.5), bh);
    }
  }

  /** Draw entropy (green, scaled to its max) and diversity (orange, 0..1) as
   *  two sparklines over the captured series. */
  function drawSeries(canvas: HTMLCanvasElement): void {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const { width: w, height: h } = canvas;
    ctx.clearRect(0, 0, w, h);
    const line = (series: number[], scale: number, color: string): void => {
      if (series.length < 2) return;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let i = 0; i < series.length; i += 1) {
        const x = (i / (series.length - 1)) * (w - 1);
        const y = h - 1 - Math.min(1, series[i]! / scale) * (h - 2);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    };
    line(entropySeries, metricsMaxEntropy, '#6ee06e');
    line(diversitySeries, 1, '#e0a056');
  }

  function sendRunning(running: boolean): void {
    if (!workerReady) return;
    const msg: RunningMsg = { type: 'running', running };
    channel.postMessage(msg);
  }

  function sendStep(): void {
    if (!workerReady) return;
    const msg: StepMsg = { type: 'step' };
    channel.postMessage(msg);
  }

  function requestInspect(x: number, y: number, z: number): void {
    if (!workerReady) return;
    const msg: InspectMsg = { type: 'inspect', x, y, z };
    channel.postMessage(msg);
  }

  /** Assemble the program textarea and ask the worker to possess an
   *  eligible host with it at runtime (Project Pilgrim). The worker
   *  replies `programStarted` (→ follow) or `programRejected`. */
  function sendRunProgram(): void {
    if (!workerReady) return;
    const { slots, errors } = assemble(dom.programText.value);
    if (errors.length > 0) {
      dom.programStatus.textContent = `${errors.length} parse error(s): ${errors.join('; ')}`;
      return;
    }
    if (slots.length === 0) {
      dom.programStatus.textContent = 'prázdný program';
      return;
    }
    const msg: RunProgramMsg = {
      type: 'runProgram',
      code: slots,
      reserve: PILGRIM_RESERVE,
      tag: PILGRIM_TAG,
      appearance: PILGRIM_APPEARANCE,
    };
    channel.postMessage(msg);
    dom.programStatus.textContent = `${slots.length} slot(s) → hledám hostitele…`;
  }

  /** A possession succeeded: lock the tracker, camera and inspector onto
   *  the new lineage and pull the camera back from auto-zoom. */
  function onProgramStarted(msg: ProgramStartedMsg): void {
    trackedTag = msg.tag;
    pilgrimFollow = true;
    pilgrimSeen = false;
    lastPilgrimPos = null;
    trackerState = resetTrackerState();
    trackerEnabled = true;
    dom.trackerEnabled.checked = true;
    cancelAutoZoom();
    inspector.coord = { x: msg.x, y: msg.y, z: msg.z };
    inspector.panel.classList.add('visible');
    requestInspect(msg.x, msg.y, msg.z);
    dom.programStatus.textContent = `pilgrim spuštěn @ (${msg.x},${msg.y},${msg.z})`;
    // The post-possess snapshot carries the same tick as the last render
    // (possess doesn't advance the clock), so the tick-gated render would
    // skip it. Force the next snapshot to render so the freshly-possessed,
    // tagged cell shows up immediately — even before the user steps.
    forceRenderNext = true;
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

  // ----- Auto-zoom-out after a fresh fit -------------------------------------
  // The initial fit lands tight against the tick-0 bbox; auto-zoom-out gently
  // pulls the camera back as the world grows. Cancelled the moment the user
  // touches the canvas or presses any key so manual control always wins.
  const AUTO_ZOOM_TICKS = 400;
  // Total distance multiplier over the window: the camera eases from the tight
  // tick-0 fit out to this × the start distance, then stops. Kept modest on
  // purpose — a large factor flings the world away *and* drives it into the fog
  // (density·distance grows with the pull-back). A gentle ~3× keeps the world
  // big and well within the fog's visible range, so the fog needs no tweaking.
  const AUTO_ZOOM_TOTAL_FACTOR = 8.0;
  let autoZoomTicksLeft = 0;
  let autoZoomBaseTick = 0;
  let autoZoomBaseDist = 0;
  const autoZoomDir = new THREE.Vector3();

  function startAutoZoom(tick: number): void {
    autoZoomDir.subVectors(camera.position, controls.target);
    autoZoomBaseDist = autoZoomDir.length();
    if (autoZoomBaseDist < 1e-6) {
      autoZoomTicksLeft = 0;
      return;
    }
    autoZoomDir.normalize();
    autoZoomBaseTick = tick;
    autoZoomTicksLeft = AUTO_ZOOM_TICKS;
  }

  function cancelAutoZoom(): void {
    autoZoomTicksLeft = 0;
  }

  function stepAutoZoom(): void {
    if (autoZoomTicksLeft <= 0 || !latestSnapshot) return;
    const elapsed = latestSnapshot.tick - autoZoomBaseTick;
    const progress = Math.max(0, Math.min(1, elapsed / AUTO_ZOOM_TICKS));
    const dist = autoZoomBaseDist * Math.pow(AUTO_ZOOM_TOTAL_FACTOR, progress);
    camera.position.copy(controls.target).addScaledVector(autoZoomDir, dist);
    if (progress >= 1) autoZoomTicksLeft = 0;
  }

  renderer.domElement.addEventListener('pointerdown', cancelAutoZoom);
  renderer.domElement.addEventListener('wheel', cancelAutoZoom, { passive: true });

  // ----- Pilgrim camera-follow ----------------------------------------------
  // Per-frame lerp fraction toward the pilgrim. Small enough to smooth the
  // discrete cell-to-cell hops of the lineage torch into continuous motion.
  const FOLLOW_LERP = 0.18;
  const _followDir = new THREE.Vector3();
  /** One-time lock-on: snap the view onto the pilgrim at a fixed, readable
   *  follow distance (the auto-zoom may have pulled the camera far out). */
  function recenterCameraOnPilgrim(x: number, y: number, z: number): void {
    _followDir.subVectors(camera.position, controls.target);
    let dist = _followDir.length();
    if (!Number.isFinite(dist) || dist < 1e-3) dist = 30;
    dist = Math.min(Math.max(dist, 18), 45);
    _followDir.normalize();
    controls.target.set(x, y, z);
    camera.position.copy(controls.target).addScaledVector(_followDir, dist);
  }
  /** Per-frame smooth follow: ease target + camera toward the latest
   *  pilgrim position by the same delta, preserving zoom/angle so the user
   *  can still orbit. Frame-paced (not tick-paced) so cell hops don't lurch. */
  function smoothFollowFrame(): void {
    if (!pilgrimFollow || !pilgrimTargetPos) return;
    const dx = (pilgrimTargetPos.x - controls.target.x) * FOLLOW_LERP;
    const dy = (pilgrimTargetPos.y - controls.target.y) * FOLLOW_LERP;
    const dz = (pilgrimTargetPos.z - controls.target.z) * FOLLOW_LERP;
    controls.target.set(controls.target.x + dx, controls.target.y + dy, controls.target.z + dz);
    camera.position.set(camera.position.x + dx, camera.position.y + dy, camera.position.z + dz);
  }

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
  // Depth of field (Bokeh). Off by default — when on, cells in front of /
  // behind the focus plane melt into a soft crystalline haze while the
  // focused mid-field stays sharp. Inserted before bloom so the glow
  // blooms the already-defocused image. Re-renders a depth pass, so it's
  // the cheap-toggle kind; disabled passes are skipped by the composer.
  const bokehPass = new BokehPass(scene, camera, {
    focus: parseFloat(dom.dofFocus.value),
    aperture: parseFloat(dom.dofAperture.value),
    maxblur: parseFloat(dom.dofMaxblur.value),
  });
  bokehPass.enabled = dom.dofEnabled.checked;
  composer.addPass(bokehPass);
  // three's BokehPass types `uniforms` as an opaque `{}`; alias it so the
  // focus / aperture / maxblur sliders can poke the shader values directly.
  const bokehUniforms = bokehPass.uniforms as Record<string, THREE.IUniform>;
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
    bokehPass.setSize(window.innerWidth, window.innerHeight);
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
    cancelAutoZoom();
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
  // The icosahedron is inscribed, so it reads optically smaller than the
  // sphere at the same nominal radius; 0.6 lands it on roughly the same
  // visual footprint. Detail 0 = 20 triangles — cheap, with sharp facets.
  const VOXEL_CRYSTAL_RADIUS = 0.6;
  type VoxelShape = 'sphere' | 'octahedron' | 'crystal';
  function makeVoxelGeometry(shape: VoxelShape): THREE.BufferGeometry {
    switch (shape) {
      case 'octahedron':
        return new THREE.OctahedronGeometry(VOXEL_OCTA_RADIUS, 0);
      case 'crystal':
        return new THREE.IcosahedronGeometry(VOXEL_CRYSTAL_RADIUS, 0);
      case 'sphere':
      default:
        return new THREE.SphereGeometry(VOXEL_SPHERE_RADIUS, 8, 6);
    }
  }
  let voxelShape: VoxelShape = 'crystal';
  let voxelGeometry: THREE.BufferGeometry = makeVoxelGeometry(voxelShape);
  // PBR material — metalness 0 (no specular highlights from a metallic
  // surface) and a soft roughness keep the diffuse shading subtle so
  // the heat-ramp color stays the dominant visual signal. Per-cell
  // emissive contribution is injected via onBeforeCompile below: the
  // surface color is added to the emissive radiance, so a hot (white)
  // cell radiates strongly while a cold (near-black) cell barely glows.
  const voxelMaterial = new THREE.MeshStandardMaterial({
    metalness: 0.0,
    roughness: parseFloat(dom.roughness.value),
    // Crystal is the default shape, so it must boot with hard facets;
    // setVoxelShape keeps this in sync on later switches.
    flatShading: voxelShape === 'crystal',
  });
  // Captured shader handle so the emissive slider can poke at the
  // uniform after the material has been compiled. Three.js compiles
  // lazily on first render, so we read it inside `onBeforeCompile`.
  let voxelMaterialShader: THREE.WebGLProgramParametersWithUniforms | null = null;
  voxelMaterial.onBeforeCompile = (shader) => {
    shader.uniforms['uEmissiveBoost'] = { value: parseFloat(dom.emissive.value) };
    shader.uniforms['uMinLuma'] = { value: parseFloat(dom.minLuma.value) };
    shader.fragmentShader = shader.fragmentShader
      .replace(
        'uniform vec3 emissive;',
        `uniform vec3 emissive;
uniform float uEmissiveBoost;
uniform float uMinLuma;`,
      )
      // Alpha-test discard pro low-energy buňky: po color_fragment už
      // diffuseColor.rgb obsahuje per-instance heat color. Buňky pod
      // luminance prahem se zahodí ještě před lighting passes — voxel
      // zmizí včetně siluety v SSAO.
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

  function setVoxelShape(shape: VoxelShape): void {
    if (voxelShape === shape) return;
    voxelShape = shape;
    voxelGeometry.dispose();
    voxelGeometry = makeVoxelGeometry(voxelShape);
    // Only the crystal gets hard facets; sphere / octahedron keep their
    // smooth position-derived normals. Toggling flatShading forces a
    // shader recompile, which re-runs onBeforeCompile and re-captures
    // voxelMaterialShader — the emissive / min-luma uniforms are seeded
    // from the current slider values there, so both sliders keep working.
    voxelMaterial.flatShading = shape === 'crystal';
    voxelMaterial.needsUpdate = true;
    rebuildVoxelMesh(Math.max(voxelCapacity, 256));
    lastRenderedTick = -1;
  }
  ensureCapacity(1024);

  // ----- Tracker meshes ------------------------------------------------------
  let trackerState: TrackerState = EMPTY_TRACKER_STATE;
  let trackerEnabled = false;
  let trailLen = 60;

  // ----- Pilgrim tracking (Project Pilgrim, docs/pilgrim.md) -----------------
  // Reserved lineage marker stamped on a possessed entity so the camera and
  // inspector can follow it. 0x50494c47 = "PILG"; the appearance is a
  // distinct colour the war-paint / lineage view modes pick up.
  const PILGRIM_TAG = 0x50494c47;
  const PILGRIM_APPEARANCE = 0xff3030;
  // Extra slots the host must have beyond the program — scratch / compute /
  // emission fuel (docs/pilgrim.md). Kept small so cooler, more peripheral
  // cells qualify (a hot core host mutates the genome to death at once).
  // Tunable; a UI knob is a follow-up.
  const PILGRIM_RESERVE = 8;
  let trackedTag: number | null = null; // non-null → follow this lineage
  let pilgrimFollow = false; // camera tracks the carrier
  let pilgrimSeen = false; // have we seen the tag at least once
  let lastPilgrimPos: { x: number; y: number; z: number } | null = null;
  let pilgrimTargetPos: { x: number; y: number; z: number } | null = null;
  // Set when a possession lands so the next snapshot renders even though its
  // tick matches the last rendered one (possess doesn't advance the tick).
  let forceRenderNext = false;
  let highlightMesh: THREE.LineSegments | null = null;
  let trailLine: THREE.Line | null = null;
  // Wireframe cage around the WHOLE pilgrim lineage (all tagged descendants),
  // sized per tick to the lineage bounding box. A unit cube scaled to span.
  let lineageBox: THREE.LineSegments | null = null;

  function createHighlightMesh(): void {
    if (highlightMesh) {
      scene.remove(highlightMesh);
      highlightMesh.geometry.dispose();
      (highlightMesh.material as THREE.Material).dispose();
    }
    // Larger, high-contrast cyan cage so the tracked cell (rendered bright
    // magenta) is unmistakable against the field in any colour mode.
    const geo = new THREE.BoxGeometry(2.4, 2.4, 2.4);
    const mat = new THREE.LineBasicMaterial({ color: 0x00ffff, transparent: true, opacity: 1.0 });
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
  function createLineageBox(): void {
    if (lineageBox) {
      scene.remove(lineageBox);
      lineageBox.geometry.dispose();
      (lineageBox.material as THREE.Material).dispose();
    }
    // Unit cube centered at origin; scaled to the lineage span each tick.
    const geo = new THREE.BoxGeometry(1, 1, 1);
    const mat = new THREE.LineBasicMaterial({ color: 0x00ffff, transparent: true, opacity: 0.9 });
    const wire = new THREE.LineSegments(new THREE.EdgesGeometry(geo), mat);
    wire.visible = false;
    wire.frustumCulled = false;
    scene.add(wire);
    lineageBox = wire;
  }

  /** Size + position the lineage cage to the descendant cloud's bbox. */
  function updateLineageBox(lin: LineageStats): void {
    if (!lineageBox) return;
    lineageBox.position.set(
      (lin.minX + lin.maxX) / 2,
      (lin.minY + lin.maxY) / 2,
      (lin.minZ + lin.maxZ) / 2,
    );
    // +2 so a single-cell (zero-span) lineage still gets a visible cage.
    lineageBox.scale.set(
      lin.maxX - lin.minX + 2,
      lin.maxY - lin.minY + 2,
      lin.maxZ - lin.minZ + 2,
    );
    lineageBox.visible = true;
  }

  createHighlightMesh();
  createTrailLine();
  createLineageBox();

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
  let voxelScale = parseFloat(dom.voxelSize.value);
  dom.voxelSize.addEventListener('input', () => {
    voxelScale = parseFloat(dom.voxelSize.value);
    dom.voxelSizeVal.textContent = voxelScale.toFixed(2);
    lastRenderedTick = -1; // force a re-render even if no new snapshot.
  });
  dom.minLuma.addEventListener('input', () => {
    const v = parseFloat(dom.minLuma.value);
    if (voxelMaterialShader) {
      voxelMaterialShader.uniforms['uMinLuma']!.value = v;
    }
    dom.minLumaVal.textContent = v.toFixed(2);
  });
  dom.shapeSel.addEventListener('change', () => {
    setVoxelShape(dom.shapeSel.value as VoxelShape);
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
  dom.dofEnabled.addEventListener('change', () => {
    bokehPass.enabled = dom.dofEnabled.checked;
  });
  dom.dofFocus.addEventListener('input', () => {
    const v = parseFloat(dom.dofFocus.value);
    bokehUniforms['focus']!.value = v;
    dom.dofFocusVal.textContent = v.toFixed(0);
  });
  dom.dofAperture.addEventListener('input', () => {
    const v = parseFloat(dom.dofAperture.value);
    bokehUniforms['aperture']!.value = v;
    dom.dofApertureVal.textContent = v.toFixed(4);
  });
  dom.dofMaxblur.addEventListener('input', () => {
    const v = parseFloat(dom.dofMaxblur.value);
    bokehUniforms['maxblur']!.value = v;
    dom.dofMaxblurVal.textContent = v.toFixed(3);
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

  // ----- Color mode (energy heat ramp / war paint / lineage) -----------------
  let colorMode: ColorMode = 'energy';
  dom.colorMode.addEventListener('change', () => {
    colorMode = dom.colorMode.value as ColorMode;
    lastRenderedTick = -1; // force a re-render even if no new snapshot.
  });

  // ----- Pause / Tick / Reset / config listeners ----------------------------
  // Initial state mirrors `initPaused()`: world is held on tick 0 until
  // the user explicitly starts it with Pause/Resume or steps with Tick.
  let running = false;
  dom.pauseBtn.textContent = 'Resume';
  dom.pauseBtn.addEventListener('click', () => {
    running = !running;
    dom.pauseBtn.textContent = running ? 'Pause' : 'Resume';
    sendRunning(running);
  });
  dom.tickBtn.addEventListener('click', () => {
    // Single-step. If the loop is running, pause first so the manual
    // tick doesn't race with auto-stepping. Worker queues messages in
    // order, so `running:false` lands before `step` and the next loop
    // iteration is skipped.
    if (running) {
      running = false;
      dom.pauseBtn.textContent = 'Resume';
      sendRunning(false);
    }
    sendStep();
  });
  dom.resetBtn.addEventListener('click', () => {
    config.seed = parseInt(dom.seed.value, 10) || 0;
    config.energy = parseInt(dom.energyIn.value, 10) || 0;
    // Genesis (initial-condition) knobs are read at Reset time, like
    // seed/energy — they reshape the origin program, not the live world.
    config.genesisWindow = Math.max(1, parseInt(dom.genesisWindow.value, 10) || 256);
    config.genesisFertility = Math.max(0, parseFloat(dom.genesisFertility.value) || 0);
    initPaused();
  });
  // Seed the program textarea from the shared default so the origin-cell
  // overlay matches what the render-tuner captures (single source of truth).
  dom.programText.value = DEFAULT_PROGRAM_TEXT;
  for (const preset of PRESETS) {
    const opt = document.createElement('option');
    opt.value = preset.name;
    opt.textContent = preset.name;
    opt.title = preset.hint;
    dom.programPreset.appendChild(opt);
  }
  dom.programPreset.addEventListener('change', () => {
    const preset = findPreset(dom.programPreset.value);
    if (preset) {
      dom.programText.value = preset.source;
      const { status } = parseProgramText(preset.source);
      dom.programStatus.textContent = status;
    }
  });
  dom.programText.addEventListener('input', () => {
    // User-edited program no longer matches any preset.
    dom.programPreset.value = '';
  });
  dom.runPilgrimBtn.addEventListener('click', () => {
    sendRunProgram();
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
  dom.gravity.addEventListener('input', () => {
    config.gravity = parseFloat(dom.gravity.value) || 0.0;
    dom.gravityVal.textContent = config.gravity.toFixed(2);
    sendConfig();
  });
  dom.gravityAlpha.addEventListener('input', () => {
    config.gravityAlpha = parseFloat(dom.gravityAlpha.value) || 0.0;
    dom.gravityAlphaVal.textContent = config.gravityAlpha.toFixed(2);
    sendConfig();
  });
  dom.gravityRadius.addEventListener('input', () => {
    config.gravityRadius = parseInt(dom.gravityRadius.value, 10) || 1;
    dom.gravityRadiusVal.textContent = String(config.gravityRadius);
    sendConfig();
  });
  dom.pressure.addEventListener('input', () => {
    config.pressure = parseFloat(dom.pressure.value) || 0.0;
    dom.pressureVal.textContent = config.pressure.toFixed(3);
    sendConfig();
  });
  dom.pressureGamma.addEventListener('input', () => {
    config.pressureGamma = parseFloat(dom.pressureGamma.value) || 2.0;
    dom.pressureGammaVal.textContent = config.pressureGamma.toFixed(1);
    sendConfig();
  });
  dom.pressureEref.addEventListener('input', () => {
    config.pressureEref = parseFloat(dom.pressureEref.value) || 1.0;
    dom.pressureErefVal.textContent = String(Math.round(config.pressureEref));
    sendConfig();
  });
  dom.mutationStrength.addEventListener('input', () => {
    config.mutationStrength = parseFloat(dom.mutationStrength.value) || 0.0;
    dom.mutationStrengthVal.textContent = config.mutationStrength.toFixed(2);
    sendConfig();
  });
  dom.mutationHalfDensity.addEventListener('input', () => {
    config.mutationHalfDensity = parseInt(dom.mutationHalfDensity.value, 10) || 0;
    dom.mutationHalfDensityVal.textContent = String(config.mutationHalfDensity);
    sendConfig();
  });
  // Metrics cadence is a live config knob (0 = off → full speed). 'change'
  // not 'input' so a brief keystroke like an empty field mid-edit doesn't
  // thrash the worker.
  dom.metricsEvery.addEventListener('change', sendConfig);
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
  const tempEuler = new THREE.Euler();
  const tempScale = new THREE.Vector3(1, 1, 1);
  const zeroScale = new THREE.Vector3(0, 0, 0);
  // Reused RGB buffer for the alloc-free per-cell color path (see
  // cellColorInto) — one array for the whole render loop, not one per cell.
  const colorOut: [number, number, number] = [0, 0, 0];
  const TWO_PI = Math.PI * 2;

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
    stepAutoZoom();
    smoothFollowFrame();

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
      startAutoZoom(state.tick);
    }

    // Crystals (and octahedra) get a per-cell orientation; a rotated sphere
    // is identical, so it would just waste a quaternion.
    const rotated = voxelShape !== 'sphere';

    for (let i = 0; i < cellCount; i += 1) {
      const off = i * stride;
      const x = snap[off]! | 0;
      const y = snap[off + 1]! | 0;
      const z = snap[off + 2]! | 0;
      const e = snap[off + 3]!;
      const originTag = snap[off + 4]!;
      const appearance = snap[off + 5]!;

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
      // Tiny deterministic per-cell offset breaks grid-aligned moire in
      // dense fields. Stable across frames so cells don't shimmer.
      tempPos.set(
        x + gridJitter(x, y, z, 0) * JITTER_AMPLITUDE,
        y + gridJitter(x, y, z, 1) * JITTER_AMPLITUDE,
        z + gridJitter(x, y, z, 2) * JITTER_AMPLITUDE,
      );
      if (rotated) {
        // Deterministic per-cell orientation: stable across frames (no
        // shimmer on pause) yet varied cell-to-cell (no aligned-facet
        // moire). Reuses the same jitter hash as the position offset.
        tempEuler.set(
          gridJitter(x, y, z, 0) * TWO_PI,
          gridJitter(x, y, z, 1) * TWO_PI,
          gridJitter(x, y, z, 2) * TWO_PI,
        );
        tempQuat.setFromEuler(tempEuler);
      } else {
        tempQuat.identity();
      }
      tempMatrix.compose(tempPos, tempQuat, tempScale);
      voxelMesh.setMatrixAt(i, tempMatrix);

      // Every pilgrim descendant (any cell carrying the tracked tag) glows
      // bright magenta in any colour mode, so the whole lineage cloud pops.
      if (trackedTag !== null && originTag === trackedTag) {
        tempColor.setRGB(1, 0, 1);
      } else {
        // Alloc-free: write into the shared `colorOut` buffer instead of
        // allocating an Rgb array per cell (×cellCount per snapshot).
        cellColorInto(colorOut, colorMode, t, appearance, originTag);
        tempColor.setRGB(colorOut[0], colorOut[1], colorOut[2]);
      }
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

    // Tracking: a possessed lineage (Project Pilgrim) is followed as a whole
    // CLOUD — centroid for the camera, bbox cage around every descendant —
    // rather than a single max-energy cell that flickers between fragments.
    // Otherwise the legacy tracker follows the global max-energy cell.
    if (trackedTag !== null) {
      // Lineage mode owns its own visuals; hide the legacy single-cell ones.
      if (highlightMesh) highlightMesh.visible = false;
      if (trailLine) trailLine.visible = false;
      const lin = analyzeLineage(snap, stride, cellCount, trackedTag);
      if (lin) {
        pilgrimSeen = true;
        // Camera follows the energy-weighted centroid (stable); snap on once.
        pilgrimTargetPos = { x: lin.cx, y: lin.cy, z: lin.cz };
        if (pilgrimFollow && lastPilgrimPos === null) {
          recenterCameraOnPilgrim(lin.cx, lin.cy, lin.cz);
        }
        lastPilgrimPos = pilgrimTargetPos;
        updateLineageBox(lin);
        // Inspector tracks the strongest carrier (the torch).
        const off = lin.maxIdx * stride;
        inspector.coord = { x: snap[off]! | 0, y: snap[off + 1]! | 0, z: snap[off + 2]! | 0 };
        inspector.panel.classList.add('visible');
        dom.programStatus.textContent =
          `pilgrim: ${lin.count} buněk, E=${lin.sumEnergy.toLocaleString()} @ ` +
          `(${Math.round(lin.cx)},${Math.round(lin.cy)},${Math.round(lin.cz)})`;
      } else if (pilgrimSeen) {
        // The lineage died out — its tag is gone from the world.
        dom.programStatus.textContent = 'pilgrim ztracen (tag vyhynul)';
        trackedTag = null;
        pilgrimFollow = false;
        pilgrimTargetPos = null;
        if (lineageBox) lineageBox.visible = false;
      }
    } else {
      if (lineageBox) lineageBox.visible = false;
      updateTrackerVisuals(analysis.maxCellIdx, snap, stride);
    }
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
    iMemoryView: requireEl('iMemoryView', HTMLButtonElement),
    iClose: requireEl('iClose', HTMLButtonElement),
    memoryView: 'disasm' as 'disasm' | 'hex',
    lastMsg: null as CellDetailMsg | null,
  };
  inspector.iClose.addEventListener('click', () => {
    inspector.coord = null;
    inspector.lastMsg = null;
    inspector.panel.classList.remove('visible');
  });
  inspector.iMemoryView.addEventListener('click', () => {
    inspector.memoryView = inspector.memoryView === 'disasm' ? 'hex' : 'disasm';
    inspector.iMemoryView.textContent = inspector.memoryView;
    if (inspector.lastMsg) renderInspector(inspector.lastMsg);
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
    inspector.lastMsg = msg;
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
    const pc = data[0]!;
    inspector.iCoord.textContent = `(${x}, ${y}, ${z})`;
    inspector.iTick.textContent = String(msg.tick);
    inspector.iPc.textContent = String(pc);
    inspector.iEnergy.textContent = data[1]!.toLocaleString();
    inspector.iOriginTag.textContent = `0x${data[2]!.toString(16).padStart(8, '0')}`;
    inspector.iAppearance.textContent = `0x${data[3]!.toString(16).padStart(8, '0')}`;
    inspector.iPointers.textContent = fmtDirArr(data.slice(4, 10));
    inspector.iRates.textContent = fmtDirArr(data.slice(10, 16));
    inspector.iActiveOutflow.textContent = fmtDirArr(data.slice(16, 22));
    inspector.iInflow.textContent = fmtDirArr(data.slice(22, 28));
    const memSlots = data.slice(prefix);
    inspector.iMemory.textContent =
      inspector.memoryView === 'disasm'
        ? disassemble(memSlots, { pc })
        : fmtMemoryHexDump(memSlots);
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

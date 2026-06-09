"use strict";

// Prototyp 11 - Gravitace a tlak.
//
// Samostatna JS reimplementace jadra (jako prototypy 01-09), ktera k difuzi
// (radiaci) prida dve souperici sily, vse jako tok pres steny mezi sousedy:
//
//   drive(A->B) =  coeff * (E_A - E_B)            // radiace: z kopce dolu (anizotropni)
//               +  (Pi(E_A) - Pi(E_B))            // tlak: dolu po tlaku, ~ hustota^gamma (izotropni)
//               +  Ggrav * (M_B - M_A)            // gravitace: dolu po -potencialu, k hmote (izotropni)
//   out(A->B)   =  max(0, drive) * dt
//
// Hmota = alpha * E (alpha slozeno do Ggrav). Gravitacni potencial M je
// dlouhodosahovy: M[x] = sum_{0<|d|<=R} E[x+d] / |d|  (jadro 1/r => sila ~ 1/r^2).
// Vsechny odtoky bunky se proporcionalne orezou na jeji energii -> nikdy
// nejde do zaporu a tok je konzervativni. Hranice je void (E=0): energie,
// ktera vytece ven, je ztracena (= "rozredeni do voidu").

const elements = {
  worldCanvas: document.querySelector("#worldCanvas"),
  graphCanvas: document.querySelector("#graphCanvas"),
  stepValue: document.querySelector("#stepValue"),
  phaseValue: document.querySelector("#phaseValue"),
  runButton: document.querySelector("#runButton"),
  stepButton: document.querySelector("#stepButton"),
  resetButton: document.querySelector("#resetButton"),
  presetInput: document.querySelector("#presetInput"),
  sizeInput: document.querySelector("#sizeInput"),
  energyInput: document.querySelector("#energyInput"),
  coeffInput: document.querySelector("#coeffInput"),
  coeffValue: document.querySelector("#coeffValue"),
  gravInput: document.querySelector("#gravInput"),
  gravValue: document.querySelector("#gravValue"),
  radiusInput: document.querySelector("#radiusInput"),
  radiusValue: document.querySelector("#radiusValue"),
  pressureInput: document.querySelector("#pressureInput"),
  pressureValue: document.querySelector("#pressureValue"),
  gammaInput: document.querySelector("#gammaInput"),
  gammaValue: document.querySelector("#gammaValue"),
  dtInput: document.querySelector("#dtInput"),
  dtValue: document.querySelector("#dtValue"),
  noiseInput: document.querySelector("#noiseInput"),
  noiseValue: document.querySelector("#noiseValue"),
  stepsPerFrameInput: document.querySelector("#stepsPerFrameInput"),
  scaleInput: document.querySelector("#scaleInput"),
  sliceInput: document.querySelector("#sliceInput"),
  sliceValue: document.querySelector("#sliceValue"),
  totalMetric: document.querySelector("#totalMetric"),
  leakedMetric: document.querySelector("#leakedMetric"),
  maxMetric: document.querySelector("#maxMetric"),
  meanMetric: document.querySelector("#meanMetric"),
  clumpsMetric: document.querySelector("#clumpsMetric"),
  imbalanceMetric: document.querySelector("#imbalanceMetric"),
  axisInputs: Array.from(document.querySelectorAll("input[name='axis']")),
};

const worldContext = elements.worldCanvas.getContext("2d");
const graphContext = elements.graphCanvas.getContext("2d");

const state = {
  size: 32,
  totalEnergy: 1_000_000,
  initialEnergy: 1_000_000,
  preset: "bigbang",
  coeff: 0.08, // radiace (difuze)
  grav: 0.03, // sila gravitace
  radius: 3, // dosah gravitace (cutoff, v bunkach)
  pressure: 0.5, // sila tlaku
  gamma: 2.0, // exponent stavove rovnice tlaku
  dt: 0.4, // casovy krok
  noise: 0.02, // symetrii lamajici sum (analog stochastic_floor)
  step: 0,
  running: false,
  axis: "z",
  slice: 16,
  colorScale: "auto",
  leaked: 0,
  eref: 1, // referencni hustota = pocatecni prumer
  current: new Float64Array(0),
  next: new Float64Array(0),
  potential: new Float64Array(0),
  offsets: [], // {di, dj, dk, w} pro gravitacni jadro
  imageData: null,
  history: [],
  latestMetrics: null,
  rngState: 0x9e3779b9,
};

function cellIndex(x, y, z, size) {
  return x + size * (y + size * z);
}

// Deterministicky RNG (mulberry32) - aby byl prubeh reprodukovatelny.
function nextRandom() {
  let t = (state.rngState += 0x6d2b79f5);
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

// Predpocet offsetu gravitacniho jadra v kouli polomeru R, vaha 1/|d|.
function buildOffsets(radius) {
  const offsets = [];
  const r2 = radius * radius;
  for (let dk = -radius; dk <= radius; dk += 1) {
    for (let dj = -radius; dj <= radius; dj += 1) {
      for (let di = -radius; di <= radius; di += 1) {
        if (di === 0 && dj === 0 && dk === 0) continue;
        const dist2 = di * di + dj * dj + dk * dk;
        if (dist2 > r2) continue;
        offsets.push({ di, dj, dk, w: 1 / Math.sqrt(dist2) });
      }
    }
  }
  return offsets;
}

function resetWorld() {
  state.size = Number(elements.sizeInput.value);
  state.initialEnergy = Math.max(1, Number(elements.energyInput.value) || 1);
  state.totalEnergy = state.initialEnergy;
  state.preset = elements.presetInput.value;
  state.coeff = Number(elements.coeffInput.value);
  state.grav = Number(elements.gravInput.value);
  state.radius = Number(elements.radiusInput.value);
  state.pressure = Number(elements.pressureInput.value);
  state.gamma = Number(elements.gammaInput.value);
  state.dt = Number(elements.dtInput.value);
  state.noise = Number(elements.noiseInput.value);
  state.colorScale = elements.scaleInput.value;
  state.step = 0;
  state.running = false;
  state.leaked = 0;
  state.history = [];
  state.rngState = 0x9e3779b9;

  const size = state.size;
  const cellCount = size * size * size;
  state.current = new Float64Array(cellCount);
  state.next = new Float64Array(cellCount);
  state.potential = new Float64Array(cellCount);
  state.offsets = buildOffsets(state.radius);
  state.eref = state.initialEnergy / cellCount;
  state.imageData = worldContext.createImageData(size, size);

  seedWorld();

  const center = Math.floor(size / 2);
  state.slice = center;
  elements.sliceInput.max = String(size - 1);
  elements.sliceInput.value = String(center);
  elements.sliceValue.value = String(center);
  elements.stepValue.textContent = "0";
  elements.runButton.textContent = "Spustit";
  elements.runButton.classList.remove("primary");

  updateMetrics();
  render();
}

function seedWorld() {
  const size = state.size;
  const current = state.current;
  const center = Math.floor(size / 2);

  if (state.preset === "bigbang") {
    // Prvotni entita: vsechna energie v jedne bunce -> maximalni hustota.
    current[cellIndex(center, center, center, size)] = state.initialEnergy;
  } else if (state.preset === "noise") {
    // Nahodne pole -> Jeansova nestabilita: kde se sum nakupi, gravitace zacne.
    let sum = 0;
    for (let i = 0; i < current.length; i += 1) {
      const v = nextRandom();
      current[i] = v;
      sum += v;
    }
    const scale = state.initialEnergy / sum;
    for (let i = 0; i < current.length; i += 1) current[i] *= scale;
  } else if (state.preset === "two-bodies") {
    // Dve telesa - sledovat, jestli se k sobe pritahnou (akrece pres void).
    const off = Math.max(2, Math.floor(size / 5));
    current[cellIndex(center - off, center, center, size)] = state.initialEnergy / 2;
    current[cellIndex(center + off, center, center, size)] = state.initialEnergy / 2;
  }
}

// Tlakovy potencial: Pi(E) = pressure * eref * (E/eref)^gamma.
// Strmy v hustote (gamma>1) -> pri extremni hustote pretlaci gravitaci a
// zastavi kolaps; pri prumerne hustote je to jen mirna korekce.
function pressurePotential(energy) {
  if (energy <= 0) return 0;
  const eref = state.eref;
  return state.pressure * eref * Math.pow(energy / eref, state.gamma);
}

function computePotential() {
  const size = state.size;
  const size2 = size * size;
  const current = state.current;
  const potential = state.potential;
  const offsets = state.offsets;
  const offsetCount = offsets.length;

  for (let z = 0; z < size; z += 1) {
    for (let y = 0; y < size; y += 1) {
      for (let x = 0; x < size; x += 1) {
        let acc = 0;
        for (let o = 0; o < offsetCount; o += 1) {
          const off = offsets[o];
          const nx = x + off.di;
          const ny = y + off.dj;
          const nz = z + off.dk;
          if (nx < 0 || nx >= size || ny < 0 || ny >= size || nz < 0 || nz >= size) {
            continue; // void neprispiva
          }
          acc += current[nx + size * ny + size2 * nz] * off.w;
        }
        potential[x + size * y + size2 * z] = acc;
      }
    }
  }
}

// Smer A->B: kladny drive = cista snaha presunout energii z A do B.
function faceDrive(energyA, energyB, potA, potB) {
  return (
    state.coeff * (energyA - energyB) + // radiace ven (k nizsi E)
    (pressurePotential(energyA) - pressurePotential(energyB)) + // tlak ven
    state.grav * (potB - potA) // gravitace k hmote (k vyssimu potencialu)
  );
}

function simulateStep() {
  const size = state.size;
  const size2 = size * size;
  const current = state.current;
  const next = state.next;
  const potential = state.potential;
  const dt = state.dt;
  const noise = state.noise;

  computePotential();
  next.set(current);

  // Smery k sesti sousedum: index sousedni bunky + zda je za hranici (void).
  for (let z = 0; z < size; z += 1) {
    for (let y = 0; y < size; y += 1) {
      for (let x = 0; x < size; x += 1) {
        const index = x + size * y + size2 * z;
        const energyA = current[index];
        if (energyA <= 0) continue;
        const potA = potential[index];

        // Spocti pozadovany odtok do kazdeho z 6 smeru (void = E,pot 0).
        let out0 = 0, out1 = 0, out2 = 0, out3 = 0, out4 = 0, out5 = 0;
        let total = 0;

        // -x, +x, -y, +y, -z, +z
        const nbr = [
          x > 0 ? index - 1 : -1,
          x < size - 1 ? index + 1 : -1,
          y > 0 ? index - size : -1,
          y < size - 1 ? index + size : -1,
          z > 0 ? index - size2 : -1,
          z < size - 1 ? index + size2 : -1,
        ];

        for (let d = 0; d < 6; d += 1) {
          const ni = nbr[d];
          const energyB = ni >= 0 ? current[ni] : 0;
          const potB = ni >= 0 ? potential[ni] : 0;
          let drive = faceDrive(energyA, energyB, potA, potB);
          if (noise > 0) drive *= 1 + noise * (nextRandom() - 0.5);
          let out = drive > 0 ? drive * dt : 0;
          if (d === 0) out0 = out;
          else if (d === 1) out1 = out;
          else if (d === 2) out2 = out;
          else if (d === 3) out3 = out;
          else if (d === 4) out4 = out;
          else out5 = out;
          total += out;
        }

        if (total <= 0) continue;
        // Proporcionalni orez: nikdy neodtece vic nez ma bunka k dispozici.
        if (total > energyA) {
          const s = energyA / total;
          out0 *= s; out1 *= s; out2 *= s; out3 *= s; out4 *= s; out5 *= s;
          total = energyA;
        }

        next[index] -= total;
        const outs = [out0, out1, out2, out3, out4, out5];
        for (let d = 0; d < 6; d += 1) {
          const ni = nbr[d];
          if (ni >= 0) next[ni] += outs[d];
          else state.leaked += outs[d]; // odtok do voidu = ztrata
        }
      }
    }
  }

  const previous = state.current;
  state.current = state.next;
  state.next = previous;
  state.step += 1;
}

function updateMetrics() {
  const values = state.current;
  const eref = state.eref;
  const clumpThreshold = eref * 4; // bunka je "shluk", kdyz >4x prumerna hustota
  let sum = 0;
  let max = 0;
  let clumps = 0;

  for (let i = 0; i < values.length; i += 1) {
    const value = values[i];
    sum += value;
    if (value > max) max = value;
    if (value > clumpThreshold) clumps += 1;
  }

  const mean = sum / values.length;
  let squaredDeviationSum = 0;
  for (let i = 0; i < values.length; i += 1) {
    const deviation = values[i] - mean;
    squaredDeviationSum += deviation * deviation;
  }
  const variance = squaredDeviationSum / values.length;
  const imbalance = mean > 0 ? Math.sqrt(variance) / mean : 0;

  state.latestMetrics = { sum, max, mean, clumps, imbalance };
  state.history.push({ max, clumps, imbalance });
  if (state.history.length > 240) state.history.shift();

  const phase = classifyPhase();

  elements.stepValue.textContent = formatInteger(state.step);
  elements.phaseValue.textContent = phase;
  elements.totalMetric.textContent = formatNumber(sum);
  elements.leakedMetric.textContent = formatNumber(state.leaked);
  elements.maxMetric.textContent = formatNumber(max);
  elements.meanMetric.textContent = formatNumber(mean);
  elements.clumpsMetric.textContent = formatInteger(clumps);
  elements.imbalanceMetric.textContent = formatRatio(imbalance);
}

// Hruba klasifikace rezimu z trendu maxima a poctu shluku.
function classifyPhase() {
  const h = state.history;
  if (h.length < 6) return "-";
  const now = h[h.length - 1];
  const past = h[h.length - 6];
  const maxRising = now.max > past.max * 1.02;
  const maxFalling = now.max < past.max * 0.98;

  if (now.clumps === 0) {
    return maxFalling ? "Rozpinani / redeni" : "Difuze";
  }
  if (maxRising) return "Gravitacni kolaps";
  if (maxFalling) return "Rozpinani se shluky";
  return "Kvazi-rovnovaha";
}

function render() {
  drawWorldSlice();
  drawGraph();
}

function drawWorldSlice() {
  const size = state.size;
  const imageData = state.imageData;
  const data = imageData.data;

  let pixel = 0;
  for (let row = 0; row < size; row += 1) {
    for (let col = 0; col < size; col += 1) {
      const energy = readSliceEnergy(col, row);
      const normalized = normalizeEnergy(energy);
      const color = heatColor(normalized);
      data[pixel] = color[0];
      data[pixel + 1] = color[1];
      data[pixel + 2] = color[2];
      data[pixel + 3] = 255;
      pixel += 4;
    }
  }

  const offscreen = document.createElement("canvas");
  offscreen.width = size;
  offscreen.height = size;
  offscreen.getContext("2d").putImageData(imageData, 0, 0);

  worldContext.imageSmoothingEnabled = false;
  worldContext.clearRect(0, 0, elements.worldCanvas.width, elements.worldCanvas.height);
  worldContext.drawImage(offscreen, 0, 0, elements.worldCanvas.width, elements.worldCanvas.height);
}

function normalizeEnergy(energy) {
  const value = Math.max(0, energy);
  if (state.colorScale === "mean") {
    const mean = state.latestMetrics ? state.latestMetrics.mean : state.eref;
    return value / Math.max(1e-12, mean * 2);
  }
  if (state.colorScale === "initial") {
    return Math.log1p(value) / Math.log1p(Math.max(1, state.initialEnergy));
  }
  const max = state.latestMetrics ? state.latestMetrics.max : 1;
  return Math.log1p(value) / Math.log1p(Math.max(1, max));
}

function readSliceEnergy(col, row) {
  const size = state.size;
  if (state.axis === "z") return state.current[cellIndex(col, row, state.slice, size)];
  if (state.axis === "y") return state.current[cellIndex(col, state.slice, row, size)];
  return state.current[cellIndex(state.slice, col, row, size)];
}

function heatColor(value) {
  const t = clamp(value, 0, 1);
  if (t < 0.25) return interpolateColor([5, 7, 10], [18, 60, 105], t / 0.25);
  if (t < 0.5) return interpolateColor([18, 60, 105], [31, 182, 166], (t - 0.25) / 0.25);
  if (t < 0.75) return interpolateColor([31, 182, 166], [240, 199, 93], (t - 0.5) / 0.25);
  if (t < 0.93) return interpolateColor([240, 199, 93], [239, 106, 84], (t - 0.75) / 0.18);
  return interpolateColor([239, 106, 84], [247, 244, 220], (t - 0.93) / 0.07);
}

function interpolateColor(a, b, t) {
  const clamped = clamp(t, 0, 1);
  return [
    Math.round(a[0] + (b[0] - a[0]) * clamped),
    Math.round(a[1] + (b[1] - a[1]) * clamped),
    Math.round(a[2] + (b[2] - a[2]) * clamped),
  ];
}

function drawGraph() {
  const canvas = elements.graphCanvas;
  const context = graphContext;
  const width = canvas.width;
  const height = canvas.height;
  const history = state.history;

  context.clearRect(0, 0, width, height);
  context.fillStyle = "#0b0f15";
  context.fillRect(0, 0, width, height);

  context.strokeStyle = "#253040";
  context.lineWidth = 1;
  for (let y = 24; y < height; y += 24) {
    context.beginPath();
    context.moveTo(0, y);
    context.lineTo(width, y);
    context.stroke();
  }

  if (history.length < 2) return;

  drawHistoryLine(context, history, "max", "#f0c75d", width, height);
  drawHistoryLine(context, history, "clumps", "#ef6a54", width, height);
  drawHistoryLine(context, history, "imbalance", "#31c6b2", width, height);

  context.font = "12px system-ui, sans-serif";
  context.fillStyle = "#f0c75d";
  context.fillText("max", 10, 18);
  context.fillStyle = "#ef6a54";
  context.fillText("shluky", 48, 18);
  context.fillStyle = "#31c6b2";
  context.fillText("nerovnovaha", 104, 18);
}

function drawHistoryLine(context, history, key, color, width, height) {
  const maxValue = history.reduce((max, item) => Math.max(max, item[key]), 0);
  const scale = maxValue > 0 ? maxValue : 1;
  context.strokeStyle = color;
  context.lineWidth = 2;
  context.beginPath();
  history.forEach((item, index) => {
    const x = (index / Math.max(1, history.length - 1)) * (width - 1);
    const y = height - 8 - (item[key] / scale) * (height - 24);
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  context.stroke();
}

function animationLoop() {
  if (!state.running) return;
  const stepsPerFrame = clamp(Number(elements.stepsPerFrameInput.value) || 1, 1, 200);
  for (let i = 0; i < stepsPerFrame; i += 1) simulateStep();
  updateMetrics();
  render();
  requestAnimationFrame(animationLoop);
}

function toggleRun() {
  state.running = !state.running;
  elements.runButton.textContent = state.running ? "Pauza" : "Spustit";
  elements.runButton.classList.toggle("primary", state.running);
  if (state.running) requestAnimationFrame(animationLoop);
}

function stepOnce() {
  if (state.running) toggleRun();
  simulateStep();
  updateMetrics();
  render();
}

function setAxis(axis) {
  state.axis = axis;
  render();
}

function setSlice(slice) {
  state.slice = clamp(Number(slice), 0, state.size - 1);
  elements.sliceValue.value = String(state.slice);
  render();
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function formatNumber(value) {
  const abs = Math.abs(value);
  if (abs > 0 && abs < 0.0001) return value.toExponential(2);
  if (abs >= 1_000_000) return value.toExponential(4);
  if (abs >= 1000) return value.toFixed(2);
  if (abs >= 1) return value.toFixed(4);
  return value.toFixed(6);
}

function formatRatio(value) {
  if (value >= 100) return value.toExponential(3);
  if (value >= 1) return value.toFixed(4);
  if (value >= 0.01) return value.toFixed(5);
  return value.toExponential(3);
}

function formatInteger(value) {
  return new Intl.NumberFormat("cs-CZ").format(value);
}

// Slider -> live update bez resetu (krome tech, co meni velikost pole).
function bindLiveSlider(input, output, key, decimals) {
  input.addEventListener("input", () => {
    state[key] = Number(input.value);
    output.value = state[key].toFixed(decimals);
  });
}

elements.runButton.addEventListener("click", toggleRun);
elements.stepButton.addEventListener("click", stepOnce);
elements.resetButton.addEventListener("click", resetWorld);

elements.presetInput.addEventListener("change", resetWorld);
elements.sizeInput.addEventListener("change", resetWorld);
elements.energyInput.addEventListener("change", resetWorld);
elements.radiusInput.addEventListener("change", () => {
  state.radius = Number(elements.radiusInput.value);
  elements.radiusValue.value = String(state.radius);
  state.offsets = buildOffsets(state.radius); // jadro je treba prepocitat
});

bindLiveSlider(elements.coeffInput, elements.coeffValue, "coeff", 3);
bindLiveSlider(elements.gravInput, elements.gravValue, "grav", 3);
bindLiveSlider(elements.pressureInput, elements.pressureValue, "pressure", 2);
bindLiveSlider(elements.gammaInput, elements.gammaValue, "gamma", 1);
bindLiveSlider(elements.dtInput, elements.dtValue, "dt", 2);
bindLiveSlider(elements.noiseInput, elements.noiseValue, "noise", 3);

elements.scaleInput.addEventListener("change", () => {
  state.colorScale = elements.scaleInput.value;
  render();
});

elements.sliceInput.addEventListener("input", () => setSlice(elements.sliceInput.value));

elements.axisInputs.forEach((input) => {
  input.addEventListener("change", () => {
    if (input.checked) setAxis(input.value);
  });
});

resetWorld();

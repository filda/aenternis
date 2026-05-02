"use strict";

const elements = {
  worldCanvas: document.querySelector("#worldCanvas"),
  graphCanvas: document.querySelector("#graphCanvas"),
  stepValue: document.querySelector("#stepValue"),
  runButton: document.querySelector("#runButton"),
  stepButton: document.querySelector("#stepButton"),
  resetButton: document.querySelector("#resetButton"),
  sizeInput: document.querySelector("#sizeInput"),
  energyInput: document.querySelector("#energyInput"),
  rateInput: document.querySelector("#rateInput"),
  rateValue: document.querySelector("#rateValue"),
  stepsPerFrameInput: document.querySelector("#stepsPerFrameInput"),
  equilibriumInput: document.querySelector("#equilibriumInput"),
  scaleInput: document.querySelector("#scaleInput"),
  sliceInput: document.querySelector("#sliceInput"),
  sliceValue: document.querySelector("#sliceValue"),
  totalMetric: document.querySelector("#totalMetric"),
  driftMetric: document.querySelector("#driftMetric"),
  maxMetric: document.querySelector("#maxMetric"),
  minMetric: document.querySelector("#minMetric"),
  meanMetric: document.querySelector("#meanMetric"),
  varianceMetric: document.querySelector("#varianceMetric"),
  imbalanceMetric: document.querySelector("#imbalanceMetric"),
  cooledMetric: document.querySelector("#cooledMetric"),
  axisInputs: Array.from(document.querySelectorAll("input[name='axis']")),
};

const worldContext = elements.worldCanvas.getContext("2d");
const graphContext = elements.graphCanvas.getContext("2d");

const state = {
  size: 32,
  totalEnergy: 1_000_000,
  initialEnergy: 1_000_000,
  diffusionRate: 0.08,
  step: 0,
  running: false,
  axis: "z",
  slice: 16,
  colorScale: "mean",
  equilibriumThreshold: 0.01,
  cooledAt: null,
  current: new Float64Array(0),
  next: new Float64Array(0),
  imageData: null,
  history: [],
  latestMetrics: null,
};

function cellIndex(x, y, z, size) {
  return x + size * (y + size * z);
}

function resetWorld() {
  state.size = Number(elements.sizeInput.value);
  state.initialEnergy = Math.max(1, Number(elements.energyInput.value) || 1);
  state.totalEnergy = state.initialEnergy;
  state.diffusionRate = Number(elements.rateInput.value);
  state.equilibriumThreshold = readEquilibriumThreshold();
  state.colorScale = elements.scaleInput.value;
  state.step = 0;
  state.running = false;
  state.cooledAt = null;
  state.history = [];

  const cellCount = state.size * state.size * state.size;
  state.current = new Float64Array(cellCount);
  state.next = new Float64Array(cellCount);
  state.imageData = worldContext.createImageData(state.size, state.size);

  const center = Math.floor(state.size / 2);
  state.current[cellIndex(center, center, center, state.size)] = state.initialEnergy;
  state.slice = center;

  elements.sliceInput.max = String(state.size - 1);
  elements.sliceInput.value = String(state.slice);
  elements.sliceValue.value = String(state.slice);
  elements.stepValue.textContent = "0";
  elements.runButton.textContent = "Spustit";
  elements.runButton.classList.remove("primary");

  updateMetrics();
  render();
}

function simulateStep() {
  const size = state.size;
  const size2 = size * size;
  const rate = state.diffusionRate;
  const current = state.current;
  const next = state.next;

  for (let z = 0; z < size; z += 1) {
    const zOffset = z * size2;
    const zpOffset = ((z + 1) % size) * size2;
    const znOffset = ((z + size - 1) % size) * size2;

    for (let y = 0; y < size; y += 1) {
      const rowOffset = zOffset + y * size;
      const ypOffset = zOffset + ((y + 1) % size) * size;
      const ynOffset = zOffset + ((y + size - 1) % size) * size;

      for (let x = 0; x < size; x += 1) {
        const index = rowOffset + x;
        const xp = rowOffset + ((x + 1) % size);
        const xn = rowOffset + ((x + size - 1) % size);
        const yp = ypOffset + x;
        const yn = ynOffset + x;
        const zp = zpOffset + y * size + x;
        const zn = znOffset + y * size + x;

        const energy = current[index];
        const neighborSum = current[xp] + current[xn] + current[yp] + current[yn] + current[zp] + current[zn];
        next[index] = energy + rate * (neighborSum - 6 * energy);
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
  let sum = 0;
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;

  for (let i = 0; i < values.length; i += 1) {
    const value = values[i];
    sum += value;
    if (value < min) min = value;
    if (value > max) max = value;
  }

  const mean = sum / values.length;
  let squaredDeviationSum = 0;
  for (let i = 0; i < values.length; i += 1) {
    const deviation = values[i] - mean;
    squaredDeviationSum += deviation * deviation;
  }

  const variance = squaredDeviationSum / values.length;
  const standardDeviation = Math.sqrt(variance);
  const imbalance = mean > 0 ? standardDeviation / mean : 0;
  const drift = sum - state.initialEnergy;
  state.latestMetrics = { sum, min, max, mean, variance, imbalance, drift };
  state.history.push({ max, variance, imbalance });

  if (state.cooledAt === null && state.step > 0 && imbalance <= state.equilibriumThreshold) {
    state.cooledAt = state.step;
  }

  if (state.history.length > 240) {
    state.history.shift();
  }

  elements.stepValue.textContent = formatInteger(state.step);
  elements.totalMetric.textContent = formatNumber(sum);
  elements.driftMetric.textContent = formatNumber(drift);
  elements.maxMetric.textContent = formatNumber(max);
  elements.minMetric.textContent = formatNumber(min);
  elements.meanMetric.textContent = formatNumber(mean);
  elements.varianceMetric.textContent = formatNumber(variance);
  elements.imbalanceMetric.textContent = formatRatio(imbalance);
  elements.cooledMetric.textContent = state.cooledAt === null ? "-" : formatInteger(state.cooledAt);
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

  if (state.colorScale === "auto") {
    const max = state.latestMetrics ? state.latestMetrics.max : 1;
    return Math.log1p(value) / Math.log1p(Math.max(1, max));
  }

  if (state.colorScale === "initial") {
    return Math.log1p(value) / Math.log1p(Math.max(1, state.initialEnergy));
  }

  const mean = state.latestMetrics ? state.latestMetrics.mean : state.initialEnergy / state.current.length;
  return value / Math.max(1e-12, mean * 2);
}

function readSliceEnergy(col, row) {
  const size = state.size;
  if (state.axis === "z") {
    return state.current[cellIndex(col, row, state.slice, size)];
  }
  if (state.axis === "y") {
    return state.current[cellIndex(col, state.slice, row, size)];
  }
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
  drawHistoryLine(context, history, "imbalance", "#31c6b2", width, height);

  context.fillStyle = "#a9b3c1";
  context.font = "12px system-ui, sans-serif";
  context.fillText("max", 10, 18);
  context.fillStyle = "#31c6b2";
  context.fillText("nerovnovaha", 48, 18);
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
    if (index === 0) {
      context.moveTo(x, y);
    } else {
      context.lineTo(x, y);
    }
  });

  context.stroke();
}

function animationLoop() {
  if (!state.running) return;

  const stepsPerFrame = clamp(Number(elements.stepsPerFrameInput.value) || 1, 1, 200);
  for (let i = 0; i < stepsPerFrame; i += 1) {
    simulateStep();
  }

  updateMetrics();
  render();
  requestAnimationFrame(animationLoop);
}

function toggleRun() {
  state.running = !state.running;
  elements.runButton.textContent = state.running ? "Pauza" : "Spustit";
  elements.runButton.classList.toggle("primary", state.running);
  if (state.running) {
    requestAnimationFrame(animationLoop);
  }
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

function readEquilibriumThreshold() {
  const value = Number(elements.equilibriumInput.value);
  if (!Number.isFinite(value) || value < 0) return 0.01;
  return value;
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

elements.runButton.addEventListener("click", toggleRun);
elements.stepButton.addEventListener("click", stepOnce);
elements.resetButton.addEventListener("click", resetWorld);

elements.sizeInput.addEventListener("change", resetWorld);
elements.energyInput.addEventListener("change", resetWorld);

elements.rateInput.addEventListener("input", () => {
  state.diffusionRate = Number(elements.rateInput.value);
  elements.rateValue.value = state.diffusionRate.toFixed(3);
});

elements.equilibriumInput.addEventListener("change", () => {
  state.equilibriumThreshold = readEquilibriumThreshold();
  state.cooledAt = null;
  updateMetrics();
  render();
});

elements.scaleInput.addEventListener("change", () => {
  state.colorScale = elements.scaleInput.value;
  render();
});

elements.sliceInput.addEventListener("input", () => {
  setSlice(elements.sliceInput.value);
});

elements.axisInputs.forEach((input) => {
  input.addEventListener("change", () => {
    if (input.checked) setAxis(input.value);
  });
});

resetWorld();

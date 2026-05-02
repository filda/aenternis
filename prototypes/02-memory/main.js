"use strict";

const elements = {
  memoryCanvas: document.querySelector("#memoryCanvas"),
  graphCanvas: document.querySelector("#graphCanvas"),
  stepValue: document.querySelector("#stepValue"),
  runButton: document.querySelector("#runButton"),
  stepButton: document.querySelector("#stepButton"),
  resetButton: document.querySelector("#resetButton"),
  energyInput: document.querySelector("#energyInput"),
  energyPerCellInput: document.querySelector("#energyPerCellInput"),
  leakInput: document.querySelector("#leakInput"),
  leakValue: document.querySelector("#leakValue"),
  energyPulseInput: document.querySelector("#energyPulseInput"),
  loseEnergyButton: document.querySelector("#loseEnergyButton"),
  gainEnergyButton: document.querySelector("#gainEnergyButton"),
  refreshNowButton: document.querySelector("#refreshNowButton"),
  maxCellsInput: document.querySelector("#maxCellsInput"),
  patternInput: document.querySelector("#patternInput"),
  refreshEnabledInput: document.querySelector("#refreshEnabledInput"),
  coreSizeInput: document.querySelector("#coreSizeInput"),
  refreshIntervalInput: document.querySelector("#refreshIntervalInput"),
  halfLifeInput: document.querySelector("#halfLifeInput"),
  maxMutationInput: document.querySelector("#maxMutationInput"),
  maxMutationValue: document.querySelector("#maxMutationValue"),
  stepsPerFrameInput: document.querySelector("#stepsPerFrameInput"),
  seedInput: document.querySelector("#seedInput"),
  energyMetric: document.querySelector("#energyMetric"),
  memoryMetric: document.querySelector("#memoryMetric"),
  lostMetric: document.querySelector("#lostMetric"),
  mutationMetric: document.querySelector("#mutationMetric"),
  lastMutationMetric: document.querySelector("#lastMutationMetric"),
  averageAgeMetric: document.querySelector("#averageAgeMetric"),
  riskyMetric: document.querySelector("#riskyMetric"),
  refreshMetric: document.querySelector("#refreshMetric"),
  selectedAddress: document.querySelector("#selectedAddress"),
  selectedValue: document.querySelector("#selectedValue"),
  viewModeInputs: Array.from(document.querySelectorAll("input[name='viewMode']")),
};

const memoryContext = elements.memoryCanvas.getContext("2d");
const graphContext = elements.graphCanvas.getContext("2d");

const state = {
  step: 0,
  running: false,
  energy: 512,
  energyPerCell: 1,
  leakRate: 0.001,
  maxCells: 512,
  liveCells: 512,
  lostCells: 0,
  totalMutations: 0,
  lastMutations: 0,
  refreshWrites: 0,
  viewMode: "age",
  hoveredCell: -1,
  values: new Uint8Array(0),
  ages: new Float64Array(0),
  history: [],
  rng: mulberry32(12345),
};

function resetEntity() {
  state.step = 0;
  state.running = false;
  state.energy = Math.max(0, Number(elements.energyInput.value) || 0);
  state.energyPerCell = Math.max(0.01, Number(elements.energyPerCellInput.value) || 1);
  state.leakRate = Number(elements.leakInput.value);
  state.maxCells = Number(elements.maxCellsInput.value);
  state.liveCells = 0;
  state.lostCells = 0;
  state.totalMutations = 0;
  state.lastMutations = 0;
  state.refreshWrites = 0;
  state.history = [];
  state.rng = mulberry32(Number(elements.seedInput.value) || 1);
  state.values = new Uint8Array(state.maxCells);
  state.ages = new Float64Array(state.maxCells);

  fillInitialMemory();
  resizeMemoryFromEnergy();

  elements.stepValue.textContent = "0";
  elements.runButton.textContent = "Spustit";
  elements.runButton.classList.remove("primary");
  elements.leakValue.value = state.leakRate.toFixed(4);
  elements.maxMutationValue.value = Number(elements.maxMutationInput.value).toFixed(4);

  updateMetrics();
  render();
}

function fillInitialMemory() {
  const pattern = elements.patternInput.value;

  for (let i = 0; i < state.maxCells; i += 1) {
    if (pattern === "zero") {
      state.values[i] = 0;
    } else if (pattern === "ramp") {
      state.values[i] = i & 255;
    } else if (pattern === "core") {
      state.values[i] = i < 64 ? coreProgramByte(i) : randomByte();
    } else {
      state.values[i] = randomByte();
    }
    state.ages[i] = Math.floor(state.rng() * 24);
  }
}

function coreProgramByte(index) {
  const pattern = [0x10, 0x20, 0x20, 0x30, 0x40, 0x30, 0x20, 0x50];
  return pattern[index % pattern.length];
}

function simulateStep() {
  state.step += 1;
  state.energy = Math.max(0, state.energy * (1 - state.leakRate));
  resizeMemoryFromEnergy();
  ageLiveCells();
  maybeRefreshCore();
  mutateOldCells();
}

function resizeMemoryFromEnergy() {
  const previousLiveCells = state.liveCells;
  const nextLiveCells = clamp(Math.floor(state.energy / state.energyPerCell), 0, state.maxCells);
  state.liveCells = nextLiveCells;

  if (nextLiveCells < previousLiveCells) {
    state.lostCells += previousLiveCells - nextLiveCells;
    for (let i = nextLiveCells; i < previousLiveCells; i += 1) {
      state.ages[i] = 0;
    }
  } else if (nextLiveCells > previousLiveCells) {
    for (let i = previousLiveCells; i < nextLiveCells; i += 1) {
      state.values[i] = 0;
      state.ages[i] = 0;
    }
  }
}

function ageLiveCells() {
  for (let i = 0; i < state.liveCells; i += 1) {
    state.ages[i] += 1;
  }
}

function maybeRefreshCore() {
  if (!elements.refreshEnabledInput.checked) return;

  const interval = Math.max(1, Math.floor(Number(elements.refreshIntervalInput.value) || 1));
  if (state.step % interval !== 0) return;
  refreshCore();
}

function refreshCore() {
  const count = clamp(Math.floor(Number(elements.coreSizeInput.value) || 0), 0, state.liveCells);
  for (let i = 0; i < count; i += 1) {
    state.ages[i] = 0;
  }
  state.refreshWrites += count;
}

function mutateOldCells() {
  const halfLife = Math.max(1, Number(elements.halfLifeInput.value) || 1);
  const maxMutation = Number(elements.maxMutationInput.value);
  let mutations = 0;

  for (let i = 0; i < state.liveCells; i += 1) {
    const probability = mutationProbability(state.ages[i], halfLife, maxMutation);
    if (state.rng() < probability) {
      const bit = 1 << Math.floor(state.rng() * 8);
      state.values[i] ^= bit;
      state.ages[i] = 0;
      mutations += 1;
    }
  }

  state.lastMutations = mutations;
  state.totalMutations += mutations;
}

function mutationProbability(age, halfLife, maxMutation) {
  return maxMutation * (1 - 2 ** (-age / halfLife));
}

function applyEnergyPulse(delta) {
  state.energy = Math.max(0, state.energy + delta);
  resizeMemoryFromEnergy();
  updateMetrics();
  render();
}

function updateMetrics() {
  let ageSum = 0;
  let risky = 0;
  const halfLife = Math.max(1, Number(elements.halfLifeInput.value) || 1);
  const maxMutation = Number(elements.maxMutationInput.value);
  const riskyLimit = maxMutation * 0.5;

  for (let i = 0; i < state.liveCells; i += 1) {
    ageSum += state.ages[i];
    if (mutationProbability(state.ages[i], halfLife, maxMutation) >= riskyLimit && maxMutation > 0) {
      risky += 1;
    }
  }

  const averageAge = state.liveCells > 0 ? ageSum / state.liveCells : 0;
  state.history.push({
    energy: state.energy,
    memory: state.liveCells,
    mutations: state.totalMutations,
  });

  if (state.history.length > 240) {
    state.history.shift();
  }

  elements.stepValue.textContent = formatInteger(state.step);
  elements.energyMetric.textContent = formatNumber(state.energy);
  elements.memoryMetric.textContent = `${formatInteger(state.liveCells)} / ${formatInteger(state.maxCells)}`;
  elements.lostMetric.textContent = formatInteger(state.lostCells);
  elements.mutationMetric.textContent = formatInteger(state.totalMutations);
  elements.lastMutationMetric.textContent = formatInteger(state.lastMutations);
  elements.averageAgeMetric.textContent = formatNumber(averageAge);
  elements.riskyMetric.textContent = formatInteger(risky);
  elements.refreshMetric.textContent = formatInteger(state.refreshWrites);
}

function render() {
  drawMemory();
  drawGraph();
  updateSelectedCell();
}

function drawMemory() {
  const cols = getGridColumns();
  const rows = Math.ceil(state.maxCells / cols);
  const imageData = memoryContext.createImageData(cols, rows);
  const data = imageData.data;
  const halfLife = Math.max(1, Number(elements.halfLifeInput.value) || 1);
  const maxMutation = Number(elements.maxMutationInput.value);

  let pixel = 0;
  for (let i = 0; i < cols * rows; i += 1) {
    const color = memoryCellColor(i, halfLife, maxMutation);
    data[pixel] = color[0];
    data[pixel + 1] = color[1];
    data[pixel + 2] = color[2];
    data[pixel + 3] = 255;
    pixel += 4;
  }

  const offscreen = document.createElement("canvas");
  offscreen.width = cols;
  offscreen.height = rows;
  offscreen.getContext("2d").putImageData(imageData, 0, 0);

  memoryContext.imageSmoothingEnabled = false;
  memoryContext.clearRect(0, 0, elements.memoryCanvas.width, elements.memoryCanvas.height);
  memoryContext.drawImage(offscreen, 0, 0, elements.memoryCanvas.width, elements.memoryCanvas.height);
  drawMemoryGuides(cols, rows);
}

function memoryCellColor(index, halfLife, maxMutation) {
  if (index >= state.maxCells) return [8, 10, 14];
  if (index >= state.liveCells) return [20, 24, 31];

  if (state.viewMode === "value") {
    return valueColor(state.values[index]);
  }

  if (state.viewMode === "risk") {
    const probability = mutationProbability(state.ages[index], halfLife, maxMutation);
    const risk = maxMutation > 0 ? probability / maxMutation : 0;
    return interpolateColor([42, 198, 178], [239, 106, 84], clamp(risk, 0, 1));
  }

  const normalizedAge = clamp(state.ages[index] / Math.max(1, halfLife * 2), 0, 1);
  return interpolateColor([49, 198, 178], [239, 106, 84], normalizedAge);
}

function drawMemoryGuides(cols, rows) {
  const canvas = elements.memoryCanvas;
  const cellWidth = canvas.width / cols;
  const cellHeight = canvas.height / rows;
  const coreSize = clamp(Math.floor(Number(elements.coreSizeInput.value) || 0), 0, state.maxCells);

  memoryContext.strokeStyle = "rgba(255, 255, 255, 0.08)";
  memoryContext.lineWidth = 1;
  for (let x = 0; x <= cols; x += 8) {
    memoryContext.beginPath();
    memoryContext.moveTo(Math.round(x * cellWidth) + 0.5, 0);
    memoryContext.lineTo(Math.round(x * cellWidth) + 0.5, canvas.height);
    memoryContext.stroke();
  }

  if (coreSize > 0) {
    const coreRows = Math.ceil(coreSize / cols);
    memoryContext.strokeStyle = "rgba(49, 198, 178, 0.85)";
    memoryContext.lineWidth = 2;
    memoryContext.strokeRect(1, 1, canvas.width - 2, Math.max(2, coreRows * cellHeight) - 2);
  }

  if (state.hoveredCell >= 0 && state.hoveredCell < state.maxCells) {
    const x = state.hoveredCell % cols;
    const y = Math.floor(state.hoveredCell / cols);
    memoryContext.strokeStyle = "rgba(247, 244, 220, 0.95)";
    memoryContext.lineWidth = 2;
    memoryContext.strokeRect(x * cellWidth + 1, y * cellHeight + 1, cellWidth - 2, cellHeight - 2);
  }
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

  drawHistoryLine(context, history, "energy", "#f0c75d", width, height);
  drawHistoryLine(context, history, "memory", "#31c6b2", width, height);

  context.font = "12px system-ui, sans-serif";
  context.fillStyle = "#f0c75d";
  context.fillText("energie", 10, 18);
  context.fillStyle = "#31c6b2";
  context.fillText("pamet", 68, 18);
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

function updateSelectedCell() {
  const index = state.hoveredCell;
  if (index < 0 || index >= state.maxCells) {
    elements.selectedAddress.textContent = "Adresa -";
    elements.selectedValue.textContent = "hodnota - / vek -";
    return;
  }

  const status = index < state.liveCells ? "ziva" : "odriznuta";
  elements.selectedAddress.textContent = `Adresa ${index} (${status})`;
  elements.selectedValue.textContent = `0x${state.values[index].toString(16).padStart(2, "0")} / vek ${formatNumber(state.ages[index])}`;
}

function animationLoop() {
  if (!state.running) return;

  const stepsPerFrame = clamp(Number(elements.stepsPerFrameInput.value) || 1, 1, 500);
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

function getGridColumns() {
  if (state.maxCells <= 256) return 32;
  if (state.maxCells <= 512) return 32;
  if (state.maxCells <= 1024) return 32;
  return 64;
}

function valueColor(value) {
  const t = value / 255;
  if (t < 0.33) return interpolateColor([18, 60, 105], [49, 198, 178], t / 0.33);
  if (t < 0.66) return interpolateColor([49, 198, 178], [240, 199, 93], (t - 0.33) / 0.33);
  return interpolateColor([240, 199, 93], [239, 106, 84], (t - 0.66) / 0.34);
}

function interpolateColor(a, b, t) {
  const clamped = clamp(t, 0, 1);
  return [
    Math.round(a[0] + (b[0] - a[0]) * clamped),
    Math.round(a[1] + (b[1] - a[1]) * clamped),
    Math.round(a[2] + (b[2] - a[2]) * clamped),
  ];
}

function randomByte() {
  return Math.floor(state.rng() * 256);
}

function mulberry32(seed) {
  let value = seed >>> 0;
  return function nextRandom() {
    value += 0x6d2b79f5;
    let t = value;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function canvasCellFromEvent(event) {
  const rect = elements.memoryCanvas.getBoundingClientRect();
  const cols = getGridColumns();
  const rows = Math.ceil(state.maxCells / cols);
  const x = clamp(Math.floor(((event.clientX - rect.left) / rect.width) * cols), 0, cols - 1);
  const y = clamp(Math.floor(((event.clientY - rect.top) / rect.height) * rows), 0, rows - 1);
  return y * cols + x;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function formatNumber(value) {
  const abs = Math.abs(value);
  if (abs > 0 && abs < 0.0001) return value.toExponential(2);
  if (abs >= 1_000_000) return value.toExponential(4);
  if (abs >= 1000) return value.toFixed(2);
  if (abs >= 1) return value.toFixed(3);
  return value.toFixed(6);
}

function formatInteger(value) {
  return new Intl.NumberFormat("cs-CZ").format(value);
}

elements.runButton.addEventListener("click", toggleRun);
elements.stepButton.addEventListener("click", stepOnce);
elements.resetButton.addEventListener("click", resetEntity);

elements.loseEnergyButton.addEventListener("click", () => {
  applyEnergyPulse(-Math.max(0, Number(elements.energyPulseInput.value) || 0));
});

elements.gainEnergyButton.addEventListener("click", () => {
  applyEnergyPulse(Math.max(0, Number(elements.energyPulseInput.value) || 0));
});

elements.refreshNowButton.addEventListener("click", () => {
  refreshCore();
  updateMetrics();
  render();
});

elements.energyInput.addEventListener("change", resetEntity);
elements.energyPerCellInput.addEventListener("change", resetEntity);
elements.maxCellsInput.addEventListener("change", resetEntity);
elements.patternInput.addEventListener("change", resetEntity);
elements.seedInput.addEventListener("change", resetEntity);

elements.leakInput.addEventListener("input", () => {
  state.leakRate = Number(elements.leakInput.value);
  elements.leakValue.value = state.leakRate.toFixed(4);
});

elements.maxMutationInput.addEventListener("input", () => {
  elements.maxMutationValue.value = Number(elements.maxMutationInput.value).toFixed(4);
});

elements.viewModeInputs.forEach((input) => {
  input.addEventListener("change", () => {
    if (input.checked) {
      state.viewMode = input.value;
      render();
    }
  });
});

elements.memoryCanvas.addEventListener("mousemove", (event) => {
  state.hoveredCell = canvasCellFromEvent(event);
  render();
});

elements.memoryCanvas.addEventListener("mouseleave", () => {
  state.hoveredCell = -1;
  render();
});

resetEntity();

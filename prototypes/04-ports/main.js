"use strict";

const DIRECTIONS = {
  xp: { dx: 1, dy: 0, dz: 0, opposite: "xn", label: "X+" },
  xn: { dx: -1, dy: 0, dz: 0, opposite: "xp", label: "X-" },
  yp: { dx: 0, dy: 1, dz: 0, opposite: "yn", label: "Y+" },
  yn: { dx: 0, dy: -1, dz: 0, opposite: "yp", label: "Y-" },
  zp: { dx: 0, dy: 0, dz: 1, opposite: "zn", label: "Z+" },
  zn: { dx: 0, dy: 0, dz: -1, opposite: "zp", label: "Z-" },
};

const elements = {
  worldCanvas: document.querySelector("#worldCanvas"),
  stepValue: document.querySelector("#stepValue"),
  runButton: document.querySelector("#runButton"),
  stepButton: document.querySelector("#stepButton"),
  resetButton: document.querySelector("#resetButton"),
  sizeInput: document.querySelector("#sizeInput"),
  ambientInput: document.querySelector("#ambientInput"),
  sourceInput: document.querySelector("#sourceInput"),
  diffusionInput: document.querySelector("#diffusionInput"),
  diffusionValue: document.querySelector("#diffusionValue"),
  stepsPerFrameInput: document.querySelector("#stepsPerFrameInput"),
  entityEnergyInput: document.querySelector("#entityEnergyInput"),
  energyPerCellInput: document.querySelector("#energyPerCellInput"),
  entityLeakInput: document.querySelector("#entityLeakInput"),
  entityLeakValue: document.querySelector("#entityLeakValue"),
  moveThresholdInput: document.querySelector("#moveThresholdInput"),
  moveThresholdValue: document.querySelector("#moveThresholdValue"),
  suctionImpulseInput: document.querySelector("#suctionImpulseInput"),
  portAmountInput: document.querySelector("#portAmountInput"),
  sliceInput: document.querySelector("#sliceInput"),
  sliceValue: document.querySelector("#sliceValue"),
  followInput: document.querySelector("#followInput"),
  positionValue: document.querySelector("#positionValue"),
  entityEnergyValue: document.querySelector("#entityEnergyValue"),
  memoryValue: document.querySelector("#memoryValue"),
  actionValue: document.querySelector("#actionValue"),
  fieldEnergyMetric: document.querySelector("#fieldEnergyMetric"),
  totalEnergyMetric: document.querySelector("#totalEnergyMetric"),
  emittedMetric: document.querySelector("#emittedMetric"),
  suckedMetric: document.querySelector("#suckedMetric"),
  absorbedMetric: document.querySelector("#absorbedMetric"),
  movesMetric: document.querySelector("#movesMetric"),
  axisInputs: Array.from(document.querySelectorAll("input[name='axis']")),
  portButtons: Array.from(document.querySelectorAll("[data-action][data-direction]")),
  sense: {
    xp: document.querySelector("#sense-xp"),
    xn: document.querySelector("#sense-xn"),
    yp: document.querySelector("#sense-yp"),
    yn: document.querySelector("#sense-yn"),
    zp: document.querySelector("#sense-zp"),
    zn: document.querySelector("#sense-zn"),
  },
};

const worldContext = elements.worldCanvas.getContext("2d");

const state = {
  size: 32,
  step: 0,
  running: false,
  axis: "z",
  slice: 16,
  diffusionRate: 0.04,
  current: new Float64Array(0),
  next: new Float64Array(0),
  imageData: null,
  entity: { x: 16, y: 16, z: 16, energy: 512 },
  emitted: 0,
  sucked: 0,
  absorbed: 0,
  moves: 0,
  lastAction: "-",
  fieldEnergy: 0,
};

function resetWorld() {
  state.size = Number(elements.sizeInput.value);
  state.step = 0;
  state.running = false;
  state.axis = selectedAxis();
  state.diffusionRate = Number(elements.diffusionInput.value);
  state.emitted = 0;
  state.sucked = 0;
  state.absorbed = 0;
  state.moves = 0;
  state.lastAction = "-";

  const cellCount = state.size * state.size * state.size;
  state.current = new Float64Array(cellCount);
  state.next = new Float64Array(cellCount);
  state.imageData = worldContext.createImageData(state.size, state.size);

  const ambient = Math.max(0, Number(elements.ambientInput.value) || 0);
  state.current.fill(ambient);

  const center = Math.floor(state.size / 2);
  state.entity = {
    x: center,
    y: center,
    z: center,
    energy: Math.max(0, Number(elements.entityEnergyInput.value) || 0),
  };

  const sourceEnergy = Math.max(0, Number(elements.sourceInput.value) || 0);
  const sourceX = wrap(center + Math.floor(state.size / 5));
  state.current[cellIndex(sourceX, center, center)] += sourceEnergy;

  absorbEntityCell("start");
  followEntitySlice();

  elements.sliceInput.max = String(state.size - 1);
  elements.sliceInput.value = String(state.slice);
  elements.sliceValue.value = String(state.slice);
  elements.runButton.textContent = "Spustit";
  elements.runButton.classList.remove("primary");
  elements.diffusionValue.value = state.diffusionRate.toFixed(3);
  elements.entityLeakValue.value = Number(elements.entityLeakInput.value).toFixed(4);
  elements.moveThresholdValue.value = Number(elements.moveThresholdInput.value).toFixed(2);

  updateMetrics();
  render();
}

function simulateStep() {
  diffuseField();
  leakEntity();
  state.step += 1;
}

function diffuseField() {
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
        const neighborSum =
          current[rowOffset + ((x + 1) % size)] +
          current[rowOffset + ((x + size - 1) % size)] +
          current[ypOffset + x] +
          current[ynOffset + x] +
          current[zpOffset + y * size + x] +
          current[znOffset + y * size + x];
        next[index] = current[index] + rate * (neighborSum - 6 * current[index]);
      }
    }
  }

  const previous = state.current;
  state.current = state.next;
  state.next = previous;
}

function leakEntity() {
  const leakRate = Number(elements.entityLeakInput.value);
  if (leakRate <= 0 || state.entity.energy <= 0) return;

  const totalLeak = Math.min(state.entity.energy, state.entity.energy * leakRate);
  const perDirection = totalLeak / 6;
  state.entity.energy -= totalLeak;

  for (const key of Object.keys(DIRECTIONS)) {
    const position = neighborPosition(key);
    state.current[cellIndex(position.x, position.y, position.z)] += perDirection;
  }

  state.emitted += totalLeak;
}

function firePort(directionKey) {
  const amount = Math.min(readPortAmount(), state.entity.energy);
  if (amount <= 0) {
    state.lastAction = `zazeh ${DIRECTIONS[directionKey].label}: bez energie`;
    updateMetrics();
    render();
    return;
  }

  const exhaust = neighborPosition(directionKey);
  state.current[cellIndex(exhaust.x, exhaust.y, exhaust.z)] += amount;
  state.entity.energy -= amount;
  state.emitted += amount;

  moveEntity(DIRECTIONS[directionKey].opposite, `zazeh ${DIRECTIONS[directionKey].label}`);
}

function suckPort(directionKey) {
  const amount = readPortAmount();
  const source = neighborPosition(directionKey);
  const index = cellIndex(source.x, source.y, source.z);
  const taken = Math.min(amount, state.current[index]);
  state.current[index] -= taken;
  state.entity.energy += taken;
  state.sucked += taken;
  state.lastAction = `sani ${DIRECTIONS[directionKey].label}: ${formatNumber(taken)}`;

  if (taken > 0 && elements.suctionImpulseInput.checked) {
    moveEntity(directionKey, `sani ${DIRECTIONS[directionKey].label}`);
  } else {
    updateMetrics();
    render();
  }
}

function moveEntity(directionKey, reason) {
  const direction = DIRECTIONS[directionKey];
  const target = {
    x: wrap(state.entity.x + direction.dx),
    y: wrap(state.entity.y + direction.dy),
    z: wrap(state.entity.z + direction.dz),
  };
  const targetEnergy = state.current[cellIndex(target.x, target.y, target.z)];
  const ratio = state.entity.energy > 0 ? targetEnergy / state.entity.energy : Number.POSITIVE_INFINITY;
  const threshold = Number(elements.moveThresholdInput.value);

  if (ratio > threshold) {
    state.lastAction = `${reason}: blok ${DIRECTIONS[directionKey].label}, cil/ent ${formatNumber(ratio)}`;
    updateMetrics();
    render();
    return;
  }

  state.entity.x = target.x;
  state.entity.y = target.y;
  state.entity.z = target.z;
  state.moves += 1;
  const absorbed = absorbEntityCell(reason);
  state.lastAction = `${reason}: pohyb ${DIRECTIONS[directionKey].label}, cil/ent ${formatNumber(ratio)}, absorb ${formatNumber(absorbed)}`;
  followEntitySlice();
  updateMetrics();
  render();
}

function absorbEntityCell(reason) {
  const index = entityIndex();
  const absorbed = state.current[index];
  if (absorbed > 0) {
    state.entity.energy += absorbed;
    state.current[index] = 0;
    state.absorbed += absorbed;
    state.lastAction = `${reason}: absorb ${formatNumber(absorbed)}`;
  }
  return absorbed;
}

function updateMetrics() {
  let fieldEnergy = 0;
  let maxField = 0;
  for (let i = 0; i < state.current.length; i += 1) {
    fieldEnergy += state.current[i];
    if (state.current[i] > maxField) maxField = state.current[i];
  }

  state.fieldEnergy = fieldEnergy;

  elements.stepValue.textContent = formatInteger(state.step);
  elements.positionValue.textContent = `${state.entity.x}, ${state.entity.y}, ${state.entity.z}`;
  elements.entityEnergyValue.textContent = formatNumber(state.entity.energy);
  elements.memoryValue.textContent = formatInteger(Math.floor(state.entity.energy / Math.max(0.01, Number(elements.energyPerCellInput.value) || 1)));
  elements.actionValue.textContent = state.lastAction;
  elements.fieldEnergyMetric.textContent = formatNumber(fieldEnergy);
  elements.totalEnergyMetric.textContent = formatNumber(fieldEnergy + state.entity.energy);
  elements.emittedMetric.textContent = formatNumber(state.emitted);
  elements.suckedMetric.textContent = formatNumber(state.sucked);
  elements.absorbedMetric.textContent = formatNumber(state.absorbed);
  elements.movesMetric.textContent = formatInteger(state.moves);

  for (const key of Object.keys(DIRECTIONS)) {
    const position = neighborPosition(key);
    elements.sense[key].textContent = formatNumber(state.current[cellIndex(position.x, position.y, position.z)]);
  }
}

function render() {
  drawWorldSlice();
}

function drawWorldSlice() {
  const size = state.size;
  const imageData = state.imageData;
  const data = imageData.data;
  const max = findSliceMax();
  const scale = Math.log1p(Math.max(1, max));

  let pixel = 0;
  for (let row = 0; row < size; row += 1) {
    for (let col = 0; col < size; col += 1) {
      const energy = readSliceEnergy(col, row);
      const normalized = Math.log1p(Math.max(0, energy)) / scale;
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
  drawEntityOverlay();
}

function drawEntityOverlay() {
  const projected = projectEntityToSlice();
  if (!projected.visible) return;

  const cellSize = elements.worldCanvas.width / state.size;
  const x = projected.x * cellSize;
  const y = projected.y * cellSize;

  worldContext.strokeStyle = "rgba(247, 244, 220, 0.98)";
  worldContext.lineWidth = Math.max(2, cellSize * 0.12);
  worldContext.strokeRect(x + 2, y + 2, cellSize - 4, cellSize - 4);
  worldContext.fillStyle = "rgba(247, 244, 220, 0.55)";
  worldContext.fillRect(x + cellSize * 0.35, y + cellSize * 0.35, cellSize * 0.3, cellSize * 0.3);
}

function findSliceMax() {
  let max = 1;
  for (let row = 0; row < state.size; row += 1) {
    for (let col = 0; col < state.size; col += 1) {
      max = Math.max(max, readSliceEnergy(col, row));
    }
  }
  return max;
}

function readSliceEnergy(col, row) {
  if (state.axis === "z") {
    return state.current[cellIndex(col, row, state.slice)];
  }
  if (state.axis === "y") {
    return state.current[cellIndex(col, state.slice, row)];
  }
  return state.current[cellIndex(state.slice, col, row)];
}

function projectEntityToSlice() {
  if (state.axis === "z") {
    return { visible: state.entity.z === state.slice, x: state.entity.x, y: state.entity.y };
  }
  if (state.axis === "y") {
    return { visible: state.entity.y === state.slice, x: state.entity.x, y: state.entity.z };
  }
  return { visible: state.entity.x === state.slice, x: state.entity.y, y: state.entity.z };
}

function followEntitySlice() {
  if (!elements.followInput.checked) return;
  if (state.axis === "z") state.slice = state.entity.z;
  if (state.axis === "y") state.slice = state.entity.y;
  if (state.axis === "x") state.slice = state.entity.x;
  elements.sliceInput.value = String(state.slice);
  elements.sliceValue.value = String(state.slice);
}

function neighborPosition(directionKey) {
  const direction = DIRECTIONS[directionKey];
  return {
    x: wrap(state.entity.x + direction.dx),
    y: wrap(state.entity.y + direction.dy),
    z: wrap(state.entity.z + direction.dz),
  };
}

function entityIndex() {
  return cellIndex(state.entity.x, state.entity.y, state.entity.z);
}

function cellIndex(x, y, z) {
  return x + state.size * (y + state.size * z);
}

function wrap(value) {
  return ((value % state.size) + state.size) % state.size;
}

function readPortAmount() {
  return Math.max(0, Number(elements.portAmountInput.value) || 0);
}

function selectedAxis() {
  const selected = elements.axisInputs.find((input) => input.checked);
  return selected ? selected.value : "z";
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

function animationLoop() {
  if (!state.running) return;

  const steps = clamp(Number(elements.stepsPerFrameInput.value) || 1, 1, 100);
  for (let i = 0; i < steps; i += 1) {
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
  if (state.running) requestAnimationFrame(animationLoop);
}

function stepOnce() {
  if (state.running) toggleRun();
  simulateStep();
  updateMetrics();
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
  if (abs >= 1) return value.toFixed(3);
  return value.toFixed(6);
}

function formatInteger(value) {
  return new Intl.NumberFormat("cs-CZ").format(value);
}

elements.runButton.addEventListener("click", toggleRun);
elements.stepButton.addEventListener("click", stepOnce);
elements.resetButton.addEventListener("click", resetWorld);

elements.sizeInput.addEventListener("change", resetWorld);
elements.ambientInput.addEventListener("change", resetWorld);
elements.sourceInput.addEventListener("change", resetWorld);
elements.entityEnergyInput.addEventListener("change", resetWorld);
elements.energyPerCellInput.addEventListener("change", updateMetrics);

elements.diffusionInput.addEventListener("input", () => {
  state.diffusionRate = Number(elements.diffusionInput.value);
  elements.diffusionValue.value = state.diffusionRate.toFixed(3);
});

elements.entityLeakInput.addEventListener("input", () => {
  elements.entityLeakValue.value = Number(elements.entityLeakInput.value).toFixed(4);
});

elements.moveThresholdInput.addEventListener("input", () => {
  elements.moveThresholdValue.value = Number(elements.moveThresholdInput.value).toFixed(2);
});

elements.sliceInput.addEventListener("input", () => {
  state.slice = Number(elements.sliceInput.value);
  elements.sliceValue.value = String(state.slice);
  render();
});

elements.axisInputs.forEach((input) => {
  input.addEventListener("change", () => {
    if (input.checked) {
      state.axis = input.value;
      followEntitySlice();
      render();
    }
  });
});

elements.followInput.addEventListener("change", () => {
  followEntitySlice();
  render();
});

elements.portButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const action = button.dataset.action;
    const direction = button.dataset.direction;
    if (action === "fire") firePort(direction);
    if (action === "suck") suckPort(direction);
  });
});

resetWorld();

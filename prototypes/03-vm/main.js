"use strict";

const OP = {
  NOP0: 0x00,
  NOP: 0x01,
  SET: 0x10,
  COPY: 0x11,
  ADD: 0x20,
  SUB: 0x21,
  INC: 0x30,
  DEC: 0x31,
  JMP: 0x40,
  JZ: 0x41,
};

const OPCODE_NAMES = new Map([
  [OP.NOP0, "nop0"],
  [OP.NOP, "nop"],
  [OP.SET, "set"],
  [OP.COPY, "copy"],
  [OP.ADD, "add"],
  [OP.SUB, "sub"],
  [OP.INC, "inc"],
  [OP.DEC, "dec"],
  [OP.JMP, "jmp"],
  [OP.JZ, "jz"],
]);

const elements = {
  memoryCanvas: document.querySelector("#memoryCanvas"),
  stepValue: document.querySelector("#stepValue"),
  pcValue: document.querySelector("#pcValue"),
  spValue: document.querySelector("#spValue"),
  zeroValue: document.querySelector("#zeroValue"),
  instructionValue: document.querySelector("#instructionValue"),
  traceList: document.querySelector("#traceList"),
  runButton: document.querySelector("#runButton"),
  stepButton: document.querySelector("#stepButton"),
  tickButton: document.querySelector("#tickButton"),
  resetButton: document.querySelector("#resetButton"),
  programInput: document.querySelector("#programInput"),
  seedInput: document.querySelector("#seedInput"),
  energyInput: document.querySelector("#energyInput"),
  leakInput: document.querySelector("#leakInput"),
  leakValue: document.querySelector("#leakValue"),
  energyPulseInput: document.querySelector("#energyPulseInput"),
  loseEnergyButton: document.querySelector("#loseEnergyButton"),
  gainEnergyButton: document.querySelector("#gainEnergyButton"),
  mutateButton: document.querySelector("#mutateButton"),
  halfLifeInput: document.querySelector("#halfLifeInput"),
  maxMutationInput: document.querySelector("#maxMutationInput"),
  maxMutationValue: document.querySelector("#maxMutationValue"),
  stepsPerFrameInput: document.querySelector("#stepsPerFrameInput"),
  ageEnabledInput: document.querySelector("#ageEnabledInput"),
  energyMetric: document.querySelector("#energyMetric"),
  memoryMetric: document.querySelector("#memoryMetric"),
  instructionMetric: document.querySelector("#instructionMetric"),
  mutationMetric: document.querySelector("#mutationMetric"),
  writeMetric: document.querySelector("#writeMetric"),
  statusMetric: document.querySelector("#statusMetric"),
  viewModeInputs: Array.from(document.querySelectorAll("input[name='viewMode']")),
};

const memoryContext = elements.memoryCanvas.getContext("2d");

const state = {
  step: 0,
  running: false,
  stopped: false,
  fault: "",
  pc: 0,
  sp: 255,
  zero: false,
  energy: 256,
  liveCells: 256,
  instructionCount: 0,
  totalMutations: 0,
  writes: 0,
  viewMode: "value",
  lastInstruction: "-",
  memory: new Uint8Array(256),
  ages: new Float64Array(256),
  trace: [],
  rng: mulberry32(12345),
};

function resetVm() {
  state.step = 0;
  state.running = false;
  state.stopped = false;
  state.fault = "";
  state.pc = 0;
  state.sp = 255;
  state.zero = false;
  state.energy = clamp(Number(elements.energyInput.value) || 1, 1, 256);
  state.liveCells = Math.floor(state.energy);
  state.instructionCount = 0;
  state.totalMutations = 0;
  state.writes = 0;
  state.lastInstruction = "-";
  state.trace = [];
  state.rng = mulberry32(Number(elements.seedInput.value) || 1);
  state.memory.fill(0);
  state.ages.fill(0);

  loadProgram(elements.programInput.value);
  resizeMemoryFromEnergy();

  elements.runButton.textContent = "Spustit";
  elements.runButton.classList.remove("primary");
  elements.leakValue.value = Number(elements.leakInput.value).toFixed(4);
  elements.maxMutationValue.value = Number(elements.maxMutationInput.value).toFixed(4);

  updateUi();
  render();
}

function loadProgram(name) {
  if (name === "random") {
    for (let i = 0; i < 256; i += 1) {
      state.memory[i] = randomByte();
      state.ages[i] = Math.floor(state.rng() * 60);
    }
    return;
  }

  const programs = {
    loop: [
      OP.INC, 0x80,
      OP.COPY, 0x00, 0x00,
      OP.JMP, 0x00,
    ],
    selfModify: [
      OP.INC, 0x08,
      OP.JMP, 0x00,
      OP.NOP,
    ],
    decay: [
      OP.INC, 0xf8,
      OP.INC, 0xf9,
      OP.INC, 0xfa,
      OP.JMP, 0x00,
    ],
    dataRun: [
      OP.SET, 0x20, OP.INC,
      OP.SET, 0x21, 0x80,
      OP.SET, 0x22, OP.JMP,
      OP.SET, 0x23, 0x20,
      OP.JMP, 0x20,
    ],
  };

  const bytes = programs[name] || programs.loop;
  state.memory.set(bytes, 0);
  for (let i = 0; i < 256; i += 1) {
    state.ages[i] = i < bytes.length ? 0 : 80 + Math.floor(state.rng() * 80);
    if (i >= bytes.length) {
      state.memory[i] = randomByte();
    }
  }
}

function tick() {
  if (state.stopped) return;
  state.step += 1;
  state.energy = Math.max(0, state.energy * (1 - Number(elements.leakInput.value)));
  resizeMemoryFromEnergy();

  if (elements.ageEnabledInput.checked) {
    ageMemory();
    mutateMemory();
  }

  executeInstruction();
}

function executeInstruction() {
  if (state.liveCells <= 0) {
    state.lastInstruction = "idle bez pameti";
    rememberTrace("idle bez pameti");
    return;
  }

  state.pc = normalizeAddress(state.pc);
  const startPc = state.pc;
  const opcode = readByte(state.pc);
  let text = "";

  switch (opcode) {
    case OP.NOP0:
      text = `${hex(startPc)} nop0/data`;
      advancePc(1);
      break;
    case OP.NOP:
      text = `${hex(startPc)} nop`;
      advancePc(1);
      break;
    case OP.SET: {
      const address = readByte((state.pc + 1) & 255);
      const value = readByte((state.pc + 2) & 255);
      writeByte(address, value);
      state.zero = value === 0;
      text = `${hex(startPc)} set [${addressText(address)}], ${hex(value)}`;
      advancePc(3);
      break;
    }
    case OP.COPY: {
      const dest = readByte((state.pc + 1) & 255);
      const src = readByte((state.pc + 2) & 255);
      const value = readByte(src);
      writeByte(dest, value);
      state.zero = value === 0;
      text = `${hex(startPc)} copy [${addressText(dest)}], [${addressText(src)}]`;
      advancePc(3);
      break;
    }
    case OP.ADD: {
      const dest = readByte((state.pc + 1) & 255);
      const src = readByte((state.pc + 2) & 255);
      const value = (readByte(dest) + readByte(src)) & 255;
      writeByte(dest, value);
      state.zero = value === 0;
      text = `${hex(startPc)} add [${addressText(dest)}], [${addressText(src)}]`;
      advancePc(3);
      break;
    }
    case OP.SUB: {
      const dest = readByte((state.pc + 1) & 255);
      const src = readByte((state.pc + 2) & 255);
      const value = (readByte(dest) - readByte(src)) & 255;
      writeByte(dest, value);
      state.zero = value === 0;
      text = `${hex(startPc)} sub [${addressText(dest)}], [${addressText(src)}]`;
      advancePc(3);
      break;
    }
    case OP.INC: {
      const address = readByte((state.pc + 1) & 255);
      const value = (readByte(address) + 1) & 255;
      writeByte(address, value);
      state.zero = value === 0;
      text = `${hex(startPc)} inc [${addressText(address)}]`;
      advancePc(2);
      break;
    }
    case OP.DEC: {
      const address = readByte((state.pc + 1) & 255);
      const value = (readByte(address) - 1) & 255;
      writeByte(address, value);
      state.zero = value === 0;
      text = `${hex(startPc)} dec [${addressText(address)}]`;
      advancePc(2);
      break;
    }
    case OP.JMP: {
      const target = readByte((state.pc + 1) & 255);
      text = `${hex(startPc)} jmp ${addressText(target)}`;
      state.pc = normalizeAddress(target);
      break;
    }
    case OP.JZ: {
      const address = readByte((state.pc + 1) & 255);
      const target = readByte((state.pc + 2) & 255);
      const value = readByte(address);
      state.zero = value === 0;
      text = `${hex(startPc)} jz [${addressText(address)}], ${addressText(target)} -> ${state.zero ? "skok" : "nic"}`;
      state.pc = state.zero ? normalizeAddress(target) : normalizeAddress(state.pc + 3);
      break;
    }
    default:
      text = `${hex(startPc)} data/opcode ${hex(opcode)}`;
      advancePc(1);
      break;
  }

  state.instructionCount += 1;
  state.lastInstruction = text;
  rememberTrace(text);
}

function readByte(address) {
  if (state.liveCells <= 0) {
    return 0;
  }
  return state.memory[normalizeAddress(address)];
}

function writeByte(address, value) {
  if (state.liveCells <= 0) {
    return;
  }
  const effectiveAddress = normalizeAddress(address);
  state.memory[effectiveAddress] = value & 255;
  state.ages[effectiveAddress] = 0;
  state.writes += 1;
}

function resizeMemoryFromEnergy() {
  const nextLiveCells = clamp(Math.floor(state.energy), 0, 256);
  if (nextLiveCells < state.liveCells) {
    for (let i = nextLiveCells; i < state.liveCells; i += 1) {
      state.ages[i] = 0;
    }
  } else if (nextLiveCells > state.liveCells) {
    for (let i = state.liveCells; i < nextLiveCells; i += 1) {
      state.memory[i] = 0;
      state.ages[i] = 0;
    }
  }
  state.liveCells = nextLiveCells;
  if (state.liveCells > 0) {
    state.pc = normalizeAddress(state.pc);
  }
}

function ageMemory() {
  for (let i = 0; i < state.liveCells; i += 1) {
    state.ages[i] += 1;
  }
}

function mutateMemory() {
  const halfLife = Math.max(1, Number(elements.halfLifeInput.value) || 1);
  const maxMutation = Number(elements.maxMutationInput.value);

  for (let i = 0; i < state.liveCells; i += 1) {
    const probability = maxMutation * (1 - 2 ** (-state.ages[i] / halfLife));
    if (state.rng() < probability) {
      state.memory[i] ^= 1 << Math.floor(state.rng() * 8);
      state.ages[i] = 0;
      state.totalMutations += 1;
      rememberTrace(`mut ${hex(i)}`);
    }
  }
}

function mutatePcByte() {
  if (state.liveCells <= 0) return;
  state.pc = normalizeAddress(state.pc);
  state.memory[state.pc] ^= 1 << Math.floor(state.rng() * 8);
  state.ages[state.pc] = 0;
  state.totalMutations += 1;
  rememberTrace(`manual mut PC ${hex(state.pc)}`);
  updateUi();
  render();
}

function applyEnergyPulse(delta) {
  state.energy = clamp(state.energy + delta, 0, 256);
  resizeMemoryFromEnergy();
  updateUi();
  render();
}

function stopWithFault(message) {
  state.stopped = true;
  state.fault = message;
  rememberTrace(message);
}

function normalizeAddress(address) {
  if (state.liveCells <= 0) return 0;
  return ((address % state.liveCells) + state.liveCells) % state.liveCells;
}

function advancePc(offset) {
  state.pc = normalizeAddress(state.pc + offset);
}

function addressText(address) {
  const effectiveAddress = normalizeAddress(address);
  if (state.liveCells === 256 || effectiveAddress === address) return hex(address);
  return `${hex(address)}->${hex(effectiveAddress)}`;
}

function rememberTrace(text) {
  state.trace.unshift(text);
  if (state.trace.length > 18) {
    state.trace.pop();
  }
}

function updateUi() {
  elements.stepValue.textContent = formatInteger(state.step);
  elements.pcValue.textContent = hex(state.pc);
  elements.spValue.textContent = hex(state.sp);
  elements.zeroValue.textContent = state.zero ? "1" : "0";
  elements.instructionValue.textContent = state.lastInstruction;
  elements.energyMetric.textContent = formatNumber(state.energy);
  elements.memoryMetric.textContent = `${formatInteger(state.liveCells)} / 256`;
  elements.instructionMetric.textContent = formatInteger(state.instructionCount);
  elements.mutationMetric.textContent = formatInteger(state.totalMutations);
  elements.writeMetric.textContent = formatInteger(state.writes);
  elements.statusMetric.textContent = state.fault || (state.stopped ? "stopped" : "bezi");
  renderTrace();
}

function render() {
  drawMemory();
}

function drawMemory() {
  const cols = 16;
  const rows = 16;
  const imageData = memoryContext.createImageData(cols, rows);
  const data = imageData.data;
  const halfLife = Math.max(1, Number(elements.halfLifeInput.value) || 1);
  const maxMutation = Number(elements.maxMutationInput.value);

  let pixel = 0;
  for (let i = 0; i < 256; i += 1) {
    const color = cellColor(i, halfLife, maxMutation);
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

function cellColor(index, halfLife, maxMutation) {
  if (index >= state.liveCells) return [20, 24, 31];
  if (index === state.pc) return [247, 244, 220];

  if (state.viewMode === "age") {
    const age = clamp(state.ages[index] / Math.max(1, halfLife * 2), 0, 1);
    return interpolateColor([49, 198, 178], [239, 106, 84], age);
  }

  if (state.viewMode === "risk") {
    const probability = maxMutation * (1 - 2 ** (-state.ages[index] / halfLife));
    const risk = maxMutation > 0 ? probability / maxMutation : 0;
    return interpolateColor([42, 198, 178], [239, 106, 84], risk);
  }

  return valueColor(state.memory[index]);
}

function drawMemoryGuides(cols, rows) {
  const cellWidth = elements.memoryCanvas.width / cols;
  const cellHeight = elements.memoryCanvas.height / rows;

  memoryContext.strokeStyle = "rgba(255, 255, 255, 0.08)";
  memoryContext.lineWidth = 1;
  for (let x = 0; x <= cols; x += 1) {
    memoryContext.beginPath();
    memoryContext.moveTo(Math.round(x * cellWidth) + 0.5, 0);
    memoryContext.lineTo(Math.round(x * cellWidth) + 0.5, elements.memoryCanvas.height);
    memoryContext.stroke();
  }
  for (let y = 0; y <= rows; y += 1) {
    memoryContext.beginPath();
    memoryContext.moveTo(0, Math.round(y * cellHeight) + 0.5);
    memoryContext.lineTo(elements.memoryCanvas.width, Math.round(y * cellHeight) + 0.5);
    memoryContext.stroke();
  }

  const pcX = state.pc % cols;
  const pcY = Math.floor(state.pc / cols);
  memoryContext.strokeStyle = "rgba(247, 244, 220, 0.95)";
  memoryContext.lineWidth = 3;
  memoryContext.strokeRect(pcX * cellWidth + 2, pcY * cellHeight + 2, cellWidth - 4, cellHeight - 4);
}

function renderTrace() {
  elements.traceList.replaceChildren(...state.trace.map((entry) => {
    const item = document.createElement("li");
    item.textContent = entry;
    return item;
  }));
}

function animationLoop() {
  if (!state.running) return;

  const instructionsPerFrame = clamp(Number(elements.stepsPerFrameInput.value) || 1, 1, 1000);
  for (let i = 0; i < instructionsPerFrame && !state.stopped; i += 1) {
    tick();
  }

  if (state.stopped) {
    state.running = false;
    elements.runButton.textContent = "Spustit";
    elements.runButton.classList.remove("primary");
  }

  updateUi();
  render();
  if (state.running) requestAnimationFrame(animationLoop);
}

function toggleRun() {
  if (state.stopped) return;
  state.running = !state.running;
  elements.runButton.textContent = state.running ? "Pauza" : "Spustit";
  elements.runButton.classList.toggle("primary", state.running);
  if (state.running) requestAnimationFrame(animationLoop);
}

function stepInstruction() {
  if (state.running) toggleRun();
  if (!state.stopped) {
    executeInstruction();
    updateUi();
    render();
  }
}

function stepTick() {
  if (state.running) toggleRun();
  if (!state.stopped) {
    tick();
    updateUi();
    render();
  }
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

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function hex(value) {
  return `0x${(value & 255).toString(16).padStart(2, "0")}`;
}

function formatNumber(value) {
  const abs = Math.abs(value);
  if (abs > 0 && abs < 0.0001) return value.toExponential(2);
  if (abs >= 1000) return value.toFixed(2);
  if (abs >= 1) return value.toFixed(3);
  return value.toFixed(6);
}

function formatInteger(value) {
  return new Intl.NumberFormat("cs-CZ").format(value);
}

elements.runButton.addEventListener("click", toggleRun);
elements.stepButton.addEventListener("click", stepInstruction);
elements.tickButton.addEventListener("click", stepTick);
elements.resetButton.addEventListener("click", resetVm);

elements.programInput.addEventListener("change", resetVm);
elements.seedInput.addEventListener("change", resetVm);
elements.energyInput.addEventListener("change", resetVm);

elements.leakInput.addEventListener("input", () => {
  elements.leakValue.value = Number(elements.leakInput.value).toFixed(4);
});

elements.maxMutationInput.addEventListener("input", () => {
  elements.maxMutationValue.value = Number(elements.maxMutationInput.value).toFixed(4);
});

elements.loseEnergyButton.addEventListener("click", () => {
  applyEnergyPulse(-Math.max(0, Number(elements.energyPulseInput.value) || 0));
});

elements.gainEnergyButton.addEventListener("click", () => {
  applyEnergyPulse(Math.max(0, Number(elements.energyPulseInput.value) || 0));
});

elements.mutateButton.addEventListener("click", mutatePcByte);

elements.viewModeInputs.forEach((input) => {
  input.addEventListener("change", () => {
    if (input.checked) {
      state.viewMode = input.value;
      render();
    }
  });
});

resetVm();

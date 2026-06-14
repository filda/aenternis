// Aenternis assembler.
//
// Translates a simple human-readable mnemonic source into a Uint32Array
// of memory slots that match the layout produced by the Rust VM. Used
// by the frontend's "Program v centrální buňce" textarea to inject a
// programmer-defined prefix into the big-bang cell, mirroring the
// `bigBang(eTotal, programSlots)` API of JS prototype 9.
//
// Syntax
// ------
//
//   ; line comment to end of line
//   label:                       — defines a label at the current slot offset
//   mnemonic [arg [, arg [, ...]]]
//
// A line containing only a number (decimal or `0x`-hex) emits one raw
// slot — useful for embedding constants without a `set` instruction.
//
// Operands accept:
//   - direction names: `xp xn yp yn zp zn` (resolved to 0..5)
//   - decimal integers (`42`, `-1` wraps to `0xFFFFFFFF`)
//   - hex integers: `0x1A`, `0XFF`
//   - label references (resolved to the slot offset of the label)
//
// Mnemonics cover the *implemented* opcodes from `docs/vm.md` — the ones
// the Rust VM actually executes (0x00–0x1E, `Opcode::MAX`). Adding a new
// opcode requires extending this map (and the Rust VM, of course); keep the
// codes contiguous and append-only so the VM's `byte % COUNT` fold stays
// stable for existing programs (see `docs/vm.md`).

export interface Opcode {
  readonly code: number;
  readonly args: number;
}

export const OPCODES: Readonly<Record<string, Opcode>> = Object.freeze({
  nop:     { code: 0x00, args: 0 },
  set:     { code: 0x01, args: 2 }, // a, v
  copy:    { code: 0x02, args: 2 }, // a, b
  add:     { code: 0x03, args: 2 }, // a, b
  sub:     { code: 0x04, args: 2 }, // a, b
  inc:     { code: 0x05, args: 1 }, // a
  dec:     { code: 0x06, args: 1 }, // a
  jmp:     { code: 0x07, args: 1 }, // a
  jz:      { code: 0x08, args: 2 }, // a, t
  setp:    { code: 0x09, args: 2 }, // d, v
  getp:    { code: 0x0a, args: 2 }, // d, a
  port:    { code: 0x0b, args: 2 }, // d, i
  senergy: { code: 0x0c, args: 2 }, // d, a
  jne:     { code: 0x0d, args: 2 }, // a, t
  je:      { code: 0x0e, args: 3 }, // a, b, t
  ldi:     { code: 0x0f, args: 2 }, // a, b
  sti:     { code: 0x10, args: 2 }, // a, b
  setpv:   { code: 0x11, args: 2 }, // d, a
  sid:     { code: 0x12, args: 1 }, // a
  paint:   { code: 0x13, args: 1 }, // v
  and:     { code: 0x14, args: 2 }, // a, b
  or:      { code: 0x15, args: 2 }, // a, b
  xor:     { code: 0x16, args: 2 }, // a, b
  not:     { code: 0x17, args: 1 }, // a
  shl:     { code: 0x18, args: 2 }, // a, b
  shr:     { code: 0x19, args: 2 }, // a, b
  mul:     { code: 0x1a, args: 2 }, // a, b
  div:     { code: 0x1b, args: 2 }, // a, b
  mod:     { code: 0x1c, args: 2 }, // a, b
  jp:      { code: 0x1d, args: 2 }, // a, t
  jn:      { code: 0x1e, args: 2 }, // a, t
});

export const DIRECTIONS = Object.freeze({
  xp: 0, xn: 1, yp: 2, yn: 3, zp: 4, zn: 5,
} as const);

type Direction = keyof typeof DIRECTIONS;

function isDirection(s: string): s is Direction {
  return Object.prototype.hasOwnProperty.call(DIRECTIONS, s);
}

/**
 * Try to parse a single operand into a u32. Returns `null` on failure.
 * The caller distinguishes "needs label resolution" from "garbage" by
 * passing `labels`; if the operand is an identifier and `labels` has
 * an entry, the slot offset is returned.
 */
export function resolveOperand(s: string, labels: Map<string, number>): number | null {
  const t = s.trim();
  if (t.length === 0) return null;

  // Direction shorthand
  const lower = t.toLowerCase();
  if (isDirection(lower)) {
    return DIRECTIONS[lower];
  }

  // Hex
  if (/^0[xX][0-9a-fA-F]+$/.test(t)) {
    const v = parseInt(t.slice(2), 16);
    return Number.isFinite(v) ? v >>> 0 : null;
  }

  // Decimal (positive or negative; negative wraps to u32 two's complement)
  if (/^-?\d+$/.test(t)) {
    const v = parseInt(t, 10);
    return Number.isFinite(v) ? v >>> 0 : null;
  }

  // Label
  if (/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(t) && labels.has(t)) {
    // labels.has(t) guarantees the entry exists; the `!` is justified.
    return labels.get(t)!;
  }

  return null;
}

type Token =
  | { readonly kind: 'raw'; readonly value: number; readonly lineNo: number }
  | { readonly kind: 'instr'; readonly code: number; readonly args: readonly string[]; readonly lineNo: number };

export interface AssembleResult {
  readonly slots: Uint32Array;
  readonly errors: string[];
}

/**
 * Two-pass assembler.
 *
 * Pass 1 — tokenize each line and accumulate slot offsets, building a
 *          label-to-offset map.
 * Pass 2 — emit slots, resolving label references against the map.
 *
 * Returns `{ slots, errors }`. `slots` is always present (best-effort
 * partial output even on errors); `errors` is empty on a clean parse.
 */
export function assemble(text: string): AssembleResult {
  const errors: string[] = [];
  const labels = new Map<string, number>();
  const tokens: Token[] = [];
  let slotOffset = 0;

  // ---- Pass 1: tokenize + label collection ---------------------------------

  const lines = text.split(/\r?\n/);
  for (let lineNo = 0; lineNo < lines.length; lineNo++) {
    // The `for` loop bounds guarantee `lines[lineNo]` is defined.
    let line = lines[lineNo]!;

    // Strip comments
    const commentAt = line.indexOf(';');
    if (commentAt !== -1) line = line.slice(0, commentAt);
    line = line.trim();
    if (line.length === 0) continue;

    // Optional label prefix: "name: rest"
    const labelMatch = line.match(/^([a-zA-Z_][a-zA-Z0-9_]*)\s*:\s*(.*)$/);
    if (labelMatch) {
      // Both capture groups are mandatory in the regex above.
      const name = labelMatch[1]!;
      if (labels.has(name)) {
        errors.push(`line ${lineNo + 1}: duplicate label "${name}"`);
      } else {
        labels.set(name, slotOffset);
      }
      line = labelMatch[2]!.trim();
      if (line.length === 0) continue;
    }

    // Sole-number line → raw slot
    if (/^(0[xX][0-9a-fA-F]+|-?\d+)$/.test(line)) {
      const value = resolveOperand(line, labels);
      if (value === null) {
        errors.push(`line ${lineNo + 1}: cannot parse number "${line}"`);
        continue;
      }
      tokens.push({ kind: 'raw', value, lineNo });
      slotOffset += 1;
      continue;
    }

    // Mnemonic + optional args
    const head = line.match(/^([a-zA-Z_][a-zA-Z0-9_]*)\s*(.*)$/);
    if (!head) {
      errors.push(`line ${lineNo + 1}: cannot parse "${line}"`);
      continue;
    }
    // Both capture groups are mandatory in the regex above.
    const mnemonic = head[1]!.toLowerCase();
    const argsRaw = head[2]!.trim();
    const op = OPCODES[mnemonic];
    if (!op) {
      errors.push(`line ${lineNo + 1}: unknown mnemonic "${mnemonic}"`);
      continue;
    }
    const args = argsRaw.length > 0 ? argsRaw.split(',').map((s) => s.trim()) : [];
    if (args.length !== op.args) {
      errors.push(
        `line ${lineNo + 1}: ${mnemonic} expects ${op.args} arg(s), got ${args.length}`,
      );
      continue;
    }

    tokens.push({ kind: 'instr', code: op.code, args, lineNo });
    slotOffset += 1 + op.args;
  }

  // ---- Pass 2: emit slots --------------------------------------------------

  const slots = new Uint32Array(slotOffset);
  let cursor = 0;
  for (const t of tokens) {
    if (t.kind === 'raw') {
      slots[cursor++] = t.value;
      continue;
    }
    slots[cursor++] = t.code;
    for (const arg of t.args) {
      const v = resolveOperand(arg, labels);
      if (v === null) {
        errors.push(`line ${t.lineNo + 1}: cannot resolve "${arg}"`);
        slots[cursor++] = 0;
      } else {
        slots[cursor++] = v;
      }
    }
  }

  return { slots, errors };
}

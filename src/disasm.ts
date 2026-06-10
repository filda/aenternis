// Aenternis disassembler.
//
// Inverse of `src/asm.ts`: takes a `Uint32Array` (or any `ArrayLike`)
// of memory slots and renders them as a multi-line disassembly with
// optional PC marker. Used by the inspector to render the memory tail
// as a program listing instead of (or alongside) the raw hex dump.
//
// Variable-length instructions: a known opcode consumes `1 + args`
// slots. If the opcode is unknown (> 0x13, the VM's `Opcode::MAX`) or the
// instruction would overrun the slot array, the slot is rendered as a
// single `raw 0x…`
// line. This is more conservative than the VM (which treats unknown
// opcodes as nop) — for a static listing, "raw" honestly conveys "I
// don't know what this is", which matches how the suffix slots (RNG
// noise after the user's program prefix) actually look.
//
// PC marker: if `options.pc` is supplied and falls anywhere inside the
// slot range of an instruction (`[i, i + 1 + args)`), that line is
// prefixed with `> `; other lines start with `  ` so columns align.

import { OPCODES } from './asm.ts';

interface DecodedOp {
  readonly mnemonic: string;
  readonly args: number;
  readonly directionArgs: readonly number[];
}

const DIR_NAMES: readonly string[] = Object.freeze(['xp', 'xn', 'yp', 'yn', 'zp', 'zn']);

// Direction is always the first arg in these mnemonics (`docs/vm.md`).
const DIR_ARG_OPS: ReadonlySet<string> = new Set([
  'setp', 'getp', 'port', 'senergy', 'setpv',
]);

const OPS_BY_CODE: ReadonlyMap<number, DecodedOp> = (() => {
  const map = new Map<number, DecodedOp>();
  for (const [mnemonic, { code, args }] of Object.entries(OPCODES)) {
    map.set(code, {
      mnemonic,
      args,
      directionArgs: DIR_ARG_OPS.has(mnemonic) ? [0] : [],
    });
  }
  return map;
})();

export interface DisasmOptions {
  /** Program-counter slot index. Lines whose instruction range contains
   *  this index are prefixed with `> `. */
  readonly pc?: number;
}

/** Render an integer operand. Single-digit values decimal, anything
 *  larger as hex. This keeps direction indices, tight jump targets,
 *  and small constants as decimal while addresses (`0x10`, `0x20`),
 *  byte constants (`0xDE`), and 32-bit sentinels (`0xDEADBEEF`) stay
 *  in their natural hex form — matching how prototype-9 presets were
 *  written. */
function formatValue(v: number): string {
  if (v < 10) return v.toString(10);
  return `0x${v.toString(16)}`;
}

/** Disassemble a slot array into a printable multi-line listing.
 *  Returns an empty string for empty input. */
export function disassemble(slots: ArrayLike<number>, options: DisasmOptions = {}): string {
  if (slots.length === 0) return '';
  const pc = options.pc;
  const lines: string[] = [];
  let i = 0;
  while (i < slots.length) {
    // Loop bounds guarantee slots[i] is defined.
    const slot = slots[i]! >>> 0;
    const opcode = slot & 0xff;
    const op = OPS_BY_CODE.get(opcode);
    const addr = i.toString(16).padStart(4, '0');

    let nextI: number;
    let body: string;
    if (op && i + 1 + op.args <= slots.length) {
      const argParts: string[] = [];
      for (let j = 0; j < op.args; j += 1) {
        // Bounds check above guarantees slots[i+1+j] is defined.
        const argVal = slots[i + 1 + j]! >>> 0;
        if (op.directionArgs.includes(j) && argVal < DIR_NAMES.length) {
          argParts.push(DIR_NAMES[argVal]!);
        } else {
          argParts.push(formatValue(argVal));
        }
      }
      body = op.args === 0 ? op.mnemonic : `${op.mnemonic} ${argParts.join(', ')}`;
      nextI = i + 1 + op.args;
    } else {
      body = `raw 0x${slot.toString(16).padStart(8, '0')}`;
      nextI = i + 1;
    }
    const pcMarker = pc !== undefined && pc >= i && pc < nextI ? '> ' : '  ';
    lines.push(`${pcMarker}${addr}: ${body}`);
    i = nextI;
  }
  return lines.join('\n');
}

import { describe, it, expect } from 'vitest';
import { disassemble } from '../src/disasm.ts';
import { OPCODES, assemble } from '../src/asm.ts';

describe('disassemble — empty / trivial', () => {
  it('returns an empty string for empty input', () => {
    expect(disassemble([])).toBe('');
    expect(disassemble(new Uint32Array(0))).toBe('');
  });

  it('renders nop without args', () => {
    expect(disassemble([OPCODES.nop!.code])).toBe('  0000: nop');
  });

  it('prefixes every line with two-space padding when no PC supplied', () => {
    const slots = [OPCODES.nop!.code, OPCODES.nop!.code];
    const lines = disassemble(slots).split('\n');
    expect(lines).toHaveLength(2);
    for (const line of lines) {
      expect(line.startsWith('  ')).toBe(true);
    }
  });
});

describe('disassemble — known opcodes', () => {
  it('formats set a, v with single-digit values as decimal', () => {
    const slots = [OPCODES.set!.code, 5, 9];
    expect(disassemble(slots)).toBe('  0000: set 5, 9');
  });

  it('switches to hex for two-digit-and-up values', () => {
    const slots = [OPCODES.set!.code, 5, 42];
    expect(disassemble(slots)).toBe('  0000: set 5, 0x2a');
  });

  it('formats set with large values as hex', () => {
    const slots = [OPCODES.set!.code, 0x100, 0xDEADBEEF];
    expect(disassemble(slots)).toBe('  0000: set 0x100, 0xdeadbeef');
  });

  it('renders direction operands by name for d < 6', () => {
    // setp xp, 0
    expect(disassemble([OPCODES.setp!.code, 0, 0])).toBe('  0000: setp xp, 0');
    // setp zn, 0
    expect(disassemble([OPCODES.setp!.code, 5, 0])).toBe('  0000: setp zn, 0');
  });

  it('falls back to numeric for direction operand >= 6', () => {
    // VM treats d mod DIRS, but disasm preserves the raw value when out of range.
    const out = disassemble([OPCODES.setp!.code, 6, 0]);
    expect(out).toBe('  0000: setp 6, 0');
  });

  it('formats three-arg instructions (je) with single-digit operands', () => {
    const slots = [OPCODES.je!.code, 1, 2, 5];
    expect(disassemble(slots)).toBe('  0000: je 1, 2, 5');
  });

  it('chains multiple instructions with address increments', () => {
    const slots = [
      OPCODES.inc!.code, 0x10,
      OPCODES.jmp!.code, 0,
    ];
    expect(disassemble(slots)).toBe('  0000: inc 0x10\n  0002: jmp 0');
  });

  it('renders single-arg opcodes', () => {
    // sid 5
    expect(disassemble([OPCODES.sid!.code, 5])).toBe('  0000: sid 5');
    // paint 0xCAFE
    expect(disassemble([OPCODES.paint!.code, 0xCAFE])).toBe('  0000: paint 0xcafe');
  });

  it('renders the bitwise/arith opcodes added for opcode density', () => {
    // 0x14–0x1e are real opcodes now (and, or, xor, not, …, jp, jn).
    // Given enough slots they render as mnemonics.
    expect(disassemble([OPCODES.and!.code, 3, 4])).toBe('  0000: and 3, 4');
    expect(disassemble([OPCODES.not!.code, 5])).toBe('  0000: not 5');
    expect(disassemble([OPCODES.jp!.code, 1, 2])).toBe('  0000: jp 1, 2');
  });

  it('hex/decimal boundary at 10', () => {
    expect(disassemble([OPCODES.inc!.code, 9])).toBe('  0000: inc 9');
    expect(disassemble([OPCODES.inc!.code, 10])).toBe('  0000: inc 0xa');
  });

  it('uses opcode low byte only — upper bits are decorative', () => {
    // 0xDEADBE00 has opcode 0x00 = nop in low byte.
    expect(disassemble([0xDEADBE00])).toBe('  0000: nop');
  });
});

describe('disassemble — fold + raw fallback', () => {
  it('folds a high byte onto its real opcode (byte % COUNT)', () => {
    // No "unknown opcode" exists post-fold: 0xff & 0xff = 255; 255 % 31 = 7
    // = jmp (1 arg). With a following slot it renders the folded mnemonic.
    expect(disassemble([0xFF, 0])).toBe('  0000: jmp 0');
    // 0x20 = 32; 32 % 31 = 1 = set (2 args).
    expect(disassemble([0x20, 5, 9])).toBe('  0000: set 5, 9');
  });

  it('renders a truncated tail as raw', () => {
    // set expects 2 args but only one slot follows → cannot complete.
    const slots = [OPCODES.set!.code, 42];
    const lines = disassemble(slots).split('\n');
    expect(lines).toEqual([
      '  0000: raw 0x00000001',
      '  0001: raw 0x0000002a',
    ]);
  });

  it('renders a high-arg opcode at the dump tail as raw (truncated)', () => {
    // je needs 3 args; alone it cannot complete, so it renders raw rather
    // than reading past the end of the slot array.
    expect(disassemble([OPCODES.je!.code])).toBe('  0000: raw 0x0000000e');
  });
});

describe('disassemble — PC marker', () => {
  it('prefixes the instruction containing PC with "> "', () => {
    const slots = [
      OPCODES.inc!.code, 0x10,     // 0000..0001
      OPCODES.jmp!.code, 0,        // 0002..0003
    ];
    const out = disassemble(slots, { pc: 2 });
    expect(out).toBe('  0000: inc 0x10\n> 0002: jmp 0');
  });

  it('marks the instruction even when PC lands inside its arg range', () => {
    // je is 4 slots; PC=3 is in its range [0, 4).
    const slots = [OPCODES.je!.code, 1, 2, 5];
    const out = disassemble(slots, { pc: 3 });
    expect(out).toBe('> 0000: je 1, 2, 5');
  });

  it('PC past end of slots produces no marker', () => {
    const slots = [OPCODES.nop!.code];
    const out = disassemble(slots, { pc: 7 });
    expect(out).toBe('  0000: nop');
  });

  it('PC on a truncated (raw) slot marks just that line', () => {
    // je (3 args) twice at the tail → both truncate to raw; pc=1 marks 2nd.
    const slots = [OPCODES.je!.code, OPCODES.je!.code];
    const out = disassemble(slots, { pc: 1 });
    expect(out).toBe('  0000: raw 0x0000000e\n> 0001: raw 0x0000000e');
  });

  it('PC = 0 is correctly identified as the first instruction', () => {
    const slots = [OPCODES.nop!.code, OPCODES.nop!.code];
    const out = disassemble(slots, { pc: 0 });
    expect(out).toBe('> 0000: nop\n  0001: nop');
  });
});

describe('disassemble — round-trip with assembler', () => {
  it('counter preset round-trips to assemble', () => {
    const src = 'loop:\n  inc 0x10\n  jmp loop';
    const { slots, errors } = assemble(src);
    expect(errors).toEqual([]);
    expect(disassemble(slots)).toBe('  0000: inc 0x10\n  0002: jmp 0');
  });

  it('self_xp replicator round-trips', () => {
    const src = 'start:\n  setp xp, start\n  jmp start';
    const { slots, errors } = assemble(src);
    expect(errors).toEqual([]);
    expect(disassemble(slots)).toBe('  0000: setp xp, 0\n  0003: jmp 0');
  });

  it('roundtrips across all opcodes covered by direction-arg branch', () => {
    const src = [
      'setp xp, 0',
      'getp xn, 1',
      'port yp, 2',
      'senergy yn, 3',
      'setpv zp, 4',
    ].join('\n');
    const { slots, errors } = assemble(src);
    expect(errors).toEqual([]);
    const lines = disassemble(slots).split('\n');
    expect(lines).toEqual([
      '  0000: setp xp, 0',
      '  0003: getp xn, 1',
      '  0006: port yp, 2',
      '  0009: senergy yn, 3',
      '  000c: setpv zp, 4',
    ]);
  });
});

describe('disassemble — Uint32Array input', () => {
  it('accepts a Uint32Array directly', () => {
    const slots = new Uint32Array([OPCODES.nop!.code, OPCODES.inc!.code, 0]);
    expect(disassemble(slots)).toBe('  0000: nop\n  0001: inc 0');
  });
});

describe('disassemble — address padding', () => {
  it('pads addresses to four hex digits even past 0xFFF', () => {
    const slots = new Array<number>(0x1001).fill(OPCODES.nop!.code);
    const lines = disassemble(slots).split('\n');
    expect(lines[0]).toBe('  0000: nop');
    expect(lines[0x1000]).toBe('  1000: nop');
  });
});

import { describe, it, expect } from 'vitest';
import { OPCODES, DIRECTIONS, resolveOperand, assemble } from '../src/asm.ts';

describe('OPCODES table', () => {
  it('has 23 entries matching the Rust VM', () => {
    expect(Object.keys(OPCODES)).toHaveLength(23);
  });

  it('opcode codes are unique', () => {
    const codes = Object.values(OPCODES).map((o) => o.code);
    expect(new Set(codes).size).toBe(codes.length);
  });

  it('each opcode has a non-negative arg count', () => {
    for (const op of Object.values(OPCODES)) {
      expect(op.args).toBeGreaterThanOrEqual(0);
    }
  });
});

describe('DIRECTIONS', () => {
  it('uses canonical xp/xn/yp/yn/zp/zn ordering', () => {
    expect(DIRECTIONS).toEqual({
      xp: 0, xn: 1, yp: 2, yn: 3, zp: 4, zn: 5,
    });
  });
});

describe('resolveOperand', () => {
  const labels = new Map<string, number>([['start', 0], ['loop', 5]]);

  it('resolves direction names case-insensitively', () => {
    expect(resolveOperand('xp', labels)).toBe(0);
    expect(resolveOperand('Yn', labels)).toBe(3);
    expect(resolveOperand('ZN', labels)).toBe(5);
  });

  it('resolves hex literals', () => {
    expect(resolveOperand('0x1A', labels)).toBe(26);
    expect(resolveOperand('0XFF', labels)).toBe(255);
    expect(resolveOperand('0xCAFE', labels)).toBe(0xCAFE);
  });

  it('resolves decimal literals', () => {
    expect(resolveOperand('42', labels)).toBe(42);
    expect(resolveOperand('0', labels)).toBe(0);
  });

  it("wraps negative decimals to two's complement", () => {
    expect(resolveOperand('-1', labels)).toBe(0xFFFFFFFF);
    expect(resolveOperand('-2', labels)).toBe(0xFFFFFFFE);
  });

  it('resolves label references', () => {
    expect(resolveOperand('start', labels)).toBe(0);
    expect(resolveOperand('loop', labels)).toBe(5);
  });

  it('returns null on unknown identifiers', () => {
    expect(resolveOperand('unknown_label', labels)).toBeNull();
  });

  it('returns null on garbage', () => {
    expect(resolveOperand('0x', labels)).toBeNull();
    expect(resolveOperand('123abc', labels)).toBeNull();
    expect(resolveOperand('', labels)).toBeNull();
  });
});

describe('assemble — empty inputs', () => {
  it('empty string produces no slots and no errors', () => {
    const { slots, errors } = assemble('');
    expect(Array.from(slots)).toEqual([]);
    expect(errors).toEqual([]);
  });

  it('only comments produces no slots', () => {
    const { slots, errors } = assemble('; comment\n  ; another\n');
    expect(Array.from(slots)).toEqual([]);
    expect(errors).toEqual([]);
  });
});

describe('assemble — raw slots', () => {
  it('emits one slot per number line', () => {
    const { slots, errors } = assemble('0x09\n0\n0\n0x07\n0');
    expect(Array.from(slots)).toEqual([0x09, 0, 0, 0x07, 0]);
    expect(errors).toEqual([]);
  });

  it('accepts decimal and hex on adjacent lines', () => {
    const { slots, errors } = assemble('42\n0xCAFE\n-1');
    expect(Array.from(slots)).toEqual([42, 0xCAFE, 0xFFFFFFFF]);
    expect(errors).toEqual([]);
  });
});

describe('assemble — instructions', () => {
  it('emits a 2-slot inc with operand', () => {
    const { slots, errors } = assemble('inc 5');
    expect(Array.from(slots)).toEqual([0x05, 5]);
    expect(errors).toEqual([]);
  });

  it('emits a 3-slot set with two operands', () => {
    const { slots, errors } = assemble('set 0, 0xCAFE');
    expect(Array.from(slots)).toEqual([0x01, 0, 0xCAFE]);
    expect(errors).toEqual([]);
  });

  it('emits a 3-slot port with direction shorthand', () => {
    const { slots, errors } = assemble('port xp, 10');
    expect(Array.from(slots)).toEqual([0x0b, 0, 10]);
    expect(errors).toEqual([]);
  });

  it('emits a 4-slot je', () => {
    const { slots, errors } = assemble('je 0, 1, 2');
    expect(Array.from(slots)).toEqual([0x0e, 0, 1, 2]);
    expect(errors).toEqual([]);
  });

  it('emits a 1-slot nop', () => {
    const { slots, errors } = assemble('nop');
    expect(Array.from(slots)).toEqual([0x00]);
    expect(errors).toEqual([]);
  });
});

describe('assemble — labels', () => {
  it('resolves a forward label reference', () => {
    // start: jmp end ; (jmp is 2 slots → end = 2)
    // end: nop
    const { slots, errors } = assemble('start:\n  jmp end\nend:\n  nop');
    expect(Array.from(slots)).toEqual([0x07, 2, 0x00]);
    expect(errors).toEqual([]);
  });

  it('resolves a self-referential loop', () => {
    const { slots, errors } = assemble('start:\n  setp xp, start\n  jmp start');
    // setp = 0x09, args [d=0 (xp), v=0 (start label)] → 0x09, 0, 0
    // jmp = 0x07, args [a=0 (start)] → 0x07, 0
    expect(Array.from(slots)).toEqual([0x09, 0, 0, 0x07, 0]);
    expect(errors).toEqual([]);
  });

  it('rejects duplicate labels', () => {
    const { slots, errors } = assemble('foo:\n  nop\nfoo:\n  nop');
    expect(slots).toHaveLength(2); // nop, nop emitted regardless
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/duplicate label/);
  });

  it('rejects unknown label references', () => {
    const { errors } = assemble('jmp nowhere');
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/cannot resolve/);
  });

  it('label on a line by itself is allowed', () => {
    const { slots, errors } = assemble('start:\n  nop');
    expect(Array.from(slots)).toEqual([0x00]);
    expect(errors).toEqual([]);
  });
});

describe('assemble — error reporting', () => {
  it('reports unknown mnemonic with line number', () => {
    const { errors } = assemble('nop\nbogus 1, 2\nnop');
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/line 2/);
    expect(errors[0]).toMatch(/unknown mnemonic/);
  });

  it('reports wrong arg count', () => {
    const { errors } = assemble('set 1');
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatch(/expects 2 arg/);
  });

  it('partial output is preserved on error', () => {
    const { slots, errors } = assemble('nop\nbogus\nnop');
    // Two valid nop's, one error in the middle.
    expect(Array.from(slots)).toEqual([0x00, 0x00]);
    expect(errors).toHaveLength(1);
  });
});

describe('assemble — comments and whitespace', () => {
  it('strips trailing line comments', () => {
    const { slots, errors } = assemble('nop ; this is a nop');
    expect(Array.from(slots)).toEqual([0x00]);
    expect(errors).toEqual([]);
  });

  it('ignores leading whitespace', () => {
    const { slots, errors } = assemble('    nop\n\t\tnop');
    expect(Array.from(slots)).toEqual([0x00, 0x00]);
    expect(errors).toEqual([]);
  });

  it('blank lines do not emit slots', () => {
    const { slots, errors } = assemble('nop\n\n\n\nnop');
    expect(Array.from(slots)).toEqual([0x00, 0x00]);
    expect(errors).toEqual([]);
  });
});

describe('assemble — return type', () => {
  it('returns a Uint32Array', () => {
    const { slots } = assemble('nop');
    expect(slots).toBeInstanceOf(Uint32Array);
  });
});

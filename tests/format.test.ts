import { describe, it, expect } from 'vitest';
import { DIR_LABELS, fmtBbox, fmtDirArr, fmtMemoryHexDump } from '../src/format.ts';

describe('DIR_LABELS', () => {
  it('uses the canonical xp/xn/yp/yn/zp/zn order', () => {
    expect(DIR_LABELS).toEqual(['xp', 'xn', 'yp', 'yn', 'zp', 'zn']);
  });
});

describe('fmtDirArr', () => {
  it('formats a number array with directional labels', () => {
    expect(fmtDirArr([1, 2, 3, 4, 5, 6])).toBe('xp=1  xn=2  yp=3  yn=4  zp=5  zn=6');
  });

  it('works with a Uint32Array', () => {
    expect(fmtDirArr(new Uint32Array([7, 8, 9, 10, 11, 12]))).toBe('xp=7  xn=8  yp=9  yn=10  zp=11  zn=12');
  });

  it('uses two-space separators', () => {
    const out = fmtDirArr([0, 0, 0, 0, 0, 0]);
    expect(out.split('  ')).toHaveLength(6);
  });
});

describe('fmtMemoryHexDump', () => {
  it('returns an empty string for empty input', () => {
    expect(fmtMemoryHexDump([])).toBe('');
    expect(fmtMemoryHexDump(new Uint32Array(0))).toBe('');
  });

  it('formats a single short row with leading address', () => {
    expect(fmtMemoryHexDump([0x09])).toBe('0000: 00000009');
  });

  it('packs 8 slots per row with one-space separators', () => {
    const slots = [0, 1, 2, 3, 4, 5, 6, 7];
    expect(fmtMemoryHexDump(slots)).toBe(
      '0000: 00000000 00000001 00000002 00000003 00000004 00000005 00000006 00000007',
    );
  });

  it('breaks rows after 8 slots and bumps the address to 0008', () => {
    const slots = [0, 1, 2, 3, 4, 5, 6, 7, 0xCAFE];
    const lines = fmtMemoryHexDump(slots).split('\n');
    expect(lines).toHaveLength(2);
    expect(lines[1]).toBe('0008: 0000cafe');
  });

  it('pads addresses to four hex digits', () => {
    const slots = new Array<number>(0x1F + 1).fill(0);
    const lines = fmtMemoryHexDump(slots).split('\n');
    expect(lines[0]?.startsWith('0000:')).toBe(true);
    expect(lines[lines.length - 1]?.startsWith('0018:')).toBe(true);
  });

  it('renders each value as 8-digit lowercase hex', () => {
    expect(fmtMemoryHexDump([0xDEADBEEF])).toBe('0000: deadbeef');
  });
});

describe('fmtBbox', () => {
  it('returns null for an empty bbox', () => {
    expect(fmtBbox(new Int32Array(0))).toBeNull();
  });

  it('returns null when length is not 6', () => {
    expect(fmtBbox([0, 0, 0])).toBeNull();
    expect(fmtBbox([1, 2, 3, 4, 5, 6, 7])).toBeNull();
  });

  it('formats a non-empty bbox with span dims', () => {
    expect(fmtBbox([0, 4, -1, 1, 2, 5])).toBe('(0..4, -1..1, 2..5) = 5×3×4');
  });

  it('handles a single-cell bbox (1×1×1)', () => {
    expect(fmtBbox([3, 3, 3, 3, 3, 3])).toBe('(3..3, 3..3, 3..3) = 1×1×1');
  });
});

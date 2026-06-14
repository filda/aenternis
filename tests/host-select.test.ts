import { describe, it, expect } from 'vitest';
import { findHost } from '../src/host-select.ts';

const STRIDE = 6;

/** Build a flat snapshot (stride 6) from `[x, y, z, energy]` records;
 *  origin_tag / appearance are zero-filled. Coords are written as their
 *  u32 bit pattern so negatives round-trip through the `| 0` decode. */
function snapshot(
  cells: ReadonlyArray<readonly [number, number, number, number]>,
): Uint32Array {
  const out = new Uint32Array(cells.length * STRIDE);
  cells.forEach(([x, y, z, e], i) => {
    out[i * STRIDE] = x >>> 0;
    out[i * STRIDE + 1] = y >>> 0;
    out[i * STRIDE + 2] = z >>> 0;
    out[i * STRIDE + 3] = e;
  });
  return out;
}

describe('findHost', () => {
  it('returns null for an empty world', () => {
    expect(findHost(new Uint32Array(0), STRIDE, { codeLen: 1, reserve: 0 })).toBeNull();
  });

  it('picks the only eligible cell', () => {
    const snap = snapshot([[1, 2, 3, 50]]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 1, y: 2, z: 3 });
  });

  it('returns null when no cell has enough energy', () => {
    const snap = snapshot([
      [1, 2, 3, 5],
      [4, 5, 6, 9],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toBeNull();
  });

  it('counts the reserve toward the requirement', () => {
    const snap = snapshot([[1, 2, 3, 12]]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 3 })).toBeNull(); // need 13 > 12
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 2 })).toEqual({ x: 1, y: 2, z: 3 }); // need 12
  });

  it('accepts a cell whose energy exactly meets the requirement', () => {
    const snap = snapshot([[7, 7, 7, 12]]);
    expect(findHost(snap, STRIDE, { codeLen: 12, reserve: 0 })).toEqual({ x: 7, y: 7, z: 7 });
  });

  it('picks a sole eligible host even at the minimum energy of 1', () => {
    // Pins the `bestEnergy = -1` sentinel: a `0` or `+1` start would
    // fail the `energy > bestEnergy` test for this energy-1 host.
    const snap = snapshot([[5, 5, 5, 1]]);
    expect(findHost(snap, STRIDE, { codeLen: 1, reserve: 0 })).toEqual({ x: 5, y: 5, z: 5 });
  });

  it('picks the largest-energy eligible cell', () => {
    const snap = snapshot([
      [1, 0, 0, 20],
      [2, 0, 0, 80],
      [3, 0, 0, 50],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 2, y: 0, z: 0 });
  });

  it('breaks energy ties toward the first (lex-smallest) cell', () => {
    // The snapshot is emitted lex-sorted; first-seen wins on ties.
    const snap = snapshot([
      [1, 0, 0, 60],
      [2, 0, 0, 60],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 1, reserve: 0 })).toEqual({ x: 1, y: 0, z: 0 });
  });

  it('decodes negative coordinates from their u32 bit pattern', () => {
    const snap = snapshot([[-3, -1, -100, 40]]);
    expect(findHost(snap, STRIDE, { codeLen: 1, reserve: 0 })).toEqual({
      x: -3,
      y: -1,
      z: -100,
    });
  });
});

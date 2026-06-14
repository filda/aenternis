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

  it('picks the eligible cell farthest from the center of mass', () => {
    // A heavy core near x=0 anchors the COM; among eligible cells the
    // farthest one wins regardless of its own (lower) energy.
    const snap = snapshot([
      [0, 0, 0, 1000], // core: pulls COM to ~x=5
      [10, 0, 0, 50],
      [100, 0, 0, 50], // farthest eligible
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 100, y: 0, z: 0 });
  });

  it('weights the center of mass by energy (heavy side pulls it)', () => {
    // Huge mass at x=100 drags the COM to ~99.8, so the small eligible cell
    // at x=0 is the farthest — not the one nearest in raw coordinate terms.
    const snap = snapshot([
      [0, 0, 0, 20],
      [100, 0, 0, 10000],
      [110, 0, 0, 20],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 15, reserve: 0 })).toEqual({ x: 0, y: 0, z: 0 });
  });

  it('uses the y component of the energy-weighted COM', () => {
    // Heavy mass high in +y pulls the COM to ~95.5; the candidate at y=0 is
    // therefore the farthest. Any corruption of the y accumulation moves the
    // COM and flips the winner.
    const snap = snapshot([
      [0, 0, 0, 50],
      [0, 90, 0, 50],
      [0, 100, 0, 1000],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 0, y: 0, z: 0 });
  });

  it('uses the z component of the energy-weighted COM', () => {
    const snap = snapshot([
      [0, 0, 0, 50],
      [0, 0, 90, 50],
      [0, 0, 100, 1000],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 0, y: 0, z: 0 });
  });

  it('divides the y COM by total energy (not multiplies)', () => {
    // Correct COM.y ≈ 63.6 makes the y=200 cell the farthest. A `/=`→`*=`
    // bug explodes COM.y, which would make the y=0 cell win instead.
    const snap = snapshot([
      [0, 0, 0, 50],
      [0, 60, 0, 1000],
      [0, 200, 0, 50],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 0, y: 200, z: 0 });
  });

  it('divides the z COM by total energy (not multiplies)', () => {
    const snap = snapshot([
      [0, 0, 0, 50],
      [0, 0, 60, 1000],
      [0, 0, 200, 50],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 10, reserve: 0 })).toEqual({ x: 0, y: 0, z: 200 });
  });

  it('picks a sole eligible host even when it sits at the COM (dist 0)', () => {
    // Single cell IS the COM (dist² = 0); pins the `bestDist = -1` sentinel
    // against a `0`/`+1` regression.
    const snap = snapshot([[5, 5, 5, 50]]);
    expect(findHost(snap, STRIDE, { codeLen: 1, reserve: 0 })).toEqual({ x: 5, y: 5, z: 5 });
  });

  it('breaks distance ties toward the first (lex-smallest) cell', () => {
    // Symmetric about the COM (x=0): both at distance 5; first seen wins.
    const snap = snapshot([
      [-5, 0, 0, 50],
      [5, 0, 0, 50],
    ]);
    expect(findHost(snap, STRIDE, { codeLen: 1, reserve: 0 })).toEqual({ x: -5, y: 0, z: 0 });
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

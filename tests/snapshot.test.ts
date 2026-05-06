import { describe, it, expect } from 'vitest';
import { analyzeSnapshot, cellAt } from '../src/snapshot.ts';

// Convenience builders so tests can describe cells declaratively.
function makeSnap(cells: ReadonlyArray<readonly [x: number, y: number, z: number, e: number]>, stride = 4): Uint32Array {
  const snap = new Uint32Array(cells.length * stride);
  for (let i = 0; i < cells.length; i += 1) {
    const [x, y, z, e] = cells[i]!;
    snap[i * stride] = x;
    snap[i * stride + 1] = y;
    snap[i * stride + 2] = z;
    snap[i * stride + 3] = e;
  }
  return snap;
}

describe('cellAt', () => {
  it('reads the i-th record', () => {
    const snap = makeSnap([[1, 2, 3, 100], [4, 5, 6, 200]]);
    expect(cellAt(snap, 4, 0)).toEqual({ x: 1, y: 2, z: 3, energy: 100 });
    expect(cellAt(snap, 4, 1)).toEqual({ x: 4, y: 5, z: 6, energy: 200 });
  });

  it('respects a non-default stride', () => {
    const snap = new Uint32Array([1, 2, 3, 100, 99, 99, 4, 5, 6, 200, 99, 99]);
    expect(cellAt(snap, 6, 1)).toEqual({ x: 4, y: 5, z: 6, energy: 200 });
  });
});

describe('analyzeSnapshot', () => {
  it('returns null bbox and -1 for an empty snapshot', () => {
    const r = analyzeSnapshot(new Uint32Array(0), 4, 0, false);
    expect(r.bbox).toBeNull();
    expect(r.maxCellIdx).toBe(-1);
    expect(r.maxEnergy).toBe(0);
  });

  it('computes a tight bbox over all cells', () => {
    const snap = makeSnap([[0, 0, 0, 1], [3, 5, -2, 1], [-1, 4, 1, 1]]);
    const r = analyzeSnapshot(snap, 4, 3, false);
    expect(r.bbox).toEqual({ minX: -1, maxX: 3, minY: 0, maxY: 5, minZ: -2, maxZ: 1 });
  });

  it('finds the max-energy cell index', () => {
    const snap = makeSnap([[0, 0, 0, 5], [1, 0, 0, 9], [2, 0, 0, 7]]);
    const r = analyzeSnapshot(snap, 4, 3, false);
    expect(r.maxCellIdx).toBe(1);
    expect(r.maxEnergy).toBe(9);
  });

  it('keeps the first match on tied max energies', () => {
    const snap = makeSnap([[0, 0, 0, 5], [1, 0, 0, 5]]);
    const r = analyzeSnapshot(snap, 4, 2, false);
    expect(r.maxCellIdx).toBe(0);
  });

  it('considers all cells when slice is disabled', () => {
    const snap = makeSnap([[0, 0, 0, 1], [0, 0, 1, 99]]);
    const r = analyzeSnapshot(snap, 4, 2, false);
    expect(r.maxCellIdx).toBe(1);
  });

  it('skips z!=0 cells when slice is enabled', () => {
    const snap = makeSnap([[0, 0, 0, 1], [0, 0, 1, 99], [5, 5, 0, 7]]);
    const r = analyzeSnapshot(snap, 4, 3, true);
    expect(r.maxCellIdx).toBe(2);
    expect(r.maxEnergy).toBe(7);
    expect(r.bbox).toEqual({ minX: 0, maxX: 5, minY: 0, maxY: 5, minZ: 0, maxZ: 0 });
  });

  it('returns null bbox and -1 when slice hides everything', () => {
    const snap = makeSnap([[0, 0, 1, 1], [0, 0, 2, 1]]);
    const r = analyzeSnapshot(snap, 4, 2, true);
    expect(r.bbox).toBeNull();
    expect(r.maxCellIdx).toBe(-1);
    expect(r.maxEnergy).toBe(0);
  });

  it('respects cellCount even if the snapshot has trailing slots', () => {
    const snap = makeSnap([[0, 0, 0, 1], [9, 9, 9, 999]]);
    const r = analyzeSnapshot(snap, 4, 1, false);
    expect(r.maxEnergy).toBe(1);
    expect(r.maxCellIdx).toBe(0);
  });
});

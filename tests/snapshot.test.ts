import { describe, it, expect } from 'vitest';
import {
  analyzeLineage,
  analyzeSnapshot,
  cellAt,
  findMaxEnergyIdxByTag,
} from '../src/snapshot.ts';

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

describe('findMaxEnergyIdxByTag', () => {
  const STRIDE = 6;

  /** Build a stride-6 snapshot from `[x, y, z, energy, tag]` records
   *  (appearance is zero-filled). */
  function taggedSnap(
    cells: ReadonlyArray<readonly [number, number, number, number, number]>,
  ): Uint32Array {
    const snap = new Uint32Array(cells.length * STRIDE);
    cells.forEach(([x, y, z, e, tag], i) => {
      snap[i * STRIDE] = x >>> 0;
      snap[i * STRIDE + 1] = y >>> 0;
      snap[i * STRIDE + 2] = z >>> 0;
      snap[i * STRIDE + 3] = e;
      snap[i * STRIDE + 4] = tag;
    });
    return snap;
  }

  it('returns -1 for an empty snapshot', () => {
    expect(findMaxEnergyIdxByTag(new Uint32Array(0), STRIDE, 0, 0x7)).toBe(-1);
  });

  it('returns -1 when no cell carries the tag', () => {
    const snap = taggedSnap([
      [0, 0, 0, 50, 0x1],
      [1, 0, 0, 90, 0x2],
    ]);
    expect(findMaxEnergyIdxByTag(snap, STRIDE, 2, 0x7)).toBe(-1);
  });

  it('finds the sole carrier of the tag', () => {
    const snap = taggedSnap([
      [0, 0, 0, 50, 0x1],
      [1, 0, 0, 90, 0x7],
      [2, 0, 0, 80, 0x2],
    ]);
    expect(findMaxEnergyIdxByTag(snap, STRIDE, 3, 0x7)).toBe(1);
  });

  it('picks the highest-energy carrier, ignoring other tags', () => {
    const snap = taggedSnap([
      [0, 0, 0, 99, 0x2], // higher energy but wrong tag
      [1, 0, 0, 30, 0x7],
      [2, 0, 0, 70, 0x7], // max among the 0x7 carriers
      [3, 0, 0, 40, 0x7],
    ]);
    expect(findMaxEnergyIdxByTag(snap, STRIDE, 4, 0x7)).toBe(2);
  });

  it('finds a carrier even at the minimum energy of 1', () => {
    // Pins the `bestEnergy = -1` sentinel against a `0`/`+1` regression.
    const snap = taggedSnap([[5, 5, 5, 1, 0x7]]);
    expect(findMaxEnergyIdxByTag(snap, STRIDE, 1, 0x7)).toBe(0);
  });

  it('breaks energy ties toward the first carrier', () => {
    // Strict `>` keeps the first seen; `>=` would drift to the last.
    const snap = taggedSnap([
      [0, 0, 0, 50, 0x7],
      [1, 0, 0, 50, 0x7],
    ]);
    expect(findMaxEnergyIdxByTag(snap, STRIDE, 2, 0x7)).toBe(0);
  });
});

describe('analyzeLineage', () => {
  const ST = 6;

  function taggedSnap(
    cells: ReadonlyArray<readonly [number, number, number, number, number]>,
  ): Uint32Array {
    const snap = new Uint32Array(cells.length * ST);
    cells.forEach(([x, y, z, e, tag], i) => {
      snap[i * ST] = x >>> 0;
      snap[i * ST + 1] = y >>> 0;
      snap[i * ST + 2] = z >>> 0;
      snap[i * ST + 3] = e;
      snap[i * ST + 4] = tag;
    });
    return snap;
  }

  it('returns null when no cell carries the tag', () => {
    const snap = taggedSnap([
      [0, 0, 0, 50, 0x1],
      [1, 0, 0, 90, 0x2],
    ]);
    expect(analyzeLineage(snap, ST, 2, 0x7)).toBeNull();
  });

  it('returns null for an empty snapshot', () => {
    expect(analyzeLineage(new Uint32Array(0), ST, 0, 0x7)).toBeNull();
  });

  it('summarizes a single carrier', () => {
    const snap = taggedSnap([[5, -6, 7, 40, 0x7]]);
    const lin = analyzeLineage(snap, ST, 1, 0x7)!;
    expect(lin.count).toBe(1);
    expect(lin.sumEnergy).toBe(40);
    expect([lin.cx, lin.cy, lin.cz]).toEqual([5, -6, 7]);
    expect([lin.minX, lin.maxX, lin.minY, lin.maxY, lin.minZ, lin.maxZ]).toEqual([5, 5, -6, -6, 7, 7]);
    expect(lin.maxIdx).toBe(0);
  });

  it('counts, sums, bounds, weights and finds the torch across carriers', () => {
    const snap = taggedSnap([
      [10, 0, 0, 100, 0x7], // strongest → maxIdx 0; dominates the weighted centroid
      [3, 3, 3, 999, 0x2], // other tag — ignored
      [-6, 4, 0, 20, 0x7],
      [0, -2, 8, 20, 0x7],
    ]);
    const lin = analyzeLineage(snap, ST, 4, 0x7)!;
    expect(lin.count).toBe(3);
    expect(lin.sumEnergy).toBe(140);
    // Energy-weighted centroid (sum(coord*e)/sumE), per axis.
    expect(lin.cx).toBeCloseTo(880 / 140, 5); // (10*100 - 6*20 + 0*20)/140
    expect(lin.cy).toBeCloseTo(40 / 140, 5); //  (0*100 + 4*20 - 2*20)/140
    expect(lin.cz).toBeCloseTo(160 / 140, 5); // (0*100 + 0*20 + 8*20)/140
    // Bounding box over the three carriers (the 0x2 cell excluded).
    expect([lin.minX, lin.maxX]).toEqual([-6, 10]);
    expect([lin.minY, lin.maxY]).toEqual([-2, 4]);
    expect([lin.minZ, lin.maxZ]).toEqual([0, 8]);
    expect(lin.maxIdx).toBe(0); // the 100-energy carrier
  });

  it('breaks torch (maxIdx) ties toward the first carrier', () => {
    const snap = taggedSnap([
      [0, 0, 0, 50, 0x7],
      [9, 9, 9, 50, 0x7],
    ]);
    expect(analyzeLineage(snap, ST, 2, 0x7)!.maxIdx).toBe(0);
  });

  it('registers the torch even at the minimum energy of 1', () => {
    // Pins the `maxEnergy = -1` sentinel: a `+1` start would skip an
    // energy-1 carrier and leave maxIdx at -1.
    const snap = taggedSnap([[5, 5, 5, 1, 0x7]]);
    expect(analyzeLineage(snap, ST, 1, 0x7)!.maxIdx).toBe(0);
  });
});

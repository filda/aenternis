import { describe, it, expect } from 'vitest';
import { DIST_MULT, MIN_SPAN, Y_FACTOR, fitCamera } from '../src/camera-fit.ts';

describe('fitCamera', () => {
  it('targets the bbox center', () => {
    const r = fitCamera({ minX: 0, maxX: 10, minY: -4, maxY: 4, minZ: 2, maxZ: 6 });
    expect(r.target).toEqual([5, 0, 4]);
  });

  it('targets the bbox center for offset coordinates', () => {
    // Use non-zero-summing minY+maxY so a `+ → *` mutation on the
    // averaging formula moves the result.
    const r = fitCamera({ minX: 1, maxX: 5, minY: 6, maxY: 10, minZ: 100, maxZ: 200 });
    expect(r.target).toEqual([3, 8, 150]);
  });

  it('uses the largest axis span × DIST_MULT for distance', () => {
    // X span = 10 (largest), Y span = 4, Z span = 4 → dist = 10 × DIST_MULT.
    const r = fitCamera({ minX: 0, maxX: 10, minY: 0, maxY: 4, minZ: 0, maxZ: 4 });
    const [tx, ty, tz] = r.target;
    const [ex, ey, ez] = r.eye;
    const dist = 10 * DIST_MULT;
    expect(ex - tx).toBeCloseTo(dist);
    expect(ey - ty).toBeCloseTo(dist * Y_FACTOR);
    expect(ez - tz).toBeCloseTo(dist);
  });

  it('clamps the span to MIN_SPAN for a degenerate bbox', () => {
    const r = fitCamera({ minX: 5, maxX: 5, minY: 5, maxY: 5, minZ: 5, maxZ: 5 });
    const [tx] = r.target;
    const [ex] = r.eye;
    expect(ex - tx).toBeCloseTo(MIN_SPAN * DIST_MULT);
  });

  it('places the eye above the target by Y_FACTOR × dist', () => {
    const r = fitCamera({ minX: 0, maxX: 10, minY: 0, maxY: 0, minZ: 0, maxZ: 0 });
    const [, ty] = r.target;
    const [, ey] = r.eye;
    const dist = 10 * DIST_MULT;
    expect(ey - ty).toBeCloseTo(dist * Y_FACTOR);
  });
});

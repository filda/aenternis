import { describe, it, expect } from 'vitest';
import {
  HEAT_STOPS,
  MEAN_T,
  T_SPREAD,
  absoluteEnergyT,
  heatColor,
  meanRelativeT,
} from '../src/heat.ts';

describe('HEAT_STOPS', () => {
  it('starts at t=0 and ends at t=1', () => {
    expect(HEAT_STOPS[0]?.t).toBe(0);
    expect(HEAT_STOPS[HEAT_STOPS.length - 1]?.t).toBe(1);
  });

  it('is strictly sorted by t', () => {
    for (let i = 1; i < HEAT_STOPS.length; i += 1) {
      expect(HEAT_STOPS[i]!.t).toBeGreaterThan(HEAT_STOPS[i - 1]!.t);
    }
  });

  it('has 9 stops', () => {
    expect(HEAT_STOPS).toHaveLength(9);
  });

  it('keeps the low-energy region blue-dominated', () => {
    // The cool half of the ramp (below t=0.50) must read as blue —
    // typical low-energy entities live there and "energy is blue".
    for (const stop of HEAT_STOPS) {
      if (stop.t > 0 && stop.t < 0.50) {
        expect(stop.rgb[2]).toBeGreaterThan(stop.rgb[0]);
      }
    }
  });

  it('reaches a hot core (red-dominated) above t=0.65', () => {
    // The slice view exposes the densest energy region. Without a
    // warm core in the upper half of the ramp, hot spots would never
    // visibly differentiate from the cool floor.
    for (const stop of HEAT_STOPS) {
      if (stop.t >= 0.65 && stop.t < 1.00) {
        expect(stop.rgb[0]).toBeGreaterThan(stop.rgb[2]);
      }
    }
  });
});

describe('heatColor', () => {
  it('returns black at t=0', () => {
    expect(heatColor(0)).toEqual([0, 0, 0]);
  });

  it('returns white-hot at t=1', () => {
    // Last stop is [1.00, 1.00, 0.95] — a near-white with a faint
    // warm tint, not cold pure white.
    const [r, g, b] = heatColor(1);
    expect(r).toBeCloseTo(1, 10);
    expect(g).toBeCloseTo(1, 10);
    expect(b).toBeCloseTo(0.95, 10);
  });

  it('clamps t < 0 to 0', () => {
    expect(heatColor(-0.5)).toEqual(heatColor(0));
  });

  it('clamps t > 1 to 1', () => {
    expect(heatColor(1.5)).toEqual(heatColor(1));
  });

  it('returns the last-stop color for NaN input', () => {
    // NaN propagates through `Math.max(0, Math.min(1, NaN))` so the
    // segment search hits the end without breaking — the `a === b`
    // guard on the lerp expression keeps the result well-defined.
    const r = heatColor(NaN);
    expect(r[0]).toBeCloseTo(1, 10);
    expect(r[1]).toBeCloseTo(1, 10);
    expect(r[2]).toBeCloseTo(0.95, 10);
  });

  it('returns an exact stop value when t lands on a stop', () => {
    // Stop at t=0.30 is medium blue [0.25, 0.55, 0.90].
    const [r, g, b] = heatColor(0.30);
    expect(r).toBeCloseTo(0.25, 5);
    expect(g).toBeCloseTo(0.55, 5);
    expect(b).toBeCloseTo(0.90, 5);
  });

  it('linearly interpolates between adjacent stops', () => {
    // Halfway between t=0 (black [0,0,0]) and t=0.05 (very dark blue
    // [0.05, 0.10, 0.35]) is t=0.025.
    const [r, g, b] = heatColor(0.025);
    expect(r).toBeCloseTo(0.025, 5);
    expect(g).toBeCloseTo(0.05, 5);
    expect(b).toBeCloseTo(0.175, 5);
  });

  it('returns three components', () => {
    expect(heatColor(0.5)).toHaveLength(3);
  });

  it('produces blue-dominated colors in the low-energy region', () => {
    // A typical entity (~100 units against a 10M total) lands around
    // t≈0.29 — the resulting color should still read as blue (b > r).
    const [r, , b] = heatColor(0.29);
    expect(b).toBeGreaterThan(r);
  });

  it('produces red-dominated colors in the hot-core region', () => {
    // Hot spots (energy >> 100k of a 10M total) land around t≈0.7+.
    // A slice view should clearly show them in red/orange.
    const [r, , b] = heatColor(0.70);
    expect(r).toBeGreaterThan(b);
  });
});

describe('absoluteEnergyT', () => {
  it('returns 0 for zero energy', () => {
    expect(absoluteEnergyT(0, 1_000_000)).toBe(0);
  });

  it('returns 0 for negative energy', () => {
    expect(absoluteEnergyT(-5, 1_000_000)).toBe(0);
  });

  it('returns 1 when a cell holds the entire world energy', () => {
    expect(absoluteEnergyT(1_000_000, 1_000_000)).toBeCloseTo(1, 5);
  });

  it('compresses the range logarithmically — 100 / 10M ≈ t=0.286', () => {
    // log(101) / log(10_000_001) ≈ 4.6151 / 16.1181 ≈ 0.2864
    expect(absoluteEnergyT(100, 10_000_000)).toBeCloseTo(0.286, 2);
  });

  it('compresses the range logarithmically — 10 / 10M ≈ t=0.149', () => {
    // log(11) / log(10_000_001) ≈ 2.3979 / 16.1181 ≈ 0.1488
    expect(absoluteEnergyT(10, 10_000_000)).toBeCloseTo(0.149, 2);
  });

  it('floors totalEnergy at 1 to avoid log(0) blowup', () => {
    // With totalE = 0 the denominator falls back to log(1+1) = log(2);
    // a positive cell energy then maps to a finite, large t. We just
    // check the result is finite and non-negative.
    const t = absoluteEnergyT(5, 0);
    expect(Number.isFinite(t)).toBe(true);
    expect(t).toBeGreaterThan(0);
  });

  it('is monotonically non-decreasing in energy', () => {
    const totalE = 10_000_000;
    let last = 0;
    for (const e of [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000]) {
      const t = absoluteEnergyT(e, totalE);
      expect(t).toBeGreaterThanOrEqual(last);
      last = t;
    }
  });
});

describe('meanRelativeT', () => {
  it('returns 0 for zero energy', () => {
    expect(meanRelativeT(0, 1_000_000, 1000)).toBe(0);
  });

  it('returns 0 for negative energy', () => {
    expect(meanRelativeT(-5, 1_000_000, 1000)).toBe(0);
  });

  it('returns 0 when cellCount is 0', () => {
    expect(meanRelativeT(100, 1_000_000, 0)).toBe(0);
  });

  it('returns 0 when totalEnergy is 0 (mean would be 0)', () => {
    expect(meanRelativeT(100, 0, 1000)).toBe(0);
  });

  it('maps the mean cell to MEAN_T', () => {
    // mean = 1_000_000 / 1000 = 1000.
    expect(meanRelativeT(1000, 1_000_000, 1000)).toBeCloseTo(MEAN_T, 10);
  });

  it('maps T_SPREAD × mean to 1 (white-hot)', () => {
    // mean = 1000, hot = 100_000.
    expect(meanRelativeT(1000 * T_SPREAD, 1_000_000, 1000)).toBeCloseTo(1, 10);
  });

  it('maps mean / T_SPREAD to 0 (black)', () => {
    // mean = 1000, cold = 10.
    expect(meanRelativeT(1000 / T_SPREAD, 1_000_000, 1000)).toBeCloseTo(0, 10);
  });

  it('clamps to 1 above T_SPREAD × mean', () => {
    expect(meanRelativeT(1000 * T_SPREAD * 10, 1_000_000, 1000)).toBe(1);
  });

  it('clamps to 0 below mean / T_SPREAD', () => {
    expect(meanRelativeT(1000 / T_SPREAD / 10, 1_000_000, 1000)).toBe(0);
  });

  it('places 10× mean halfway through the warm half', () => {
    // log(10) / log(100) = 0.5; t = MEAN_T + (1 - MEAN_T) * 0.5.
    const expected = MEAN_T + (1 - MEAN_T) * 0.5;
    expect(meanRelativeT(10_000, 1_000_000, 1000)).toBeCloseTo(expected, 10);
  });

  it('places 0.1× mean halfway through the cool half', () => {
    // log(0.1) / log(100) = -0.5; t = MEAN_T * (-0.5 + 1) = MEAN_T * 0.5.
    expect(meanRelativeT(100, 1_000_000, 1000)).toBeCloseTo(MEAN_T * 0.5, 10);
  });

  it('adapts to cellCount: same energy reads as warm in a sparse field', () => {
    // 100 j. against a 10-cell, 10k-total field — mean=1000, ratio=0.1
    // → t in cool half (around 0.5 × MEAN_T).
    const sparse = meanRelativeT(100, 10_000, 10);
    // Same 100 j. against a 1000-cell, 10k-total field — mean=10, ratio=10
    // → t in warm half.
    const dense = meanRelativeT(100, 10_000, 1000);
    expect(dense).toBeGreaterThan(sparse);
    expect(dense).toBeGreaterThan(MEAN_T);
    expect(sparse).toBeLessThan(MEAN_T);
  });

  it('is monotonically non-decreasing in energy for fixed field', () => {
    const totalE = 10_000_000;
    const cells = 1000;
    let last = 0;
    for (const e of [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000]) {
      const t = meanRelativeT(e, totalE, cells);
      expect(t).toBeGreaterThanOrEqual(last);
      last = t;
    }
  });
});

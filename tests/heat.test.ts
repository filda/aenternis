import { describe, it, expect } from 'vitest';
import { HEAT_STOPS, heatColor } from '../src/heat.ts';

describe('HEAT_STOPS', () => {
  it('starts at t=0 and ends at t=1', () => {
    expect(HEAT_STOPS[0]?.t).toBe(0);
    expect(HEAT_STOPS[HEAT_STOPS.length - 1]?.t).toBe(1);
  });

  it('is sorted by t', () => {
    for (let i = 1; i < HEAT_STOPS.length; i += 1) {
      expect(HEAT_STOPS[i]!.t).toBeGreaterThan(HEAT_STOPS[i - 1]!.t);
    }
  });

  it('has 6 stops', () => {
    expect(HEAT_STOPS).toHaveLength(6);
  });
});

describe('heatColor', () => {
  it('returns the first stop at t=0', () => {
    const [r, g, b] = heatColor(0);
    expect(r).toBe(0);
    expect(g).toBe(0);
    expect(b).toBe(0);
  });

  it('returns the last stop at t=1', () => {
    const [r, g, b] = heatColor(1);
    expect(r).toBe(1);
    expect(g).toBe(1);
    expect(b).toBeCloseTo(0.86);
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
    expect(r[2]).toBeCloseTo(0.86, 10);
  });

  it('returns an exact stop value when t lands on a stop', () => {
    const [r, g, b] = heatColor(0.30);
    expect(r).toBeCloseTo(0.32, 5);
    expect(g).toBeCloseTo(0.08, 5);
    expect(b).toBeCloseTo(0.24, 5);
  });

  it('linearly interpolates between adjacent stops', () => {
    // Halfway between t=0 (black) and t=0.10 (0.16, 0.24, 0.40).
    const [r, g, b] = heatColor(0.05);
    expect(r).toBeCloseTo(0.08, 5);
    expect(g).toBeCloseTo(0.12, 5);
    expect(b).toBeCloseTo(0.20, 5);
  });

  it('returns three components', () => {
    expect(heatColor(0.5)).toHaveLength(3);
  });

  it('produces monotonically non-decreasing red across stops', () => {
    // Red is monotonic in this ramp: 0, .16, .32, .78, .94, 1.
    const reds = HEAT_STOPS.map((s) => s.rgb[0]);
    for (let i = 1; i < reds.length; i += 1) {
      expect(reds[i]!).toBeGreaterThanOrEqual(reds[i - 1]!);
    }
  });
});

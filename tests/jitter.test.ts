import { describe, it, expect } from 'vitest';
import {
  JITTER_AMPLITUDE,
  type JitterAxis,
  gridJitter,
} from '../src/jitter.ts';

describe('JITTER_AMPLITUDE', () => {
  it('is small enough to keep cells visually on-grid', () => {
    // Half a unit would let neighboring cells overlap centers; we want
    // a clear visual signal that they're still aligned to integer
    // coordinates.
    expect(JITTER_AMPLITUDE).toBeLessThan(0.5);
    expect(JITTER_AMPLITUDE).toBeGreaterThan(0);
  });
});

describe('gridJitter', () => {
  it('returns values in [-0.5, 0.5) for a sample of integer cells', () => {
    for (let x = -5; x <= 5; x += 1) {
      for (let y = -5; y <= 5; y += 1) {
        for (let z = -5; z <= 5; z += 1) {
          for (const axis of [0, 1, 2] as const satisfies readonly JitterAxis[]) {
            const j = gridJitter(x, y, z, axis);
            expect(j).toBeGreaterThanOrEqual(-0.5);
            expect(j).toBeLessThan(0.5);
          }
        }
      }
    }
  });

  it('is deterministic — same input gives same output', () => {
    const a = gridJitter(7, -3, 11, 0);
    const b = gridJitter(7, -3, 11, 0);
    expect(a).toBe(b);
  });

  it('produces different jitter for different cells on the same axis', () => {
    const a = gridJitter(0, 0, 0, 0);
    const b = gridJitter(1, 0, 0, 0);
    expect(a).not.toBe(b);
  });

  it('produces different jitter on different axes for the same cell', () => {
    const x = gridJitter(3, 5, 7, 0);
    const y = gridJitter(3, 5, 7, 1);
    const z = gridJitter(3, 5, 7, 2);
    // Probability of accidental equality across distinct prime triples
    // is effectively zero; if this trips it means the prime tables
    // collapsed.
    expect(x).not.toBe(y);
    expect(y).not.toBe(z);
    expect(x).not.toBe(z);
  });

  it('returns a number for negative coordinates', () => {
    // Sin handles negative input fine; this is a sanity guard against
    // a future "abs the input first" refactor that would alias mirrored
    // cells onto the same jitter.
    const a = gridJitter(-3, 4, -2, 1);
    const b = gridJitter(3, 4, -2, 1);
    expect(Number.isFinite(a)).toBe(true);
    expect(a).not.toBe(b);
  });

  it('does not produce NaN for the origin', () => {
    // sin(0) = 0; our hash multiplier × 0 = 0; floor(0) = 0; result = -0.5.
    expect(gridJitter(0, 0, 0, 0)).toBe(-0.5);
  });

  it('matches a snapshot of known hash outputs', () => {
    // Pinning specific values so any drift in the prime triples,
    // hash multiplier, or operator order shows up loudly. If you
    // intentionally change the hash, regenerate these expectations
    // — the visual jitter pattern in the viewer changes too.
    expect(gridJitter(1, 0, 0, 0)).toBeCloseTo(0.421690389815922, 12);
    expect(gridJitter(0, 1, 0, 0)).toBeCloseTo(-0.317083647949403, 12);
    expect(gridJitter(0, 0, 1, 0)).toBeCloseTo(-0.280554611918774, 12);
    expect(gridJitter(1, 1, 1, 0)).toBeCloseTo(-0.062316292770447, 12);
    expect(gridJitter(1, 1, 1, 1)).toBeCloseTo(0.281688064347691, 12);
    expect(gridJitter(1, 1, 1, 2)).toBeCloseTo(-0.487670818241895, 12);
    expect(gridJitter(5, -3, 2, 0)).toBeCloseTo(0.240314908787241, 12);
    expect(gridJitter(5, -3, 2, 1)).toBeCloseTo(0.195463343054143, 12);
  });

  it('roughly distributes across the [-0.5, 0.5) range', () => {
    // Sample 1000 cells, check the mean is near 0 (not stuck on one
    // side of the range) and at least one value lands in each half.
    let sum = 0;
    let sawNegative = false;
    let sawPositive = false;
    let count = 0;
    for (let x = 0; x < 10; x += 1) {
      for (let y = 0; y < 10; y += 1) {
        for (let z = 0; z < 10; z += 1) {
          const j = gridJitter(x, y, z, 0);
          sum += j;
          if (j < 0) sawNegative = true;
          if (j > 0) sawPositive = true;
          count += 1;
        }
      }
    }
    expect(sawNegative).toBe(true);
    expect(sawPositive).toBe(true);
    // Mean should be close to 0 — within 0.05 is plenty for 1000 samples.
    expect(Math.abs(sum / count)).toBeLessThan(0.05);
  });
});

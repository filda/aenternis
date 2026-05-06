import { describe, it, expect } from 'vitest';
import {
  totalEnergy,
  totalSlots,
  isEnergyConserved,
  energyDelta,
} from '../src/conservation.ts';

describe('totalEnergy', () => {
  it('sums energy across cells', () => {
    expect(totalEnergy([{ energy: 1 }, { energy: 2 }, { energy: 3 }])).toBe(6);
  });

  it('returns 0 for an empty iterable', () => {
    expect(totalEnergy([])).toBe(0);
  });

  it('treats missing energy as 0', () => {
    expect(totalEnergy([{}, { energy: 5 }, { energy: 0 }])).toBe(5);
  });

  it('ignores non-numeric energy values', () => {
    expect(totalEnergy([{ energy: '7' }, { energy: 4 }, { energy: null }])).toBe(4);
  });

  it('skips null / undefined cells', () => {
    expect(totalEnergy([null, undefined, { energy: 9 }])).toBe(9);
  });

  it('works with negative energy values', () => {
    expect(totalEnergy([{ energy: -3 }, { energy: 5 }])).toBe(2);
  });
});

describe('totalSlots', () => {
  it('sums memory lengths', () => {
    expect(totalSlots([{ memory: [1, 2, 3] }, { memory: [4] }])).toBe(4);
  });

  it('returns 0 for an empty iterable', () => {
    expect(totalSlots([])).toBe(0);
  });

  it('treats missing memory as 0 slots', () => {
    expect(totalSlots([{}, { memory: [1] }])).toBe(1);
  });

  it('works with Uint32Array-typed memory', () => {
    expect(totalSlots([{ memory: new Uint32Array(7) }])).toBe(7);
  });

  it('skips null cells and cells with non-array memory', () => {
    expect(totalSlots([null, { memory: 42 }, { memory: [1, 2] }])).toBe(2);
  });

  it('skips cells whose memory is null', () => {
    expect(totalSlots([{ memory: null }, { memory: [1, 2] }])).toBe(2);
  });

  it('ignores objects whose length is not a number', () => {
    expect(totalSlots([{ memory: { length: 'foo' } }, { memory: [1, 2] }])).toBe(2);
  });
});

describe('isEnergyConserved', () => {
  it('returns true when totals match', () => {
    expect(isEnergyConserved([{ energy: 5 }], [{ energy: 2 }, { energy: 3 }])).toBe(true);
  });

  it('returns false when totals differ', () => {
    expect(isEnergyConserved([{ energy: 5 }], [{ energy: 4 }])).toBe(false);
  });

  it('returns true when both sides are empty', () => {
    expect(isEnergyConserved([], [])).toBe(true);
  });

  it('returns false when only one side is empty', () => {
    expect(isEnergyConserved([{ energy: 1 }], [])).toBe(false);
  });
});

describe('energyDelta', () => {
  it('reports zero delta when conserved', () => {
    const r = energyDelta([{ energy: 4 }], [{ energy: 1 }, { energy: 3 }]);
    expect(r).toEqual({ before: 4, after: 4, delta: 0, conserved: true });
  });

  it('reports positive delta when energy was added', () => {
    const r = energyDelta([{ energy: 2 }], [{ energy: 5 }]);
    expect(r).toEqual({ before: 2, after: 5, delta: 3, conserved: false });
  });

  it('reports negative delta when energy was lost', () => {
    const r = energyDelta([{ energy: 10 }], [{ energy: 7 }]);
    expect(r).toEqual({ before: 10, after: 7, delta: -3, conserved: false });
  });

  it('reports zeros for two empty inputs', () => {
    expect(energyDelta([], [])).toEqual({ before: 0, after: 0, delta: 0, conserved: true });
  });
});

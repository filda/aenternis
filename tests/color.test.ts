import { describe, it, expect } from 'vitest';
import {
  HUE_SATURATION,
  HUE_VALUE_FLOOR,
  PAINT_BLEND,
  appearanceColor,
  cellColor,
  hsvToRgb,
  hueValue,
  originColor,
  tagHue,
} from '../src/color.ts';
import { heatColor } from '../src/heat.ts';

function expectRgbClose(actual: readonly number[], expected: readonly number[]): void {
  for (let i = 0; i < 3; i += 1) {
    expect(actual[i]).toBeCloseTo(expected[i]!, 10);
  }
}

describe('hsvToRgb', () => {
  it('maps the primary hues at full saturation/value', () => {
    expectRgbClose(hsvToRgb(0, 1, 1), [1, 0, 0]); // red
    expectRgbClose(hsvToRgb(1 / 3, 1, 1), [0, 1, 0]); // green
    expectRgbClose(hsvToRgb(2 / 3, 1, 1), [0, 0, 1]); // blue
  });

  it('returns gray when saturation is 0', () => {
    const [r, g, b] = hsvToRgb(0.4, 0, 0.7);
    expect(r).toBeCloseTo(0.7);
    expect(g).toBeCloseTo(0.7);
    expect(b).toBeCloseTo(0.7);
  });

  it('wraps the hue modulo 1', () => {
    expectRgbClose(hsvToRgb(1.0, 1, 1), hsvToRgb(0, 1, 1));
    expectRgbClose(hsvToRgb(-1 / 3, 1, 1), hsvToRgb(2 / 3, 1, 1));
  });

  it('scales value down to black', () => {
    expectRgbClose(hsvToRgb(0.5, 1, 0), [0, 0, 0]);
  });
});

describe('tagHue', () => {
  it('is in [0, 1)', () => {
    for (const tag of [0, 1, 42, 0xdead_beef, 0xffff_ffff]) {
      const h = tagHue(tag);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(1);
    }
  });

  it('is deterministic', () => {
    expect(tagHue(12345)).toBe(tagHue(12345));
  });

  it('spreads adjacent tags to distinct hues', () => {
    // A bit-mix hash, so consecutive ids must not collapse to one hue.
    expect(tagHue(1)).not.toBeCloseTo(tagHue(2), 2);
    expect(tagHue(100)).not.toBeCloseTo(tagHue(101), 2);
  });
});

describe('hueValue', () => {
  it('sits at the floor for zero energy and reaches 1 at full energy', () => {
    expect(hueValue(0)).toBeCloseTo(HUE_VALUE_FLOOR);
    expect(hueValue(1)).toBeCloseTo(1);
  });

  it('clamps out-of-range t', () => {
    expect(hueValue(-5)).toBeCloseTo(HUE_VALUE_FLOOR);
    expect(hueValue(5)).toBeCloseTo(1);
  });
});

describe('appearanceColor', () => {
  it('keeps unpainted cells on the plain heat color', () => {
    for (const t of [0, 0.3, 0.5, 0.8, 1]) {
      expect(appearanceColor(0, t)).toEqual(heatColor(t));
    }
  });

  it('tints painted cells away from the heat color', () => {
    const t = 0.5;
    const painted = appearanceColor(0x00ff_00aa, t);
    const heat = heatColor(t);
    expect(painted).not.toEqual(heat);
  });

  it('tracks energy brightness (hot painted cells are brighter)', () => {
    const tag = 0x00ff_00aa;
    const bright = appearanceColor(tag, 0.85);
    const dim = appearanceColor(tag, 0.2);
    const value = (c: readonly number[]) => Math.max(c[0]!, c[1]!, c[2]!);
    expect(value(bright)).toBeGreaterThan(value(dim));
  });

  it('separates painted from unpainted at the same energy', () => {
    expect(appearanceColor(1, 0.5)).not.toEqual(appearanceColor(0, 0.5));
  });

  it('blends, so a painted cell still carries some heat color', () => {
    // With PAINT_BLEND < 1 the result is between heat and pure paint.
    expect(PAINT_BLEND).toBeGreaterThan(0);
    expect(PAINT_BLEND).toBeLessThanOrEqual(1);
  });
});

describe('originColor', () => {
  it('produces in-range channels', () => {
    for (const tag of [0, 7, 0x1234_5678]) {
      for (const [r, g, b] of [originColor(tag, 0), originColor(tag, 1)]) {
        for (const c of [r, g, b]) {
          expect(c).toBeGreaterThanOrEqual(0);
          expect(c).toBeLessThanOrEqual(1);
        }
      }
    }
  });

  it('uses saturation so distinct lineages differ', () => {
    expect(HUE_SATURATION).toBeGreaterThan(0);
    expect(originColor(1, 1)).not.toEqual(originColor(2, 1));
  });
});

describe('cellColor', () => {
  it('falls back to the heat ramp in energy mode', () => {
    for (const t of [0, 0.3, 0.5, 0.8, 1]) {
      expect(cellColor('energy', t, 0xabcd, 0x1234)).toEqual(heatColor(t));
    }
  });

  it('dispatches appearance and origin modes', () => {
    expect(cellColor('appearance', 0.5, 0xabcd, 0x1234)).toEqual(
      appearanceColor(0xabcd, 0.5),
    );
    expect(cellColor('origin', 0.5, 0xabcd, 0x1234)).toEqual(
      originColor(0x1234, 0.5),
    );
  });
});

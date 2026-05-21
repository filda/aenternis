import { describe, it, expect } from 'vitest';
import { PRESETS, findPreset } from '../src/presets.ts';
import { assemble } from '../src/asm.ts';

describe('PRESETS', () => {
  it('exposes at least the canonical prototype-9 set', () => {
    const names = PRESETS.map((p) => p.name);
    expect(names).toEqual(
      expect.arrayContaining(['counter', 'self_xp', 'self_omni', 'beacon', 'quine_core', 'projectile']),
    );
  });

  it('names are unique', () => {
    const names = PRESETS.map((p) => p.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it('every preset assembles cleanly', () => {
    for (const preset of PRESETS) {
      const { slots, errors } = assemble(preset.source);
      expect(errors, `preset ${preset.name} has parse errors`).toEqual([]);
      expect(slots.length, `preset ${preset.name} produced no slots`).toBeGreaterThan(0);
    }
  });

  it('every preset has a non-empty hint', () => {
    for (const preset of PRESETS) {
      expect(preset.hint.length).toBeGreaterThan(0);
    }
  });
});

describe('findPreset', () => {
  it('returns the preset by name', () => {
    const counter = findPreset('counter');
    expect(counter).not.toBeNull();
    expect(counter?.name).toBe('counter');
    expect(counter?.source).toContain('inc 0x10');
  });

  it('returns null for unknown names', () => {
    expect(findPreset('unknown_program')).toBeNull();
    expect(findPreset('')).toBeNull();
  });
});

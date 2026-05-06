import { describe, it, expect } from 'vitest';
import { parseProgramText } from '../src/program-text.ts';

describe('parseProgramText', () => {
  it('reports "empty" for a blank source', () => {
    const r = parseProgramText('');
    expect(r.status).toBe('empty');
    expect(r.program).toHaveLength(0);
  });

  it('reports "empty" for a comments-only source', () => {
    const r = parseProgramText('; just a comment\n  ; another\n');
    expect(r.status).toBe('empty');
    expect(r.program).toHaveLength(0);
  });

  it('reports the slot count on a clean parse', () => {
    const r = parseProgramText('start:\n  setp xp, start\n  jmp start');
    // setp = 3 slots, jmp = 2 slots → 5 total
    expect(r.status).toBe('5 slot(s) assembled');
    expect(Array.from(r.program)).toEqual([0x09, 0, 0, 0x07, 0]);
  });

  it('reports parse errors with semicolon-joined messages', () => {
    const r = parseProgramText('bogus 1, 2');
    expect(r.status).toMatch(/^1 parse error\(s\): /);
    expect(r.status).toMatch(/unknown mnemonic/);
  });

  it('preserves partial output on errors (best-effort)', () => {
    const r = parseProgramText('nop\nbogus\nnop');
    expect(Array.from(r.program)).toEqual([0x00, 0x00]);
    expect(r.status).toMatch(/1 parse error/);
  });

  it('reports multiple errors joined by "; "', () => {
    const r = parseProgramText('bogus\nset 1');
    expect(r.status).toMatch(/^2 parse error/);
    expect(r.status).toContain('; ');
  });

  it('returns a Uint32Array program', () => {
    const r = parseProgramText('nop');
    expect(r.program).toBeInstanceOf(Uint32Array);
  });
});

import { describe, it, expect } from 'vitest';
import {
  isMainToWorkerMsg,
  normalizeProgram,
} from '../src/protocol.ts';

describe('normalizeProgram', () => {
  it('returns an empty Uint32Array when program is undefined', () => {
    const out = normalizeProgram(undefined);
    expect(out).toBeInstanceOf(Uint32Array);
    expect(out).toHaveLength(0);
  });

  it('returns an empty Uint32Array when program is an empty array', () => {
    const out = normalizeProgram([]);
    expect(out).toBeInstanceOf(Uint32Array);
    expect(out).toHaveLength(0);
  });

  it('returns the same Uint32Array reference unchanged', () => {
    const input = new Uint32Array([1, 2, 3]);
    const out = normalizeProgram(input);
    expect(out).toBe(input);
  });

  it('copies a number array into a Uint32Array', () => {
    const out = normalizeProgram([0x01, 0x02, 0x03]);
    expect(out).toBeInstanceOf(Uint32Array);
    expect(Array.from(out)).toEqual([1, 2, 3]);
  });

  it('returns an empty Uint32Array for an empty Uint32Array', () => {
    const input = new Uint32Array(0);
    const out = normalizeProgram(input);
    expect(out).toBeInstanceOf(Uint32Array);
    expect(out).toHaveLength(0);
  });
});

describe('isMainToWorkerMsg', () => {
  it('accepts an init message', () => {
    expect(isMainToWorkerMsg({ type: 'init' })).toBe(true);
  });

  it('accepts a config message', () => {
    expect(isMainToWorkerMsg({ type: 'config' })).toBe(true);
  });

  it('accepts a running message', () => {
    expect(isMainToWorkerMsg({ type: 'running' })).toBe(true);
  });

  it('accepts an inspect message', () => {
    expect(isMainToWorkerMsg({ type: 'inspect' })).toBe(true);
  });

  it('accepts a step message', () => {
    expect(isMainToWorkerMsg({ type: 'step' })).toBe(true);
  });

  it('rejects unknown type', () => {
    expect(isMainToWorkerMsg({ type: 'snapshot' })).toBe(false);
    expect(isMainToWorkerMsg({ type: 'foo' })).toBe(false);
  });

  it('rejects null', () => {
    expect(isMainToWorkerMsg(null)).toBe(false);
  });

  it('rejects undefined', () => {
    expect(isMainToWorkerMsg(undefined)).toBe(false);
  });

  it('rejects primitive values', () => {
    expect(isMainToWorkerMsg('init')).toBe(false);
    expect(isMainToWorkerMsg(42)).toBe(false);
    expect(isMainToWorkerMsg(true)).toBe(false);
  });

  it('rejects objects without a type field', () => {
    expect(isMainToWorkerMsg({})).toBe(false);
    expect(isMainToWorkerMsg({ seed: 1234 })).toBe(false);
  });

  it('rejects objects whose type is not a recognized string', () => {
    expect(isMainToWorkerMsg({ type: 42 })).toBe(false);
    expect(isMainToWorkerMsg({ type: null })).toBe(false);
  });
});

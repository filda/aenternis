import { describe, it, expect } from 'vitest';
import type { ConfigMsg, InitMsg } from '../src/protocol.ts';
import {
  applyConfig,
  DEFAULT_STATE,
  stateFromInit,
  type WorkerSimState,
} from '../src/worker-state.ts';

const baseInit: InitMsg = {
  type: 'init',
  seed: 1234,
  energy: 10_000_000,
  coeff: 0.15,
  k: 1,
};

describe('DEFAULT_STATE', () => {
  it('matches the original worker fall-back values', () => {
    expect(DEFAULT_STATE).toEqual({
      coeff: 0.20,
      k: 1,
      moveThreshold: 2.0,
    });
  });

  it('is frozen (cannot be mutated)', () => {
    expect(Object.isFrozen(DEFAULT_STATE)).toBe(true);
  });
});

describe('stateFromInit', () => {
  it('copies coeff and k from the message', () => {
    const s = stateFromInit({ ...baseInit, coeff: 0.42, k: 7 });
    expect(s.coeff).toBe(0.42);
    expect(s.k).toBe(7);
  });

  it('uses the default moveThreshold when not provided', () => {
    const s = stateFromInit(baseInit);
    expect(s.moveThreshold).toBe(DEFAULT_STATE.moveThreshold);
  });

  it('uses the provided moveThreshold when given', () => {
    const s = stateFromInit({ ...baseInit, moveThreshold: 3.5 });
    expect(s.moveThreshold).toBe(3.5);
  });
});

describe('applyConfig', () => {
  const before: WorkerSimState = {
    coeff: 0.10,
    k: 2,
    moveThreshold: 1.5,
  };

  const baseCfg: ConfigMsg = {
    type: 'config',
    coeff: 0.30,
    k: 5,
  };

  it('overwrites coeff and k unconditionally', () => {
    const s = applyConfig(before, baseCfg);
    expect(s.coeff).toBe(0.30);
    expect(s.k).toBe(5);
  });

  it('keeps moveThreshold when not provided in the message', () => {
    const s = applyConfig(before, baseCfg);
    expect(s.moveThreshold).toBe(1.5);
  });

  it('updates moveThreshold when provided', () => {
    const s = applyConfig(before, { ...baseCfg, moveThreshold: 2.7 });
    expect(s.moveThreshold).toBe(2.7);
  });

  it('does not mutate the input state', () => {
    const snapshot = { ...before };
    applyConfig(before, { ...baseCfg, moveThreshold: 99 });
    expect(before).toEqual(snapshot);
  });

  it('updates moveThreshold to 0 when explicitly set to 0', () => {
    // Regression: a `typeof === 'number'` guard accepts 0; a truthy
    // guard would mistakenly fall through to the default.
    const s = applyConfig(before, { ...baseCfg, moveThreshold: 0 });
    expect(s.moveThreshold).toBe(0);
  });
});

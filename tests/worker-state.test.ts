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
      gravity: 0.0,
      gravityAlpha: 0.0,
      gravityRadius: 1,
      pressure: 0.0,
      pressureGamma: 2.0,
      pressureEref: 1.0,
      mutationStrength: 0.0,
      mutationHalfDensity: 40_000,
      metricsEvery: 0,
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

  it('uses default gravity/pressure fields when not provided', () => {
    const s = stateFromInit(baseInit);
    expect(s.gravity).toBe(DEFAULT_STATE.gravity);
    expect(s.gravityAlpha).toBe(DEFAULT_STATE.gravityAlpha);
    expect(s.gravityRadius).toBe(DEFAULT_STATE.gravityRadius);
    expect(s.pressure).toBe(DEFAULT_STATE.pressure);
    expect(s.pressureGamma).toBe(DEFAULT_STATE.pressureGamma);
    expect(s.pressureEref).toBe(DEFAULT_STATE.pressureEref);
    expect(s.mutationStrength).toBe(DEFAULT_STATE.mutationStrength);
    expect(s.mutationHalfDensity).toBe(DEFAULT_STATE.mutationHalfDensity);
    expect(s.metricsEvery).toBe(DEFAULT_STATE.metricsEvery);
  });

  it('uses the provided gravity/pressure fields when given', () => {
    const s = stateFromInit({
      ...baseInit,
      gravity: 0.2,
      gravityAlpha: 0.05,
      gravityRadius: 4,
      pressure: 0.03,
      pressureGamma: 2.5,
      pressureEref: 8,
      mutationStrength: 0.001,
      mutationHalfDensity: 25_000,
      metricsEvery: 50,
    });
    expect(s.gravity).toBe(0.2);
    expect(s.gravityAlpha).toBe(0.05);
    expect(s.gravityRadius).toBe(4);
    expect(s.pressure).toBe(0.03);
    expect(s.pressureGamma).toBe(2.5);
    expect(s.pressureEref).toBe(8);
    expect(s.mutationStrength).toBe(0.001);
    expect(s.mutationHalfDensity).toBe(25_000);
    expect(s.metricsEvery).toBe(50);
  });
});

describe('applyConfig', () => {
  const before: WorkerSimState = {
    coeff: 0.10,
    k: 2,
    moveThreshold: 1.5,
    gravity: 0.1,
    gravityAlpha: 0.04,
    gravityRadius: 2,
    pressure: 0.02,
    pressureGamma: 2.0,
    pressureEref: 4.0,
    mutationStrength: 0.0005,
    mutationHalfDensity: 30_000,
    metricsEvery: 0,
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

  it('keeps gravity/pressure fields when not provided', () => {
    const s = applyConfig(before, baseCfg);
    expect(s.gravity).toBe(before.gravity);
    expect(s.gravityAlpha).toBe(before.gravityAlpha);
    expect(s.gravityRadius).toBe(before.gravityRadius);
    expect(s.pressure).toBe(before.pressure);
    expect(s.pressureGamma).toBe(before.pressureGamma);
    expect(s.pressureEref).toBe(before.pressureEref);
    expect(s.mutationStrength).toBe(before.mutationStrength);
    expect(s.mutationHalfDensity).toBe(before.mutationHalfDensity);
    expect(s.metricsEvery).toBe(before.metricsEvery);
  });

  it('updates gravity/pressure fields when provided', () => {
    const s = applyConfig(before, {
      ...baseCfg,
      gravity: 0.3,
      gravityAlpha: 0.06,
      gravityRadius: 5,
      pressure: 0.05,
      pressureGamma: 3.0,
      pressureEref: 16,
      mutationStrength: 0.002,
      mutationHalfDensity: 12_345,
      metricsEvery: 25,
    });
    expect(s.gravity).toBe(0.3);
    expect(s.gravityAlpha).toBe(0.06);
    expect(s.gravityRadius).toBe(5);
    expect(s.pressure).toBe(0.05);
    expect(s.pressureGamma).toBe(3.0);
    expect(s.pressureEref).toBe(16);
    expect(s.mutationStrength).toBe(0.002);
    expect(s.mutationHalfDensity).toBe(12_345);
    expect(s.metricsEvery).toBe(25);
  });

  it('updates gravity to 0 when explicitly set to 0 (turning gravity off)', () => {
    // Same `typeof === 'number'` regression guard as moveThreshold: a
    // truthy check would refuse to turn gravity back off.
    const s = applyConfig(before, { ...baseCfg, gravity: 0 });
    expect(s.gravity).toBe(0);
  });
});

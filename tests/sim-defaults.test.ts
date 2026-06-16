import { describe, it, expect } from 'vitest';

import {
  DEFAULT_SIM_CONFIG,
  applySimConfig,
  type SimConfig,
  type SimSettableWorld,
} from '../src/sim-defaults.ts';

/** Records every setter call so we can assert the exact sequence + values. */
function recordingWorld(): { calls: Array<[string, number]> } & SimSettableWorld {
  const calls: Array<[string, number]> = [];
  const rec = (name: string) => (v: number) => {
    calls.push([name, v]);
  };
  return {
    calls,
    setMoveThreshold: rec('moveThreshold'),
    setGravity: rec('gravity'),
    setGravityAlpha: rec('gravityAlpha'),
    setGravityRadius: rec('gravityRadius'),
    setPressure: rec('pressure'),
    setPressureGamma: rec('pressureGamma'),
    setPressureEref: rec('pressureEref'),
    setMutationStrength: rec('mutationStrength'),
    setMutationHalfDensity: rec('mutationHalfDensity'),
  };
}

describe('applySimConfig', () => {
  it('pushes every physics knob from the config onto the world', () => {
    const world = recordingWorld();
    applySimConfig(world, DEFAULT_SIM_CONFIG);
    expect(world.calls).toEqual([
      ['moveThreshold', DEFAULT_SIM_CONFIG.moveThreshold],
      ['gravity', DEFAULT_SIM_CONFIG.gravity],
      ['gravityAlpha', DEFAULT_SIM_CONFIG.gravityAlpha],
      ['gravityRadius', DEFAULT_SIM_CONFIG.gravityRadius],
      ['pressure', DEFAULT_SIM_CONFIG.pressure],
      ['pressureGamma', DEFAULT_SIM_CONFIG.pressureGamma],
      ['pressureEref', DEFAULT_SIM_CONFIG.pressureEref],
      ['mutationStrength', DEFAULT_SIM_CONFIG.mutationStrength],
      ['mutationHalfDensity', DEFAULT_SIM_CONFIG.mutationHalfDensity],
    ]);
  });

  it('forwards arbitrary config values verbatim', () => {
    const cfg: SimConfig = {
      ...DEFAULT_SIM_CONFIG,
      gravity: 2.5,
      pressure: 0.33,
      mutationStrength: 0.0,
    };
    const world = recordingWorld();
    applySimConfig(world, cfg);
    const byName = Object.fromEntries(world.calls);
    expect(byName['gravity']).toBe(2.5);
    expect(byName['pressure']).toBe(0.33);
    expect(byName['mutationStrength']).toBe(0.0);
  });
});

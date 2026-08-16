// Wire-protocol parity gate between the TS viewer and the native server.
//
// `tests/fixtures/wire-messages.json` is the shared canon: this test
// pins the TS side (encodeOutgoing / decodeIncoming) to it, and
// `crates/aenternis-server/src/protocol.rs` round-trips the same file
// through `ClientMessage` / `ServerControl`. Any protocol change that
// lands on only one side breaks one of the two suites in `./check` —
// the drift where the server silently dropped gravity/pressure/
// mutation/genesis fields (2026-07) can no longer happen quietly.
//
// The message literals below use `Required<...>` on purpose: adding an
// optional field to `src/protocol.ts` makes this file stop compiling
// until the new field is added here AND to the fixture — which then
// fails the Rust round-trip until the server learns it too.

import { describe, expect, it } from 'vitest';

import { decodeIncoming, encodeOutgoing } from '../src/native-client.ts';
import type {
  ConfigMsg,
  InitMsg,
  InspectMsg,
  RunProgramMsg,
  RunningMsg,
  StepMsg,
} from '../src/protocol.ts';
import fixture from './fixtures/wire-messages.json';

/** Full init — every optional field present, values matching the fixture. */
const FULL_INIT: Required<InitMsg> = {
  type: 'init',
  seed: 1234,
  energy: 1_000_000,
  coeff: 0.5,
  k: 2,
  moveThreshold: 1.5,
  gravity: 1.5,
  gravityAlpha: 0.25,
  gravityRadius: 4,
  pressure: 0.125,
  pressureGamma: 2.5,
  pressureEref: 50_000.5,
  mutationStrength: 0.75,
  mutationHalfDensity: 40_000.5,
  genesisWindow: 512,
  genesisFertility: 1.5,
  metricsEvery: 25,
  program: [1, 2, 3],
};

const FULL_CONFIG: Required<ConfigMsg> = {
  type: 'config',
  coeff: 0.25,
  k: 3,
  moveThreshold: 2.5,
  gravity: 0.5,
  gravityAlpha: 0.125,
  gravityRadius: 2,
  pressure: 0.375,
  pressureGamma: 1.5,
  pressureEref: 25_000.5,
  mutationStrength: 0.5,
  mutationHalfDensity: 30_000.5,
  metricsEvery: 10,
};

const RUNNING: Required<RunningMsg> = { type: 'running', running: true };
const STEP: Required<StepMsg> = { type: 'step' };
const INSPECT: Required<InspectMsg> = { type: 'inspect', x: -1, y: 2, z: 3 };

const RUN_PROGRAM: Required<RunProgramMsg> = {
  type: 'runProgram',
  code: [7, 8, 9],
  reserve: 16,
  tag: 4242,
  appearance: 99,
};

describe('client → server wire parity (shared fixture)', () => {
  // Same order as the fixture's clientToServer array.
  const messages = [FULL_INIT, FULL_CONFIG, RUNNING, STEP, INSPECT, RUN_PROGRAM];

  it('covers every MainToWorkerMsg variant', () => {
    expect(fixture.clientToServer).toHaveLength(messages.length);
  });

  messages.forEach((msg, i) => {
    it(`encodes a full '${msg.type}' exactly as the fixture`, () => {
      expect(JSON.parse(encodeOutgoing(msg))).toEqual(fixture.clientToServer[i]);
    });
  });

  it('encodes a Uint32Array program identically to a plain array', () => {
    const typed = { ...FULL_INIT, program: new Uint32Array([1, 2, 3]) };
    expect(JSON.parse(encodeOutgoing(typed))).toEqual(fixture.clientToServer[0]);
  });
});

describe('server → client wire parity (shared fixture)', () => {
  // What each fixture entry must decode to — the fixture is emitted
  // verbatim by the server (pinned by the Rust serialization test), so
  // decode-equality here closes the loop.
  const expected = [
    { type: 'ready' },
    { type: 'welcome', running: true },
    { type: 'programStarted', x: -4, y: 5, z: -6, tag: 4242 },
    { type: 'programRejected', reason: 'no host cell with energy >= 19' },
  ] as const;

  it('covers every JSON server control message', () => {
    expect(fixture.serverToClient).toHaveLength(expected.length);
  });

  expected.forEach((msg, i) => {
    it(`decodes the fixture '${msg.type}' frame`, () => {
      const wire = JSON.stringify(fixture.serverToClient[i]);
      expect(decodeIncoming(wire)).toEqual(msg);
    });
  });
});

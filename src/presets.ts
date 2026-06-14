// Program presets — assembly sources for the "Program v centrální buňce"
// textarea. Mirrors prototype 9 (`prototypes/09-sparse-world/main.js`).
// Each entry assembles cleanly via `src/asm.ts`; that contract is enforced
// by `tests/presets.test.ts`.
//
// The presets exist as a starter library: a counter to see the CPU phase
// tick at all, a self-xp replicator to see propagation along one axis,
// projectile for ignition / soft mixing, and so on. Picking one fills the
// textarea — the user is expected to edit / extend from there.

export interface Preset {
  /** Display name for the <select>. */
  readonly name: string;
  /** Short Czech description shown as a status / hint. */
  readonly hint: string;
  /** Assembler source — parsed by `src/asm.ts`. */
  readonly source: string;
}

export const PRESETS: readonly Preset[] = Object.freeze([
  {
    name: 'counter',
    hint: 'inkrementuje slot 0x10 každý tick',
    source: 'loop:\n  inc 0x10\n  jmp loop',
  },
  {
    name: 'self_xp',
    hint: 'replikuje sám sebe ve směru +x (xp pointer drží program v core)',
    source: 'start:\n  setp xp, start\n  jmp start',
  },
  {
    name: 'self_omni',
    hint: 'replikuje sám sebe do 4 stran (xp/xn/yp/yn)',
    source: [
      'start:',
      '  setp xp, start',
      '  setp xn, start',
      '  setp yp, start',
      '  setp yn, start',
      '  jmp start',
    ].join('\n'),
  },
  {
    name: 'beacon',
    hint: 'replikátor xp + counter ve slotu 0x20',
    source: 'start:\n  inc 0x20\n  setp xp, start\n  jmp start',
  },
  {
    name: 'quine_core',
    hint: 'replikátor xp + DEADBEEF marker ve slotech 0x10-0x13',
    source: [
      '; quine — písmena DEADBEEF',
      'start:',
      '  set 0x10, 0xDE',
      '  set 0x11, 0xAD',
      '  set 0x12, 0xBE',
      '  set 0x13, 0xEF',
      '  setp xp, start',
      '  jmp start',
    ].join('\n'),
  },
  {
    name: 'projectile',
    hint: 'silný port xp střelba (active outflow 0x20 každý tick)',
    source: 'start:\n  setp xp, start\n  port xp, 0x20\n  jmp start',
  },
  {
    name: 'pilgrim',
    hint: 'v1: 1D gradient — kormidluje k hustšímu x-sousedovi (senergy → dominance)',
    source: [
      '; Pilgrim v1 — 1D density-gradient follower (x-axis).',
      '; Senses both x-neighbors and steers the lineage toward the denser',
      '; one by emitting its own program that way (setp+port → dominance',
      '; copies tag+program forward). See docs/pilgrim.md.',
      ';',
      '; Scratch lives in the junk region past the program (L=31 slots):',
      ';   0x20 = E(+x), 0x21 = E(-x), 0x22 = diff. Written before read.',
      ';',
      '; KEY KNOB: `port` = emission size. Dominance depends on what the',
      '; cell KEEPS after emitting (attacker_post), not what it sends — emit',
      '; everything and you keep ~0 → dominance ~0 in populated space (though',
      '; into the void it is always 1.0). So emit a portion: ~program length,',
      '; keep the rest to out-mass the target. Tune `port` by observation.',
      'start:',
      '  senergy xp, 0x20',
      '  senergy xn, 0x21',
      '  copy 0x22, 0x20',
      '  sub 0x22, 0x21      ; diff = E(+x) - E(-x)  (wrapping; signed test below)',
      '  jn 0x22, go_xn      ; diff < 0  ->  -x is denser, steer there',
      '  setp xp, start      ; else +x is denser (or equal): emit toward +x',
      '  port xp, 4',
      '  jmp start',
      'go_xn:',
      '  setp xn, start',
      '  port xn, 4',
      '  jmp start',
    ].join('\n'),
  },
]);

/** Lookup by name. `null` when the name is unknown — caller decides
 *  on the fallback (typically: leave the textarea untouched). */
export function findPreset(name: string): Preset | null {
  return PRESETS.find((p) => p.name === name) ?? null;
}

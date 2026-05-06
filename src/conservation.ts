// Conservation audit helpers for Aenternis simulation state.
//
// A `cell` is anything with at least an `energy` field (number) and
// optionally a `memory` field (Uint32Array or Array). These helpers are
// used by debug tooling and (eventually) by integration tests that
// assert energy / slot count is preserved across simulation ticks.
//
// They are deliberately tolerant of partial cells (missing fields,
// wrong-typed fields, null entries) so they can be pointed at
// half-initialized state without throwing. The public types reflect
// that contract: `energy` and `memory` are typed as `unknown` and the
// implementation narrows at runtime.

/**
 * The minimum shape these helpers care about. Both fields are tolerated
 * as `unknown` because callers may pass partially-initialized state;
 * non-conforming values are silently ignored at runtime.
 */
export interface CellLike {
  readonly energy?: unknown;
  readonly memory?: unknown;
}

/** Tolerates `null` / `undefined` entries in the iterable. */
export type CellsInput = Iterable<CellLike | null | undefined>;

interface HasLength {
  readonly length: number;
}

function hasNumericLength(value: unknown): value is HasLength {
  return (
    typeof value === 'object'
    && value !== null
    && typeof (value as { readonly length?: unknown }).length === 'number'
  );
}

/**
 * Sum of `energy` across all cells in the iterable.
 * Cells without a numeric `energy` are treated as 0.
 */
export function totalEnergy(cells: CellsInput): number {
  let sum = 0;
  for (const c of cells) {
    if (c && typeof c.energy === 'number') sum += c.energy;
  }
  return sum;
}

/**
 * Sum of `memory.length` across all cells.
 * Cells without a length-bearing `memory` are treated as 0 slots.
 */
export function totalSlots(cells: CellsInput): number {
  let sum = 0;
  for (const c of cells) {
    if (c && hasNumericLength(c.memory)) {
      sum += c.memory.length;
    }
  }
  return sum;
}

/**
 * Returns true when `before` and `after` have identical total energy.
 * The iterables may differ in shape — only the sum is compared.
 */
export function isEnergyConserved(before: CellsInput, after: CellsInput): boolean {
  return totalEnergy(before) === totalEnergy(after);
}

export interface EnergyDelta {
  readonly before: number;
  readonly after: number;
  readonly delta: number;
  readonly conserved: boolean;
}

/**
 * Detailed energy delta report. Useful when isEnergyConserved returns
 * false and you want to know how far off and in which direction.
 */
export function energyDelta(before: CellsInput, after: CellsInput): EnergyDelta {
  const b = totalEnergy(before);
  const a = totalEnergy(after);
  return { before: b, after: a, delta: a - b, conserved: a === b };
}

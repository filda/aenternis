// Conservation audit helpers for Aenternis simulation state.
//
// A `cell` is anything with at least an `energy` field (number) and
// optionally a `memory` field (Uint32Array or Array). These helpers are
// used by debug tooling and (eventually) by integration tests that
// assert energy / slot count is preserved across simulation ticks.
//
// They are deliberately tolerant of partial cells (missing fields) so
// they can be pointed at half-initialized state without throwing.

/**
 * Sum of `energy` across all cells in the iterable.
 * Cells without `energy` are treated as 0.
 *
 * @param {Iterable<{energy?: number}>} cells
 * @returns {number}
 */
export function totalEnergy(cells) {
  let sum = 0;
  for (const c of cells) {
    if (c && typeof c.energy === 'number') sum += c.energy;
  }
  return sum;
}

/**
 * Sum of `memory.length` across all cells.
 * Cells without `memory` are treated as 0 slots.
 *
 * @param {Iterable<{memory?: {length: number}}>} cells
 * @returns {number}
 */
export function totalSlots(cells) {
  let sum = 0;
  for (const c of cells) {
    if (c && c.memory && typeof c.memory.length === 'number') {
      sum += c.memory.length;
    }
  }
  return sum;
}

/**
 * Returns true when `before` and `after` have identical total energy.
 * The arrays may differ in shape — only the sum is compared.
 *
 * @param {Iterable<{energy?: number}>} before
 * @param {Iterable<{energy?: number}>} after
 * @returns {boolean}
 */
export function isEnergyConserved(before, after) {
  return totalEnergy(before) === totalEnergy(after);
}

/**
 * Detailed energy delta report. Useful when isEnergyConserved returns
 * false and you want to know how far off and in which direction.
 *
 * @param {Iterable<{energy?: number}>} before
 * @param {Iterable<{energy?: number}>} after
 * @returns {{before: number, after: number, delta: number, conserved: boolean}}
 */
export function energyDelta(before, after) {
  const b = totalEnergy(before);
  const a = totalEnergy(after);
  return { before: b, after: a, delta: a - b, conserved: a === b };
}

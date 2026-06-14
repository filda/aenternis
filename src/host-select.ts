// Host selection for "Run Program" / Project Pilgrim possession.
//
// `possess()` is energy-neutral: it overwrites an EXISTING cell (see
// docs/pilgrim.md), so the host must already hold at least the program's
// length plus a reserve (scratch + compute + emission fuel). This module
// scans a cell snapshot and picks an eligible host. Pure & DOM-free →
// unit-testable in node.
//
// Snapshot layout (STRIDE 6, see `World::cellsSnapshot` in the Rust core):
// per cell `[x, y, z, energy, origin_tag, appearance]`, where x/y/z are
// i32 reinterpreted as u32 (decode the sign with `| 0`).

export interface HostCoord {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

export interface FindHostOptions {
  /** Program length in slots (L). */
  readonly codeLen: number;
  /** Extra slots the host must have beyond the program (scratch / fuel). */
  readonly reserve: number;
}

/** Offset of the `energy` field within a snapshot record. */
const ENERGY_OFFSET = 3;

/**
 * Pick a host cell for an injected program from a flat cell `snapshot`.
 *
 * Eligibility (hard constraint): a host's energy must be at least
 * `codeLen + reserve` — possession is energy-neutral and cannot grow the
 * cell.
 *
 * Among eligible cells the one **farthest from the energy-weighted center
 * of mass** wins. This puts the pilgrim on the cool periphery (low energy →
 * low density-coupled mutation) with the longest possible journey *inward*
 * toward the dense core — the pilgrimage arc from docs/pilgrim.md. (The
 * earlier "largest energy" criterion spawned it in the hottest, most
 * mutagenic, most-dominated core, where it died almost at once.) Ties
 * resolve to the lex-smallest coord — the snapshot is emitted in
 * `(x, y, z)` order and ties keep the first seen.
 *
 * Returns `null` if no cell is eligible (or the world is empty) — the
 * caller should refuse the run.
 */
export function findHost(
  snapshot: Uint32Array,
  stride: number,
  { codeLen, reserve }: FindHostOptions,
): HostCoord | null {
  const need = codeLen + reserve;

  // Pass 1: energy-weighted center of mass over every live cell. (The
  // `off + stride <= len` guard's `+`→`-` Stryker mutant is equivalent: the
  // extra past-the-end iteration reads `undefined`, contributing nothing.)
  let sumE = 0;
  let cx = 0;
  let cy = 0;
  let cz = 0;
  for (let off = 0; off + stride <= snapshot.length; off += stride) {
    const e = snapshot[off + ENERGY_OFFSET]!;
    sumE += e;
    cx += (snapshot[off]! | 0) * e;
    cy += (snapshot[off + 1]! | 0) * e;
    cz += (snapshot[off + 2]! | 0) * e;
  }
  // (Stryker: the `sumE === 0` → `false` mutant is equivalent — with the
  // guard removed an empty world divides by zero into a NaN COM, and pass 2
  // then finds no eligible cell, so the result is still `null`.)
  if (sumE === 0) return null;
  cx /= sumE;
  cy /= sumE;
  cz /= sumE;

  // Pass 2: eligible cell farthest from the center of mass.
  let best: HostCoord | null = null;
  // -1 (not 0) so a sole eligible host sitting exactly at the COM
  // (dist² == 0) still wins the `dist2 > bestDist` comparison.
  let bestDist = -1;
  for (let off = 0; off + stride <= snapshot.length; off += stride) {
    if (snapshot[off + ENERGY_OFFSET]! < need) continue;
    const x = snapshot[off]! | 0;
    const y = snapshot[off + 1]! | 0;
    const z = snapshot[off + 2]! | 0;
    const dx = x - cx;
    const dy = y - cy;
    const dz = z - cz;
    const dist2 = dx * dx + dy * dy + dz * dz;
    if (dist2 > bestDist) {
      bestDist = dist2;
      best = { x, y, z };
    }
  }
  return best;
}

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
 * cell. Among eligible cells the one with the **largest energy** wins;
 * because the snapshot is emitted in lexicographic `(x, y, z)` order and
 * ties keep the first seen, equal-energy ties resolve to the lex-smallest
 * coord (deterministic).
 *
 * The positional criterion is deliberately simple for now — "largest
 * energy" maximizes fuel / compute headroom. Refining *where* the pilgrim
 * is born (e.g. farthest from the densest well) is a deferred decision
 * (docs/pilgrim.md "Otevřené body"); it only changes this function.
 *
 * Returns `null` if no cell is eligible — the caller should refuse the run
 * (the world may not be warm enough, or the program is too big).
 */
export function findHost(
  snapshot: Uint32Array,
  stride: number,
  { codeLen, reserve }: FindHostOptions,
): HostCoord | null {
  const need = codeLen + reserve;
  let best: HostCoord | null = null;
  // -1 (not 0) so a sole eligible host with energy 0 or 1 still wins the
  // `energy > bestEnergy` comparison below.
  let bestEnergy = -1;
  // Accepted-as-equivalent mutant (Stryker): `off + stride` → `off -
  // stride` in this guard. The `+ stride` keeps the loop reading only
  // complete records; the mutant lets it run a couple of iterations past
  // the end, but an out-of-bounds `Uint32Array` read yields `undefined`,
  // which fails both `< need` and `> bestEnergy`, so every extra
  // iteration is an observable no-op and the result is unchanged.
  for (let off = 0; off + stride <= snapshot.length; off += stride) {
    const energy = snapshot[off + ENERGY_OFFSET]!;
    if (energy < need) continue;
    if (energy > bestEnergy) {
      bestEnergy = energy;
      best = {
        x: snapshot[off]! | 0,
        y: snapshot[off + 1]! | 0,
        z: snapshot[off + 2]! | 0,
      };
    }
  }
  return best;
}

// Pure analysis over a worker snapshot.
//
// A snapshot is a flat `Uint32Array` of `cellCount` records, each
// `stride` slots long. The first 4 slots of every record are
// `[x, y, z, energy]`; the remainder is opaque to this module.
//
// These helpers compute summary statistics (bounding box, max-energy
// cell index) without touching the DOM or THREE — the renderer
// consumes the result and translates it into draw calls.

export interface Cell3D {
  readonly x: number;
  readonly y: number;
  readonly z: number;
  readonly energy: number;
}

export interface SnapshotBbox {
  readonly minX: number;
  readonly maxX: number;
  readonly minY: number;
  readonly maxY: number;
  readonly minZ: number;
  readonly maxZ: number;
}

export interface SnapshotAnalysis {
  /** Tightest bbox over the *visible* cells (after slice filter); `null`
   *  if no cell is visible. */
  readonly bbox: SnapshotBbox | null;
  /** Index of the highest-energy *visible* cell, or `-1` if none. */
  readonly maxCellIdx: number;
  /** Energy of the highest-energy visible cell, or `0` if none. */
  readonly maxEnergy: number;
}

/** Offset of the `origin_tag` field within a snapshot record (see
 *  `World::cellsSnapshot` in the Rust core). */
const ORIGIN_TAG_OFFSET = 4;

/** Index of the highest-energy cell whose `origin_tag` equals `tag`, or
 *  `-1` if no live cell carries it.
 *
 *  Used to follow a lineage rather than the global maximum — Project
 *  Pilgrim's conversion-wave "torch" is the max-energy carrier of the
 *  pilgrim tag (docs/pilgrim.md). Requires `stride ≥ 5`, since
 *  `origin_tag` lives at offset +4. */
export function findMaxEnergyIdxByTag(
  snap: Uint32Array,
  stride: number,
  cellCount: number,
  tag: number,
): number {
  let bestIdx = -1;
  // -1 (not 0) so a sole carrier with energy 0 or 1 still wins.
  let bestEnergy = -1;
  // Accepted-as-equivalent mutant (Stryker): `i < cellCount` → `i <=
  // cellCount`. The extra iteration reads one record past the end, but an
  // out-of-bounds `Uint32Array` read is `undefined`, which fails the
  // `!== tag` filter below, so it is an observable no-op.
  for (let i = 0; i < cellCount; i += 1) {
    const off = i * stride;
    if (snap[off + ORIGIN_TAG_OFFSET] !== tag) continue;
    const e = snap[off + 3]!;
    if (e > bestEnergy) {
      bestEnergy = e;
      bestIdx = i;
    }
  }
  return bestIdx;
}

/** Read the `[x, y, z, energy]` record at index `i`. The caller is
 *  responsible for `0 ≤ i < cellCount` and `stride ≥ 4`. */
export function cellAt(snap: Uint32Array, stride: number, i: number): Cell3D {
  const off = i * stride;
  return {
    x: snap[off]! | 0,
    y: snap[off + 1]! | 0,
    z: snap[off + 2]! | 0,
    energy: snap[off + 3]!,
  };
}

/** Walk a snapshot, computing a bounding box and the max-energy cell.
 *  When `sliceZ0Only` is true, only cells at `z = 0` are considered;
 *  the rest are ignored exactly as if absent. Empty (no visible cells)
 *  yields `bbox: null`, `maxCellIdx: -1`. */
export function analyzeSnapshot(
  snap: Uint32Array,
  stride: number,
  cellCount: number,
  sliceZ0Only: boolean,
): SnapshotAnalysis {
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  let minZ = Infinity;
  let maxZ = -Infinity;
  let maxEnergy = 0;
  let maxCellIdx = -1;
  let sawAny = false;

  for (let i = 0; i < cellCount; i += 1) {
    const off = i * stride;
    const z = snap[off + 2]! | 0;
    if (sliceZ0Only && z !== 0) continue;
    const x = snap[off]! | 0;
    const y = snap[off + 1]! | 0;
    const e = snap[off + 3]!;

    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
    minZ = Math.min(minZ, z);
    maxZ = Math.max(maxZ, z);
    if (e > maxEnergy) {
      maxEnergy = e;
      maxCellIdx = i;
    }
    sawAny = true;
  }

  const bbox: SnapshotBbox | null = sawAny
    ? { minX, maxX, minY, maxY, minZ, maxZ }
    : null;
  return { bbox, maxCellIdx, maxEnergy };
}

// Pure camera-fit math. Given a bounding box, computes the camera
// target (center of the box) and a reasonable eye position offset
// from that center. The renderer (web/main.ts) reads these and pokes
// them into THREE.

import type { SnapshotBbox } from './snapshot.ts';

export interface FitResult {
  readonly target: readonly [x: number, y: number, z: number];
  readonly eye: readonly [x: number, y: number, z: number];
}

/** Minimum span used when the world is degenerate (single cell, slice
 *  with one row). Without this floor the camera would zoom straight
 *  into the geometry. */
export const MIN_SPAN = 4;

/** Distance multiplier — `dist = max-axis-span × MULT`. Picked to keep
 *  the world filling roughly the central 40 % of the viewport. */
export const DIST_MULT = 2.5;

/** Y-component of the eye offset relative to the X/Z components. The
 *  smaller value gives a slight elevation without going overhead. */
export const Y_FACTOR = 0.6;

/** Compute a camera fit for a bounding box. The returned `target` is
 *  the box center; `eye` is `target + dist × (1, Y_FACTOR, 1)`. */
export function fitCamera(bbox: SnapshotBbox): FitResult {
  const cx = (bbox.minX + bbox.maxX) / 2;
  const cy = (bbox.minY + bbox.maxY) / 2;
  const cz = (bbox.minZ + bbox.maxZ) / 2;
  const span = Math.max(
    bbox.maxX - bbox.minX,
    bbox.maxY - bbox.minY,
    bbox.maxZ - bbox.minZ,
    MIN_SPAN,
  );
  const dist = span * DIST_MULT;
  return {
    target: [cx, cy, cz],
    eye: [cx + dist, cy + dist * Y_FACTOR, cz + dist],
  };
}

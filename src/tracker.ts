// Tracker state reducer — keeps the most recent positions of the
// max-energy cell so the renderer can draw a glowing trail behind it.
//
// Pure: just data in, data out. The renderer (web/main.ts) is
// responsible for turning a `TrackerState` into a `THREE.Line` update.

import type { Cell3D } from './snapshot.ts';

export interface TrackerState {
  readonly trail: readonly Cell3D[];
  readonly current: Cell3D | null;
}

export const EMPTY_TRACKER_STATE: TrackerState = Object.freeze({
  trail: Object.freeze([]),
  current: null,
});

/** Returns a new tracker state after a fresh max-energy cell sample.
 *
 *  - When the new cell is at the same `(x, y, z)` as the previous tail
 *    of the trail, the existing entry is updated in place (energy
 *    refreshed) so the line geometry stays smooth across stationary
 *    ticks.
 *  - Otherwise, a new entry is appended; entries older than
 *    `trailLen + 1` are dropped so the trail caps at `trailLen + 1`.
 *  - `trailLen` of 0 keeps only the current sample (no historical
 *    trail). The `+ 1` is intentional: a length-N trail needs N+1
 *    points to draw N segments.
 */
export function pushTrackerSample(
  state: TrackerState,
  sample: Cell3D,
  trailLen: number,
): TrackerState {
  const cap = Math.max(0, trailLen) + 1;
  const last = state.trail.length > 0 ? state.trail[state.trail.length - 1]! : null;
  const sameCell = last !== null
    && last.x === sample.x
    && last.y === sample.y
    && last.z === sample.z;

  if (sameCell) {
    // Replace the energy on the tail entry but keep the trail history.
    const updated = state.trail.slice(0, -1);
    updated.push({ x: sample.x, y: sample.y, z: sample.z, energy: sample.energy });
    return { trail: updated, current: sample };
  }

  const next = state.trail.slice();
  next.push(sample);
  while (next.length > cap) next.shift();
  return { trail: next, current: sample };
}

/** Returns a fresh `TrackerState` with no trail and no current cell.
 *  Used on Reset so a new world doesn't inherit the previous run's
 *  trail. */
export function resetTrackerState(): TrackerState {
  return { trail: [], current: null };
}

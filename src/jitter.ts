// Deterministic per-cell render jitter — breaks the perfect-grid
// alignment that produces visible moire interference in dense voxel
// fields, while staying stable across frames so cells don't shimmer.
//
// Pure math, no side effects, no DOM / THREE. The output is a
// reproducible pseudo-random offset: the same `(x, y, z)` always
// produces the same jitter value.

/** Per-axis prime triples used to decorrelate X / Y / Z jitter. The
 *  values are arbitrary "fract-sin hash" constants borrowed from the
 *  GLSL idiom; what matters is that the three axes don't share their
 *  hash so all three offsets aren't moving together. */
const JITTER_PRIMES: ReadonlyArray<readonly [number, number, number]> = Object.freeze([
  [12.9898, 78.233, 37.719],
  [54.123, 89.456, 23.654],
  [31.789, 67.234, 91.567],
] as const);

const JITTER_HASH_MULTIPLIER = 43758.5453;

export type JitterAxis = 0 | 1 | 2;

/** Default amplitude applied to `gridJitter` output by the renderer.
 *  At 0.15 the offset is small enough that cells visually still read
 *  as on-grid but large enough to break the moire fringes. Exported
 *  so the renderer (and tests) can reference the same constant. */
export const JITTER_AMPLITUDE = 0.25;

/** Deterministic jitter offset for a single cell on a single axis.
 *  Output range is `[-0.5, 0.5)`. Multiply by an amplitude to get the
 *  actual world-space displacement; with `JITTER_AMPLITUDE = 0.15` the
 *  resulting per-axis offset is in `[-0.075, 0.075)`. */
export function gridJitter(
  x: number,
  y: number,
  z: number,
  axis: JitterAxis,
): number {
  const p = JITTER_PRIMES[axis]!;
  const dot = x * p[0] + y * p[1] + z * p[2];
  const s = Math.sin(dot) * JITTER_HASH_MULTIPLIER;
  return s - Math.floor(s) - 0.5;
}

// Heat-color ramp used by the WASM viewer to translate per-cell energy
// (normalized to [0, 1]) into RGB triples. Pure math, no DOM / THREE.
//
// "Energy is blue" in the cool low-energy region — typical entities
// (tens to hundreds of units against a multi-million-unit world total)
// land in deep / medium blue. The middle of the ramp transitions
// through cyan + warm beige into a fire core: red-orange → orange →
// white-hot at the peak. Hot spots in a slice view (where the densest
// energy region of the 3D world projects through) read as glowing red
// even though most of the field stays cool.

export type Rgb = readonly [r: number, g: number, b: number];

interface HeatStop {
  readonly t: number;
  readonly rgb: Rgb;
}

export const HEAT_STOPS: readonly HeatStop[] = Object.freeze([
  { t: 0.00, rgb: [0.00, 0.00, 0.00] }, // black
  { t: 0.05, rgb: [0.05, 0.10, 0.35] }, // very dark blue
  { t: 0.15, rgb: [0.15, 0.30, 0.70] }, // deep blue
  { t: 0.30, rgb: [0.25, 0.55, 0.90] }, // medium blue (typical low-energy entity)
  { t: 0.45, rgb: [0.45, 0.80, 0.95] }, // cyan
  { t: 0.55, rgb: [0.85, 0.65, 0.55] }, // warm beige (cool→warm transition)
  { t: 0.68, rgb: [1.00, 0.40, 0.15] }, // red-orange (warm core)
  { t: 0.82, rgb: [1.00, 0.80, 0.20] }, // orange-yellow
  { t: 1.00, rgb: [1.00, 1.00, 0.95] }, // white-hot
] as const);

/** Map a normalized energy `t ∈ [0, 1]` to an RGB triple along the
 *  HEAT_STOPS ramp. Values outside [0, 1] are clamped. The mapping is
 *  piecewise linear between adjacent stops. */
export function heatColor(t: number): Rgb {
  const clamped = Math.max(0, Math.min(1, t));

  // Find the segment [i, i+1] whose lower bound is the largest stop ≤ t.
  let i = 0;
  while (i < HEAT_STOPS.length - 1) {
    const next = HEAT_STOPS[i + 1]!;
    if (clamped <= next.t) break;
    i += 1;
  }
  const a = HEAT_STOPS[i]!;
  const b = HEAT_STOPS[Math.min(i + 1, HEAT_STOPS.length - 1)]!;

  // HEAT_STOPS is strictly increasing in t (asserted by the tests), so
  // `b.t - a.t` is positive whenever a !== b. When `i` lands on the
  // last stop, `a === b` and `lerp = 0` cleanly returns `a.rgb`.
  const span = b.t - a.t;
  const lerp = a === b ? 0 : (clamped - a.t) / span;
  return [
    a.rgb[0] + (b.rgb[0] - a.rgb[0]) * lerp,
    a.rgb[1] + (b.rgb[1] - a.rgb[1]) * lerp,
    a.rgb[2] + (b.rgb[2] - a.rgb[2]) * lerp,
  ];
}

/** Translate a per-cell energy into a normalized `t ∈ [0, 1]` for the
 *  heat ramp using an absolute (logarithmic) scale relative to the
 *  world's total energy. Empty / non-positive cells map to 0; a cell
 *  holding all the world's energy maps to 1. The log compresses the
 *  range so a few hundred units against a multi-million-unit total
 *  still shows up clearly, while equilibrated fields fade toward the
 *  cool end of the ramp.
 *
 *  Compare with the prototype-9 "relative" scale `sqrt(e / maxE)`,
 *  which always paints the brightest cell white regardless of context. */
export function absoluteEnergyT(energy: number, totalEnergy: number): number {
  if (energy <= 0) return 0;
  const denom = Math.log1p(Math.max(1, totalEnergy));
  return Math.log1p(energy) / denom;
}

/** Where on the heat ramp a cell holding exactly the field's mean
 *  energy lands. Picked to put "the average cell" inside the medium
 *  blue band; below-average cells fall toward black, above-average
 *  toward the warm core. */
export const MEAN_T = 0.30;

/** Dynamic range covered by `meanRelativeT`: a cell holding `T_SPREAD`
 *  times the mean is white-hot (`t = 1`); a cell holding `1/T_SPREAD`
 *  of the mean is black (`t = 0`). 100 = two decades each side of
 *  the mean, which seems to match how aenternis fields actually
 *  spread out at runtime. */
export const T_SPREAD = 100;

/** Translate a per-cell energy into a normalized `t ∈ [0, 1]` for the
 *  heat ramp, using the *mean* energy of the field
 *  (`totalEnergy / cellCount`) as the neutral visual point. The
 *  mapping is logarithmic in `energy / mean` and asymmetric: cool half
 *  of the ramp covers below-mean cells, warm half covers above-mean.
 *
 *  This is a hybrid between the relative scale (`sqrt(e/maxE)`, always
 *  paints the brightest cell white) and the absolute scale
 *  (`log(1+e)/log(1+totalE)`, ignores how many cells share that
 *  total). It adapts to the current population: 100 j. against a
 *  10-cell field reads as "well above average" (warm); the same 100
 *  j. against a 100k-cell field reads as "around average" (blue). */
export function meanRelativeT(
  energy: number,
  totalEnergy: number,
  cellCount: number,
): number {
  if (energy <= 0) return 0;
  if (cellCount <= 0) return 0;
  const meanE = totalEnergy / cellCount;
  if (meanE <= 0) return 0;

  const ratio = energy / meanE;
  const tNorm = Math.log(ratio) / Math.log(T_SPREAD); // -1 .. +1 (unclamped)
  const clamped = Math.max(-1, Math.min(1, tNorm));

  // Asymmetric mapping: mean cell sits at MEAN_T (medium blue).
  // Below-mean fills [0, MEAN_T], above-mean fills [MEAN_T, 1].
  if (clamped <= 0) return MEAN_T * (clamped + 1);
  return MEAN_T + (1 - MEAN_T) * clamped;
}

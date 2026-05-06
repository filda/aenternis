// Heat-color ramp used by the WASM viewer to translate per-cell energy
// (normalized to [0, 1]) into RGB triples. Pure math, no DOM / THREE.
//
// The ramp is piecewise linear between fixed stops, biased toward the
// "warm" end so high-energy cells visually dominate.

export type Rgb = readonly [r: number, g: number, b: number];

interface HeatStop {
  readonly t: number;
  readonly rgb: Rgb;
}

export const HEAT_STOPS: readonly HeatStop[] = Object.freeze([
  { t: 0.00, rgb: [0.00, 0.00, 0.00] },
  { t: 0.10, rgb: [0.16, 0.24, 0.40] },
  { t: 0.30, rgb: [0.32, 0.08, 0.24] },
  { t: 0.55, rgb: [0.78, 0.28, 0.12] },
  { t: 0.80, rgb: [0.94, 0.78, 0.32] },
  { t: 1.00, rgb: [1.00, 1.00, 0.86] },
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

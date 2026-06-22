// Per-cell color modes for the WASM viewer. The default "energy" mode
// colors by the heat ramp (src/heat.ts). The "appearance" mode keeps that
// heat map and *tints* it toward the program-controlled war paint
// (`paint` opcode → the per-cell `appearance` field), so unpainted cells
// still read as energy and painted regions show as a colored cast at the
// heat ramp's own brightness. The "origin" mode recolors every cell by its
// lineage tag (`origin_tag`, propagated by dominance / metempsychosis),
// with energy driving brightness — a territory map.
//
// Pure math, no DOM / THREE.

import { heatColorInto, type MutableRgb, type Rgb } from './heat.ts';

export type ColorMode = 'energy' | 'appearance' | 'origin';

/** Saturation used by the hue-based modes (war paint, lineage). High
 *  enough that distinct tags are easy to tell apart, not fully saturated
 *  so the colors don't vibrate against the dark background. */
export const HUE_SATURATION = 0.85;

/** Brightness floor for the hue-based modes: a painted cell with near-zero
 *  energy still shows its hue at this value, so war paint never goes fully
 *  black. Energy (the heat-ramp `t`) scales brightness from here up to 1. */
export const HUE_VALUE_FLOOR = 0.25;

/** Map an HSV triple (`h, s, v` each in [0, 1], `h` wraps) to RGB in
 *  [0, 1]. Standard six-sector conversion. */
export function hsvToRgb(h: number, s: number, v: number): Rgb {
  const out: MutableRgb = [0, 0, 0];
  hsvToRgbInto(out, h, s, v);
  return out;
}

/** Alloc-free [`hsvToRgb`]: write the RGB for `(h, s, v)` into `out`. */
export function hsvToRgbInto(out: MutableRgb, h: number, s: number, v: number): void {
  const hue = ((h % 1) + 1) % 1; // wrap into [0, 1)
  const sector = Math.floor(hue * 6);
  const f = hue * 6 - sector;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  switch (sector % 6) {
    case 0: out[0] = v; out[1] = t; out[2] = p; break;
    case 1: out[0] = q; out[1] = v; out[2] = p; break;
    case 2: out[0] = p; out[1] = v; out[2] = t; break;
    case 3: out[0] = p; out[1] = q; out[2] = v; break;
    case 4: out[0] = t; out[1] = p; out[2] = v; break;
    default: out[0] = v; out[1] = p; out[2] = q; break;
  }
}

/** Derive a stable, well-spread hue in [0, 1) from a 32-bit tag. A bit-mix
 *  hash so adjacent tag values land on visibly different hues, folded to
 *  the unit interval. Deterministic — the same tag always gets the same
 *  hue across frames and reloads. */
export function tagHue(tag: number): number {
  let x = tag >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  x = (x ^ (x >>> 16)) >>> 0;
  return x / 4294967296;
}

/** HSV value (brightness) for the lineage mode as a function of the
 *  heat-ramp `t` (energy), clamped to [floor, 1] so even cold cells stay
 *  visible. */
export function hueValue(t: number): number {
  const clamped = Math.max(0, Math.min(1, t));
  return HUE_VALUE_FLOOR + (1 - HUE_VALUE_FLOOR) * clamped;
}

/** Blend weight of the war paint over the underlying heat color in
 *  `appearanceColor`. 0 = pure heat (paint invisible), 1 = pure paint hue.
 *  The default keeps a touch of the heat color so painted regions still
 *  read as energy, just recolored. */
export const PAINT_BLEND = 0.65;

/** War-paint color: the energy heat color tinted toward the cell's paint
 *  hue. Unpainted cells (`appearance === 0`) keep the plain heat color, so
 *  the field still reads as an energy map; painted cells are recolored to
 *  their hue at the heat ramp's own brightness and blended back over the
 *  heat color by `PAINT_BLEND`. */
export function appearanceColor(appearance: number, t: number): Rgb {
  const out: MutableRgb = [0, 0, 0];
  appearanceColorInto(out, appearance, t);
  return out;
}

/** Alloc-free [`appearanceColor`]: write the war-paint color into `out`. */
export function appearanceColorInto(out: MutableRgb, appearance: number, t: number): void {
  heatColorInto(out, t);
  if ((appearance >>> 0) === 0) return;
  // Heat-ramp brightness (= max channel) carries energy into the tint.
  const value = Math.max(out[0], out[1], out[2]);
  // Capture the heat color, overwrite `out` with the paint hue, then blend
  // back toward heat by `PAINT_BLEND` — no temp array.
  const h0 = out[0];
  const h1 = out[1];
  const h2 = out[2];
  hsvToRgbInto(out, tagHue(appearance), HUE_SATURATION, value);
  out[0] = h0 + (out[0] - h0) * PAINT_BLEND;
  out[1] = h1 + (out[1] - h1) * PAINT_BLEND;
  out[2] = h2 + (out[2] - h2) * PAINT_BLEND;
}

/** Lineage color: hue from the cell's `origin_tag`, brightness from
 *  energy. Cells sharing a lineage share a hue; dominance / metempsychosis
 *  repaints a region as its tag changes. */
export function originColor(originTag: number, t: number): Rgb {
  const out: MutableRgb = [0, 0, 0];
  originColorInto(out, originTag, t);
  return out;
}

/** Alloc-free [`originColor`]: write the lineage color into `out`. */
export function originColorInto(out: MutableRgb, originTag: number, t: number): void {
  hsvToRgbInto(out, tagHue(originTag), HUE_SATURATION, hueValue(t));
}

/** Dispatch a per-cell color by mode. `t` is the heat-ramp value (energy);
 *  `appearance` / `originTag` are the raw per-cell 32-bit fields. */
export function cellColor(
  mode: ColorMode,
  t: number,
  appearance: number,
  originTag: number,
): Rgb {
  const out: MutableRgb = [0, 0, 0];
  cellColorInto(out, mode, t, appearance, originTag);
  return out;
}

/** Alloc-free [`cellColor`]: write the per-cell color into `out`. Used by
 *  the render loop to color hundreds of thousands of cells per snapshot
 *  without allocating an `Rgb` array per cell. */
export function cellColorInto(
  out: MutableRgb,
  mode: ColorMode,
  t: number,
  appearance: number,
  originTag: number,
): void {
  switch (mode) {
    case 'appearance': appearanceColorInto(out, appearance, t); break;
    case 'origin': originColorInto(out, originTag, t); break;
    default: heatColorInto(out, t); break;
  }
}

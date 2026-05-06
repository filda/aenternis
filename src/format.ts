// Inspector-panel string formatting helpers — pure, no DOM access.
//
// `fmtDirArr` annotates a 6-element directional array with the canonical
// xp/xn/yp/yn/zp/zn labels. `fmtMemoryHexDump` produces a multi-line
// hex dump (8 slots per row, 4-digit address + 8-digit hex).
// `fmtBbox` formats a 6-element bounding box as a human-readable
// "(x0..x1, y0..y1, z0..z1) = dx×dy×dz" string used in the HUD.

export const DIR_LABELS: readonly string[] = Object.freeze([
  'xp', 'xn', 'yp', 'yn', 'zp', 'zn',
]);

/** Format a 6-element directional array (rates / pointers / etc.) as
 *  `"xp=v  xn=v  yp=v  yn=v  zp=v  zn=v"`. The input only has to be
 *  index-accessible; both `number[]` and `Uint32Array` work. */
export function fmtDirArr(arr: ArrayLike<number>): string {
  return DIR_LABELS.map((label, i) => `${label}=${arr[i]}`).join('  ');
}

/** Hex dump of the given slot array — 8 slots per row, each padded to
 *  8 hex digits, prefixed by a 4-digit address. The trailing row may
 *  hold fewer than 8 slots. Returns an empty string for empty input. */
export function fmtMemoryHexDump(slots: ArrayLike<number>): string {
  const lines: string[] = [];
  for (let i = 0; i < slots.length; i += 8) {
    const addr = i.toString(16).padStart(4, '0');
    const row: string[] = [];
    for (let j = 0; j < 8 && i + j < slots.length; j += 1) {
      // The loop bounds guarantee `i + j < slots.length`.
      row.push(slots[i + j]!.toString(16).padStart(8, '0'));
    }
    lines.push(`${addr}: ${row.join(' ')}`);
  }
  return lines.join('\n');
}

/** Bounding box from the worker is a 6-element `Int32Array`
 *  `[xMin, xMax, yMin, yMax, zMin, zMax]`. An empty world is signaled
 *  by an array of length 0; this helper returns `null` in that case so
 *  the caller can decide on its own placeholder rendering. */
export function fmtBbox(bbox: ArrayLike<number>): string | null {
  if (bbox.length !== 6) return null;
  const x0 = bbox[0]!;
  const x1 = bbox[1]!;
  const y0 = bbox[2]!;
  const y1 = bbox[3]!;
  const z0 = bbox[4]!;
  const z1 = bbox[5]!;
  const dx = x1 - x0 + 1;
  const dy = y1 - y0 + 1;
  const dz = z1 - z0 + 1;
  return `(${x0}..${x1}, ${y0}..${y1}, ${z0}..${z1}) = ${dx}×${dy}×${dz}`;
}

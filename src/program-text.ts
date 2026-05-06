// Thin wrapper around `assemble` that produces the human-readable
// status string the textarea status line displays. Pure — no DOM.

import { assemble } from './asm.ts';

export interface ProgramParseResult {
  /** Always-present output, even on parse errors (best-effort partial). */
  readonly program: Uint32Array;
  /** Single-line status: "empty", "N slot(s) assembled", or
   *  "N parse error(s): err1; err2; ...". */
  readonly status: string;
}

/** Parse the textarea's program source. Mirrors what `web/main.ts` did
 *  inline before phase 3, in particular the "empty" status when the
 *  source text has no slots and no errors. */
export function parseProgramText(text: string): ProgramParseResult {
  const { slots, errors } = assemble(text);
  if (errors.length > 0) {
    return {
      program: slots,
      status: `${errors.length} parse error(s): ${errors.join('; ')}`,
    };
  }
  if (slots.length === 0) {
    return { program: slots, status: 'empty' };
  }
  return { program: slots, status: `${slots.length} slot(s) assembled` };
}

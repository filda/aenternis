import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { OPCODES } from '../src/asm.ts';

// Cross-language parity guard: `src/asm.ts`'s `OPCODES` table is a
// hand-maintained mirror of the canonical `Opcode` enum in the Rust core
// (see docs/pilgrim.md, memory project-asm-ts-ui-helper). The VM executes
// machine code; the assembler is a nástavba that must emit exactly the bytes
// the VM expects. The existing asm.test.ts only checks count + uniqueness,
// which would NOT catch a reordered discriminant or a changed arg count —
// the program would silently assemble to the wrong machine code.
//
// This test parses the canonical source directly and fails the gate the
// moment the two drift. It deliberately avoids loading WASM (vitest runs in
// node with no wasm dependency today).
//
// TODO(dedup): once the TS table is generated from the Rust enum at build
// time (docs/pilgrim.md "Otevřené body"), this hand-mirror and its guard
// both go away.

const VM_RS = fileURLToPath(
  new URL('../crates/aenternis-core/src/vm.rs', import.meta.url),
);

interface RustOpcode {
  mnemonic: string;
  code: number;
  args: number;
}

/** Slice the source between two anchor strings; assert both anchors exist. */
function between(src: string, start: string, end: string): string {
  const i = src.indexOf(start);
  expect(i, `anchor "${start}" not found in vm.rs`).toBeGreaterThanOrEqual(0);
  const j = src.indexOf(end, i + start.length);
  expect(j, `anchor "${end}" not found after "${start}" in vm.rs`).toBeGreaterThan(i);
  return src.slice(i + start.length, j);
}

/**
 * Parse the canonical opcode table out of `vm.rs` from three regular
 * structures: the enum discriminants (variant → code), the `mnemonic()`
 * match (variant → mnemonic), and the `length()` match (variant → slot
 * width, so args = length − 1).
 */
function parseRustOpcodes(src: string): Map<string, RustOpcode> {
  // variant → code:  `    Nop = 0x00,`
  const variantCode = new Map<string, number>();
  const enumBody = between(src, 'pub enum Opcode {', 'impl Opcode');
  for (const m of enumBody.matchAll(/^\s*([A-Z]\w*)\s*=\s*0x([0-9A-Fa-f]+)\s*,/gm)) {
    variantCode.set(m[1]!, parseInt(m[2]!, 16));
  }

  // variant → mnemonic:  `Self::Nop => "nop",`  (restricted to mnemonic())
  const variantMnemonic = new Map<string, string>();
  const mnemonicBody = between(
    src,
    "pub const fn mnemonic(self) -> &'static str {",
    'pub fn from_mnemonic',
  );
  for (const m of mnemonicBody.matchAll(/Self::(\w+)\s*=>\s*"([a-z]+)"/g)) {
    variantMnemonic.set(m[1]!, m[2]!);
  }

  // variant → length (slots):  `Self::A | Self::B => 2,`  /  `Self::Nop => 1,`
  const variantLength = new Map<string, number>();
  const lengthBody = between(src, 'pub const fn length(self) -> u32 {', 'pub const ALL');
  for (const arm of lengthBody.matchAll(/((?:Self::\w+\s*\|?\s*)+)=>\s*(\d+)/g)) {
    const len = parseInt(arm[2]!, 10);
    for (const v of arm[1]!.matchAll(/Self::(\w+)/g)) {
      variantLength.set(v[1]!, len);
    }
  }

  const table = new Map<string, RustOpcode>();
  for (const [variant, code] of variantCode) {
    const mnemonic = variantMnemonic.get(variant);
    const length = variantLength.get(variant);
    expect(mnemonic, `vm.rs: no mnemonic() arm for ${variant}`).toBeTypeOf('string');
    expect(length, `vm.rs: no length() arm for ${variant}`).toBeTypeOf('number');
    table.set(mnemonic!, { mnemonic: mnemonic!, code, args: length! - 1 });
  }
  return table;
}

describe('opcode parity: src/asm.ts mirrors the Rust Opcode enum', () => {
  const rust = parseRustOpcodes(readFileSync(VM_RS, 'utf8'));

  it('parses a sane Rust table (31 contiguous opcodes 0x00..0x1E)', () => {
    expect(rust.size).toBe(31);
    const codes = [...rust.values()].map((o) => o.code).sort((a, b) => a - b);
    expect(codes).toEqual(Array.from({ length: 31 }, (_, i) => i));
  });

  it('has the same set of mnemonics on both sides', () => {
    expect(new Set(Object.keys(OPCODES))).toEqual(new Set(rust.keys()));
  });

  it('every opcode matches on code and arg count', () => {
    for (const [mnemonic, r] of rust) {
      const ts = OPCODES[mnemonic];
      expect(ts, `asm.ts OPCODES missing "${mnemonic}"`).toBeDefined();
      expect(ts!.code, `code for "${mnemonic}"`).toBe(r.code);
      expect(ts!.args, `args for "${mnemonic}"`).toBe(r.args);
    }
  });
});

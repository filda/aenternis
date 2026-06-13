//! Macro library for seeded procedural genesis.
//!
//! A **macro** is a parameterized assembler fragment — a tiny program
//! that "does something on its own". The genesis generator
//! ([`crate::genesis`]) composes a whole-memory program by streaming
//! weighted macros and filling their `{param}` holes from the seed tape;
//! the same library also feeds the UI snippet inserter. Macros are
//! **data**, authored in `src/macros/v1.aenm` (embedded below), so an
//! independent generator can build on the same set. See
//! `docs/genesis-plan.md`.
//!
//! This module is the **canonical** macro expander: a minimal subset of
//! the assembler (mnemonic lookup via [`Opcode`], operand parsing,
//! `{placeholder}` substitution). `src/asm.ts` mirrors it for the UI and
//! is parity-checked — `asm.ts` is a helper, this is the source of truth.

use std::sync::OnceLock;

use crate::Opcode;

/// Embedded v1 macro library source (authored assembler-syntax fragments).
const V1_SOURCE: &str = include_str!("macros/v1.aenm");

/// Parameter type. Determines how the generator samples a value from the
/// seed tape (see `docs/genesis-plan.md`, A5):
/// `Dir` → `draw % DIRS`, `Addr` → `draw % window`, `Const` → raw `draw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// A direction operand (`0..DIRS`).
    Dir,
    /// A memory address operand (sampled within the working window).
    Addr,
    /// An arbitrary 32-bit constant operand.
    Const,
}

impl ParamKind {
    fn parse(token: &str) -> Option<Self> {
        match token {
            "DIR" => Some(Self::Dir),
            "ADDR" => Some(Self::Addr),
            "CONST" => Some(Self::Const),
            _ => None,
        }
    }
}

/// One operand slot in a macro body: either a fixed literal or a
/// reference to one of the macro's parameters (by index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand {
    Literal(u32),
    Param(usize),
}

/// One instruction in a macro body.
#[derive(Debug, Clone)]
struct BodyOp {
    opcode: Opcode,
    operands: Vec<Operand>,
}

/// A parameterized assembler fragment.
#[derive(Debug, Clone)]
pub struct Macro {
    name: String,
    weight: u32,
    tags: Vec<String>,
    params: Vec<ParamKind>,
    body: Vec<BodyOp>,
    slot_len: u32,
}

impl Macro {
    /// Display name (for the UI inserter and debugging).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Selection weight (before the fertility multiplier).
    #[must_use]
    pub const fn weight(&self) -> u32 {
        self.weight
    }

    /// Whether this macro carries the `spread` tag — the target of the
    /// generator's fertility knob.
    #[must_use]
    pub fn is_spread(&self) -> bool {
        self.tags.iter().any(|t| t == "spread")
    }

    /// Parameter kinds, in operand order. The generator samples one tape
    /// draw per kind.
    #[must_use]
    pub fn param_kinds(&self) -> &[ParamKind] {
        &self.params
    }

    /// Total number of slots this macro expands to.
    #[must_use]
    pub const fn slot_len(&self) -> u32 {
        self.slot_len
    }

    /// Expand the macro into `out` starting at `*pos`, substituting
    /// `param_values` for `{param}` holes (one value per declared param,
    /// in order). Writing stops at `out.len()` — a partial final fragment
    /// is intentional (genesis truncates the last macro; see
    /// `docs/genesis-plan.md`, R2). Advances `*pos` past what was written.
    ///
    /// `param_values` must have one entry per [`Macro::param_kinds`].
    pub fn emit(&self, param_values: &[u32], out: &mut [u32], pos: &mut usize) {
        debug_assert_eq!(param_values.len(), self.params.len());
        for op in &self.body {
            if *pos >= out.len() {
                return;
            }
            out[*pos] = u32::from(op.opcode as u8);
            *pos += 1;
            for operand in &op.operands {
                if *pos >= out.len() {
                    return;
                }
                out[*pos] = match *operand {
                    Operand::Literal(v) => v,
                    Operand::Param(i) => param_values[i],
                };
                *pos += 1;
            }
        }
    }
}

/// The parsed v1 macro library, parsed once on first access.
///
/// # Panics
///
/// Panics if the embedded library fails to parse — that is a programming
/// error in `src/macros/v1.aenm`, caught by the unit tests below.
#[must_use]
pub fn library() -> &'static [Macro] {
    static LIB: OnceLock<Vec<Macro>> = OnceLock::new();
    LIB.get_or_init(|| parse_library(V1_SOURCE).expect("embedded v1.aenm must parse"))
}

/// In-progress macro being assembled by [`parse_library`].
struct Builder {
    name: String,
    weight: u32,
    tags: Vec<String>,
    param_names: Vec<String>,
    params: Vec<ParamKind>,
    body_lines: Vec<(usize, String)>,
}

/// Parse a macro-library source string into a list of [`Macro`]s.
///
/// Returns a human-readable error (with 1-based line number) on the first
/// malformed macro. Exposed for tests and external generators.
///
/// # Errors
///
/// Returns `Err` if a directive is malformed, a parameter type is
/// unknown, a body mnemonic is unknown, an operand count mismatches the
/// opcode, or an operand references an undeclared parameter.
pub fn parse_library(src: &str) -> Result<Vec<Macro>, String> {
    let mut macros = Vec::new();
    let mut cur: Option<Builder> = None;

    for (idx, raw) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = raw.trim();

        if let Some(rest) = trimmed.strip_prefix(';') {
            let directive = rest.trim();
            if let Some(header) = directive.strip_prefix("@macro") {
                if let Some(b) = cur.take() {
                    macros.push(finalize(b)?);
                }
                cur = Some(parse_header(header.trim(), lineno)?);
            } else if let Some(decl) = directive.strip_prefix("@param") {
                let b = cur
                    .as_mut()
                    .ok_or_else(|| format!("line {lineno}: @param outside a macro"))?;
                let (name, kind) = parse_param(decl.trim(), lineno)?;
                b.param_names.push(name);
                b.params.push(kind);
            }
            // any other `;` line is a plain comment — ignore
            continue;
        }

        // strip a trailing inline comment, then trim
        let code = trimmed.split(';').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        let b = cur
            .as_mut()
            .ok_or_else(|| format!("line {lineno}: body line outside a macro"))?;
        b.body_lines.push((lineno, code.to_string()));
    }

    if let Some(b) = cur.take() {
        macros.push(finalize(b)?);
    }
    Ok(macros)
}

/// Parse the `NAME weight=W tags=A,B` tail of a `; @macro` directive.
fn parse_header(header: &str, lineno: usize) -> Result<Builder, String> {
    let mut it = header.split_whitespace();
    let name = it
        .next()
        .ok_or_else(|| format!("line {lineno}: @macro needs a name"))?
        .to_string();
    let mut weight = 1u32;
    let mut tags = Vec::new();
    for tok in it {
        if let Some(w) = tok.strip_prefix("weight=") {
            weight = w
                .parse()
                .map_err(|_| format!("line {lineno}: bad weight \"{w}\""))?;
        } else if let Some(t) = tok.strip_prefix("tags=") {
            tags = t.split(',').map(|s| s.trim().to_string()).collect();
        } else {
            return Err(format!("line {lineno}: unknown @macro attribute \"{tok}\""));
        }
    }
    Ok(Builder {
        name,
        weight,
        tags,
        param_names: Vec::new(),
        params: Vec::new(),
        body_lines: Vec::new(),
    })
}

/// Parse a `; @param NAME TYPE` declaration.
fn parse_param(decl: &str, lineno: usize) -> Result<(String, ParamKind), String> {
    let mut it = decl.split_whitespace();
    let name = it
        .next()
        .ok_or_else(|| format!("line {lineno}: @param needs a name"))?
        .to_string();
    let kind_tok = it
        .next()
        .ok_or_else(|| format!("line {lineno}: @param needs a type"))?;
    let kind = ParamKind::parse(kind_tok)
        .ok_or_else(|| format!("line {lineno}: unknown param type \"{kind_tok}\""))?;
    Ok((name, kind))
}

/// Resolve a builder's body lines into a finished [`Macro`].
fn finalize(b: Builder) -> Result<Macro, String> {
    let mut body = Vec::with_capacity(b.body_lines.len());
    let mut slot_len = 0u32;

    for (lineno, line) in &b.body_lines {
        let mut parts = line.splitn(2, char::is_whitespace);
        let mnemonic = parts.next().unwrap_or("");
        let opcode = Opcode::from_mnemonic(mnemonic)
            .ok_or_else(|| format!("line {lineno}: unknown mnemonic \"{mnemonic}\""))?;

        // Body lines are already trimmed upstream, so any `rest` after the
        // mnemonic's first whitespace is non-empty; a zero-operand
        // instruction (e.g. bare `nop`) has no `rest` at all.
        let operands: Vec<&str> = parts
            .next()
            .map_or_else(Vec::new, |rest| rest.split(',').map(str::trim).collect());
        if operands.len() as u32 != opcode.arg_count() {
            return Err(format!(
                "line {lineno}: {mnemonic} expects {} operand(s), got {}",
                opcode.arg_count(),
                operands.len()
            ));
        }

        let mut resolved = Vec::with_capacity(operands.len());
        for tok in operands {
            resolved.push(resolve_operand(tok, &b.param_names, *lineno)?);
        }
        slot_len += opcode.length();
        body.push(BodyOp {
            opcode,
            operands: resolved,
        });
    }

    Ok(Macro {
        name: b.name,
        weight: b.weight,
        tags: b.tags,
        params: b.params,
        body,
        slot_len,
    })
}

/// Resolve one operand token to a literal or a parameter reference.
fn resolve_operand(token: &str, param_names: &[String], lineno: usize) -> Result<Operand, String> {
    if let Some(inner) = token.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return param_names
            .iter()
            .position(|p| p == inner)
            .map(Operand::Param)
            .ok_or_else(|| format!("line {lineno}: unknown param \"{{{inner}}}\""));
    }
    parse_literal(token)
        .map(Operand::Literal)
        .ok_or_else(|| format!("line {lineno}: cannot parse operand \"{token}\""))
}

/// Parse a literal operand: direction name, hex (`0x..`), or decimal
/// (negatives wrap to two's complement u32). Mirrors `asm.ts`.
fn parse_literal(token: &str) -> Option<u32> {
    match token {
        "xp" => return Some(0),
        "xn" => return Some(1),
        "yp" => return Some(2),
        "yn" => return Some(3),
        "zp" => return Some(4),
        "zn" => return Some(5),
        _ => {}
    }
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Ok(v) = token.parse::<u32>() {
        return Some(v);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    token.parse::<i64>().ok().map(|v| v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_library_parses_and_is_nonempty() {
        let lib = library();
        assert!(lib.len() >= 20, "v1 library should have a rich set");
    }

    #[test]
    fn every_macro_is_well_formed() {
        for m in library() {
            assert!(m.weight() >= 1, "{}: weight must be >= 1", m.name());
            assert!(m.slot_len() >= 1, "{}: must emit >= 1 slot", m.name());
            assert!(!m.name().is_empty());
        }
    }

    #[test]
    fn at_least_one_spread_macro_exists() {
        assert!(
            library().iter().any(Macro::is_spread),
            "need a spread macro or no world can ever propagate"
        );
    }

    fn find(name: &str) -> &'static Macro {
        library().iter().find(|m| m.name() == name).unwrap()
    }

    #[test]
    fn weight_tiers_are_as_specified() {
        assert_eq!(find("and").weight(), 5, "bitwise gadgets are HIGH tier");
        assert_eq!(find("replicate").weight(), 3, "replicator is MED tier");
        assert_eq!(find("acc_mul").weight(), 1, "exotic arithmetic is LOW tier");
    }

    #[test]
    fn spread_tag_is_detected_precisely() {
        assert!(find("replicate").is_spread(), "setp is a spread macro");
        assert!(find("ignite").is_spread(), "port is a spread macro");
        assert!(!find("and").is_spread(), "a bitwise gadget is not spread");
    }

    #[test]
    fn emit_substitutes_params_and_advances() {
        // `set {a}, {v}` → opcode Set, then a, then v.
        let m = library().iter().find(|m| m.name() == "set").unwrap();
        assert_eq!(m.param_kinds(), &[ParamKind::Addr, ParamKind::Const]);
        let mut out = [0u32; 8];
        let mut pos = 0usize;
        m.emit(&[7, 42], &mut out, &mut pos);
        assert_eq!(pos, 3);
        assert_eq!(out[0], u32::from(Opcode::Set as u8));
        assert_eq!(out[1], 7);
        assert_eq!(out[2], 42);
    }

    #[test]
    fn emit_truncates_at_buffer_end() {
        let m = library().iter().find(|m| m.name() == "set").unwrap();
        let mut out = [0u32; 2]; // room for opcode + 1 operand only
        let mut pos = 0usize;
        m.emit(&[7, 42], &mut out, &mut pos);
        assert_eq!(pos, 2, "must stop exactly at the buffer end");
        assert_eq!(out[0], u32::from(Opcode::Set as u8));
        assert_eq!(out[1], 7);
    }

    #[test]
    fn composite_macro_expands_all_instructions() {
        // sense_mix: `senergy {d},{a}` + `add {b},{a}` → 6 slots.
        let m = library().iter().find(|m| m.name() == "sense_mix").unwrap();
        assert_eq!(m.slot_len(), 6);
        let mut out = [0u32; 6];
        let mut pos = 0usize;
        m.emit(&[4, 1, 2], &mut out, &mut pos);
        assert_eq!(pos, 6);
        assert_eq!(out[0], u32::from(Opcode::Senergy as u8));
        assert_eq!(out[3], u32::from(Opcode::Add as u8));
    }

    #[test]
    fn parse_rejects_operand_count_mismatch() {
        let src = "; @macro bad weight=1\n; @param a ADDR\n  set {a}\n";
        assert!(parse_library(src).is_err());
    }

    #[test]
    fn parse_rejects_unknown_param() {
        let src = "; @macro bad weight=1\n; @param a ADDR\n  inc {b}\n";
        assert!(parse_library(src).is_err());
    }

    #[test]
    fn parse_rejects_unknown_mnemonic() {
        let src = "; @macro bad weight=1\n  florp 1\n";
        let err = parse_library(src).unwrap_err();
        assert!(
            err.contains("line 2"),
            "error should report the line: {err}"
        );
    }

    #[test]
    fn parse_literal_handles_directions_hex_decimal() {
        assert_eq!(parse_literal("xp"), Some(0));
        assert_eq!(parse_literal("xn"), Some(1));
        assert_eq!(parse_literal("yp"), Some(2));
        assert_eq!(parse_literal("yn"), Some(3));
        assert_eq!(parse_literal("zp"), Some(4));
        assert_eq!(parse_literal("zn"), Some(5));
        assert_eq!(parse_literal("0xFF"), Some(255));
        assert_eq!(parse_literal("42"), Some(42));
        assert_eq!(parse_literal("-1"), Some(u32::MAX));
        assert_eq!(parse_literal("garbage"), None);
    }
}

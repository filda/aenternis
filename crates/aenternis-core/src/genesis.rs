//! Seeded procedural genesis: a world's initial program, composed from
//! macros and driven by the world seed.
//!
//! Instead of pure RNG noise, the origin cell's whole memory is filled by
//! streaming weighted [`macros`](crate::macros) and filling their
//! `{param}` holes from the seed tape. The result is a variable-but-
//! information-bearing initial condition: same seed → same program, but
//! different seeds give different "universes". See `docs/genesis-plan.md`.
//!
//! v1 is a **weighted stream**, framed as a genotype→phenotype decode of
//! the seed tape (the RNG stream is the genome; one `u32` draw per
//! decision). The stream is aperiodic and covers the whole memory, so any
//! slice the cell later emits carries real (if partial) program — "broken"
//! fragments are fuel for recombination/selection, not a defect (R2).

use crate::coord::Direction;
use crate::macros::{library, Macro, ParamKind};
use crate::Rng;

/// Generator policy — knobs of the **generator**, not of the VM or macros.
///
/// The VM only knows modular addressing, and a macro only declares operand
/// *types*. A different generator may sample the same library differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenesisConfig {
    /// Shared working-window size `A`: `ADDR` operands are sampled in
    /// `[0, window)`. Small enough (vs memory) that macros collide on a
    /// shared "register file" and can actually compute together;
    /// `% memSize` would be sterile. See `docs/genesis-plan.md`, A5.
    pub window: u32,
    /// Fertility multiplier applied to the weight of `spread`-tagged
    /// macros (replicator / igniter). `1.0` is neutral; higher = livelier
    /// (more propagation), `0.0` = no spread macros at all.
    pub fertility: f64,
}

impl Default for GenesisConfig {
    fn default() -> Self {
        Self {
            window: 256,
            fertility: 1.0,
        }
    }
}

/// Fill `out` with a procedurally generated program, drawing every
/// decision from `tape`. Deterministic in `(tape state, cfg)`.
///
/// Writes exactly `out.len()` slots — the final macro is truncated if it
/// would overflow, and a partial fragment is fine (R2).
///
/// If the effective weight of every macro is zero (e.g. a library of only
/// `spread` macros with `fertility == 0`), `out` is left as-is (`nop`s) —
/// a safe degenerate world rather than a panic.
pub fn generate_into(out: &mut [u32], tape: &mut Rng, cfg: &GenesisConfig) {
    let lib = library();
    let weights: Vec<u32> = lib.iter().map(|m| effective_weight(m, cfg)).collect();
    let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    if total == 0 {
        return;
    }
    let window = cfg.window.max(1);

    let mut pos = 0usize;
    while pos < out.len() {
        let m = pick(lib, &weights, total, tape.next_u32());
        let values: Vec<u32> = m
            .param_kinds()
            .iter()
            .map(|&kind| sample_param(kind, tape.next_u32(), window))
            .collect();
        m.emit(&values, out, &mut pos);
    }
}

/// Effective selection weight: base weight scaled by fertility for
/// `spread` macros, rounded to the nearest integer.
fn effective_weight(m: &Macro, cfg: &GenesisConfig) -> u32 {
    if m.is_spread() {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (f64::from(m.weight()) * cfg.fertility).round() as u32;
        scaled
    } else {
        m.weight()
    }
}

/// Map a tape draw onto the macro whose cumulative weight bucket it lands
/// in. `total` is the sum of `weights` (always > 0 here).
fn pick<'a>(lib: &'a [Macro], weights: &[u32], total: u64, draw: u32) -> &'a Macro {
    let mut target = u64::from(draw) % total;
    for (m, &w) in lib.iter().zip(weights) {
        let w = u64::from(w);
        if target < w {
            return m;
        }
        target -= w;
    }
    // Unreachable: target < total == Σweights, so the loop always hits a
    // bucket. Fall back to the first macro to keep the function total.
    &lib[0]
}

/// Map one tape draw onto a concrete operand value for the given kind.
const fn sample_param(kind: ParamKind, draw: u32, window: u32) -> u32 {
    match kind {
        ParamKind::Dir => draw % Direction::COUNT as u32,
        ParamKind::Addr => draw % window,
        ParamKind::Const => draw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(seed: u32, len: usize, cfg: &GenesisConfig) -> Vec<u32> {
        let mut out = vec![0u32; len];
        let mut tape = Rng::new(seed);
        generate_into(&mut out, &mut tape, cfg);
        out
    }

    #[test]
    fn fills_exactly_and_is_deterministic() {
        let cfg = GenesisConfig::default();
        let a = gen(12345, 500, &cfg);
        let b = gen(12345, 500, &cfg);
        assert_eq!(a.len(), 500);
        assert_eq!(a, b, "same seed must produce the same program");
    }

    #[test]
    fn different_seeds_differ() {
        let cfg = GenesisConfig::default();
        assert_ne!(gen(1, 500, &cfg), gen(2, 500, &cfg));
    }

    #[test]
    fn sample_dir_in_range() {
        for d in 0..1000u32 {
            assert!(sample_param(ParamKind::Dir, d.wrapping_mul(2_654_435_761), 256) < 6);
        }
    }

    #[test]
    fn sample_addr_within_window() {
        for d in 0..1000u32 {
            let a = sample_param(ParamKind::Addr, d.wrapping_mul(2_654_435_761), 128);
            assert!(a < 128, "addr {a} escaped the window");
        }
    }

    #[test]
    fn sample_const_is_raw_draw() {
        assert_eq!(
            sample_param(ParamKind::Const, 0xDEAD_BEEF, 256),
            0xDEAD_BEEF
        );
    }

    #[test]
    fn fertility_zero_drops_spread_but_still_fills() {
        let cfg = GenesisConfig {
            window: 256,
            fertility: 0.0,
        };
        // With no spread macros, generation must still fill (non-spread
        // weights remain) rather than panic or stall.
        let out = gen(999, 300, &cfg);
        assert_eq!(out.len(), 300);
    }

    #[test]
    fn effective_weight_scales_spread_by_fertility() {
        let cfg = GenesisConfig {
            window: 256,
            fertility: 2.0,
        };
        let lib = library();
        let spread = lib.iter().find(|m| m.is_spread()).unwrap();
        let plain = lib.iter().find(|m| !m.is_spread()).unwrap();
        // spread weight is doubled (× fertility); plain weight is untouched.
        assert_eq!(effective_weight(spread, &cfg), spread.weight() * 2);
        assert_eq!(effective_weight(plain, &cfg), plain.weight());
    }

    #[test]
    fn pick_distributes_exactly_by_weight() {
        // Over the contiguous draw range `[0, total)`, every macro must be
        // picked exactly `weight` times — pins the modulo, the bucket
        // comparison, and the running subtraction in `pick`.
        let cfg = GenesisConfig::default();
        let lib = library();
        let weights: Vec<u32> = lib.iter().map(|m| effective_weight(m, &cfg)).collect();
        let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
        let mut counts = vec![0u32; lib.len()];
        for draw in 0..u32::try_from(total).unwrap() {
            let m = pick(lib, &weights, total, draw);
            let idx = lib.iter().position(|x| std::ptr::eq(x, m)).unwrap();
            counts[idx] += 1;
        }
        assert_eq!(counts, weights);
    }

    #[test]
    fn larger_window_changes_addresses() {
        // Distinct windows generally yield distinct programs (addresses
        // are reduced differently), confirming the window actually bites.
        let small = gen(
            7,
            400,
            &GenesisConfig {
                window: 16,
                fertility: 1.0,
            },
        );
        let large = gen(
            7,
            400,
            &GenesisConfig {
                window: 1024,
                fertility: 1.0,
            },
        );
        assert_ne!(small, large);
    }
}

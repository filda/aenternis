//! Code-diversity metrics over the living population.
//!
//! The viewer's energy/heat visual is gravity-driven and program-blind, and
//! `origin_tag` collapses to a single lineage under a one-point big-bang (the
//! dominance conversion wave). To see whether genesis knobs leave a *lasting*
//! signature in the **code** — or whether mutation homogenizes it — we look at
//! the programs themselves.
//!
//! Every memory slot decodes to a valid opcode via the same fold the VM uses
//! ([`Opcode::decode`] = low byte `% COUNT`), so a cell's static byte
//! composition is summarized by an **opcode histogram**. Two aggregations:
//!
//! - **A — global distribution:** pool every living slot into one histogram;
//!   its Shannon entropy is a scalar "how varied is the code" readout.
//! - **B — per-cell diversity:** normalize each cell into a 31-bin probability
//!   vector and measure how far cells spread from the population mean (mean
//!   total-variation distance), plus a count of distinct *quantized*
//!   fingerprints (behavioral "types").
//!
//! This is a static genome-composition fingerprint, not an execution trace:
//! every slot counts as one opcode regardless of whether the PC would treat it
//! as an operand. That makes it deterministic and alignment-free (instruction
//! boundaries depend on the PC trajectory, which jumps/wraps), and it matches
//! the "whole memory is code" model. The fold keeps only the low byte, so the
//! fingerprint captures *which operations* a genome is made of, not its exact
//! operand bytes — the right granularity for a behavior-homogenization probe.

// Counts (slot tallies, cell counts) are cast to `f64` for probabilities,
// entropy, and distances. Slot counts are bounded by total energy and cell
// counts by the world size — both far below `f64`'s 2^52 exact-integer
// range — so the precision-loss lint does not apply to this statistical math.
#![allow(clippy::cast_precision_loss)]

use crate::vm::Opcode;
use crate::SparseWorld;

/// Number of opcode bins — the VM's opcode count (currently 31).
pub const OPCODE_BINS: usize = Opcode::COUNT as usize;

/// Quantization levels for the per-cell fingerprint used by `unique_types`.
/// Each of the 31 normalized bins is rounded to `[0, QUANT_LEVELS]`, so two
/// cells collapse to the same "type" when their opcode mix matches to within
/// `1 / QUANT_LEVELS`. Coarse on purpose: counts behavioral types, not the
/// near-infinite distinct exact mixes.
const QUANT_LEVELS: f64 = 16.0;

/// Aggregate code metrics for one sampling instant. See module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeMetrics {
    /// Number of living cells sampled.
    pub cells: u32,
    /// Global pooled opcode counts over every living slot.
    pub opcode_hist: [u64; OPCODE_BINS],
    /// Shannon entropy of the global distribution, in bits
    /// (`0 ..= log2(OPCODE_BINS)`). High = varied code, low = dominated by a
    /// few opcodes.
    pub entropy_bits: f64,
    /// Mean total-variation distance of each cell's normalized opcode vector
    /// from the population mean, in `[0, 1]`. `0` = every cell has the same
    /// opcode mix; higher = distinct sub-populations.
    pub cell_diversity: f64,
    /// Number of distinct quantized per-cell fingerprints (behavioral types).
    pub unique_types: u32,
}

impl CodeMetrics {
    /// The empty-world result: no cells, all-zero histogram, zero scalars.
    const fn empty() -> Self {
        Self {
            cells: 0,
            opcode_hist: [0; OPCODE_BINS],
            entropy_bits: 0.0,
            cell_diversity: 0.0,
            unique_types: 0,
        }
    }
}

/// Opcode-bin index for a slot — the VM fold (`Opcode::decode`), as an index.
#[inline]
const fn opcode_bin(slot: u32) -> usize {
    (slot as u8 % Opcode::COUNT) as usize
}

/// Accumulate `cell`'s per-bin opcode counts into `hist` (cleared first).
/// Returns the cell's slot count (`mem_len`).
fn cell_hist(slots: &[u32], hist: &mut [u64; OPCODE_BINS]) -> u64 {
    *hist = [0; OPCODE_BINS];
    for &slot in slots {
        hist[opcode_bin(slot)] += 1;
    }
    slots.len() as u64
}

/// Compute [`CodeMetrics`] over every living cell.
///
/// `O(total_energy)` in slot reads (decode is a mask + modulo), walked twice —
/// once to pool the global histogram and the population mean, once to measure
/// spread from that mean.
#[must_use]
pub fn compute_metrics(world: &SparseWorld) -> CodeMetrics {
    let cells = world.len();
    if cells == 0 {
        return CodeMetrics::empty();
    }
    let arena = world.arena();

    // --- Pass 1: global pool, population mean, distinct types ---
    let mut pool = [0u64; OPCODE_BINS];
    let mut mean = [0f64; OPCODE_BINS];
    let mut total_slots = 0u64;
    let mut scratch = [0u64; OPCODE_BINS];
    let mut types: std::collections::HashSet<[u8; OPCODE_BINS]> = std::collections::HashSet::new();
    for (_coord, cell) in world.sorted_iter() {
        let len = cell_hist(cell.memory(arena), &mut scratch);
        if len == 0 {
            continue;
        }
        let inv = 1.0 / len as f64;
        let mut fp = [0u8; OPCODE_BINS];
        for op in 0..OPCODE_BINS {
            pool[op] += scratch[op];
            let f = scratch[op] as f64 * inv;
            mean[op] += f;
            // Round, not truncate, so a bin near a level boundary lands on the
            // nearer type rather than always flooring.
            fp[op] = (f * QUANT_LEVELS).round() as u8;
        }
        types.insert(fp);
        total_slots += len;
    }
    let n = cells as f64;
    for m in &mut mean {
        *m /= n;
    }

    // --- Global Shannon entropy (bits) over the pooled distribution ---
    // `total_slots > 0` whenever `cells > 0` (every living cell holds at least
    // one slot), so no zero guard is needed here.
    let inv_total = 1.0 / total_slots as f64;
    let mut entropy_bits = 0.0;
    for &c in &pool {
        if c > 0 {
            let p = c as f64 * inv_total;
            entropy_bits -= p * p.log2();
        }
    }

    // --- Pass 2: mean total-variation distance from the population mean ---
    let mut tv_sum = 0.0;
    for (_coord, cell) in world.sorted_iter() {
        let len = cell_hist(cell.memory(arena), &mut scratch);
        if len == 0 {
            continue;
        }
        let inv = 1.0 / len as f64;
        let mut abs_sum = 0.0;
        for op in 0..OPCODE_BINS {
            let f = scratch[op] as f64 * inv;
            abs_sum += (f - mean[op]).abs();
        }
        tv_sum = 0.5f64.mul_add(abs_sum, tv_sum);
    }

    CodeMetrics {
        cells: cells as u32,
        opcode_hist: pool,
        entropy_bits,
        cell_diversity: tv_sum / n,
        unique_types: types.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Coord, SparseWorld};

    /// Build a world whose living cells have exactly the given memories. The
    /// first program seeds the origin (noise base, energy == len → whole
    /// memory is the program); the rest are inserted at distinct coords.
    fn world_with_cells(programs: &[&[u32]]) -> SparseWorld {
        let first = programs[0];
        let mut w = SparseWorld::big_bang_with_program(1, first.len() as u32, first);
        for (i, prog) in programs.iter().enumerate().skip(1) {
            let x = i32::try_from(i).expect("test cell index fits i32");
            w.insert_with_memory(Coord::new(x, 0, 0), prog);
        }
        // `compute_metrics` walks `sorted_iter`, which requires a clean cache;
        // in production a preceding `tick::step` rebuilds it, but manual
        // inserts leave it dirty, so rebuild here.
        w.rebuild_indices_if_dirty();
        w
    }

    #[test]
    fn empty_world_is_all_zero() {
        let w = SparseWorld::big_bang(1, 0);
        let m = compute_metrics(&w);
        assert_eq!(m, CodeMetrics::empty());
        assert_eq!(m.cells, 0);
    }

    #[test]
    fn opcode_bin_mirrors_decode() {
        // The bin index must equal the opcode the VM would decode.
        for slot in [0u32, 1, 30, 31, 255, 256, 0xDEAD_BEEF, u32::MAX] {
            assert_eq!(opcode_bin(slot), Opcode::decode(slot) as u8 as usize);
        }
    }

    #[test]
    fn histogram_sums_to_total_energy() {
        let w = SparseWorld::big_bang_macros(1234, 4096);
        let m = compute_metrics(&w);
        let hist_total: u64 = m.opcode_hist.iter().sum();
        assert_eq!(hist_total, w.total_energy(), "every slot is one opcode");
    }

    #[test]
    fn single_cell_has_zero_diversity() {
        // One cell is trivially equal to the population mean.
        let w = SparseWorld::big_bang_macros(7, 500);
        let m = compute_metrics(&w);
        assert_eq!(m.cells, 1);
        assert!(
            m.cell_diversity.abs() < 1e-12,
            "lone cell must sit on the mean (got {})",
            m.cell_diversity
        );
        assert_eq!(m.unique_types, 1);
    }

    #[test]
    fn uniform_program_has_zero_entropy() {
        // A program of a single repeated opcode (slot 0 → opcode 0) pools into
        // one bin → entropy 0.
        let w = SparseWorld::big_bang_with_program(9, 64, &vec![0u32; 64]);
        let m = compute_metrics(&w);
        assert_eq!(m.opcode_hist[0], 64);
        assert!(
            m.entropy_bits.abs() < 1e-12,
            "single-opcode genome has zero entropy (got {})",
            m.entropy_bits
        );
    }

    #[test]
    fn diversity_and_entropy_for_two_to_one_split() {
        // Two all-opcode-0 cells (len 4) + one all-opcode-1 cell (len 4).
        let zeros = [0u32; 4];
        let ones = [1u32; 4];
        let w = world_with_cells(&[&zeros, &zeros, &ones]);
        let m = compute_metrics(&w);

        assert_eq!(m.cells, 3);
        assert_eq!(m.unique_types, 2, "two distinct opcode mixes");

        // Global pool: opcode 0 → 8 slots, opcode 1 → 4, total 12.
        assert_eq!(m.opcode_hist[0], 8);
        assert_eq!(m.opcode_hist[1], 4);
        let p0: f64 = 8.0 / 12.0;
        let p1: f64 = 4.0 / 12.0;
        // Two separate terms (not a fused mul-add) to mirror the production
        // accumulation and keep clippy's `suboptimal_flops` quiet.
        let h0 = -(p0 * p0.log2());
        let h1 = -(p1 * p1.log2());
        let expected_h = h0 + h1;
        assert!(
            (m.entropy_bits - expected_h).abs() < 1e-12,
            "entropy {} vs expected {expected_h}",
            m.entropy_bits
        );

        // mean = [2/3, 1/3, 0..]; TV(zeros) = 1/3 (twice), TV(ones) = 2/3,
        // so cell_diversity = (1/3 + 1/3 + 2/3) / 3 = 4/9. Pins the per-cell
        // normalization, the mean division, the TV accumulation, and the
        // final average all at once.
        assert!(
            (m.cell_diversity - 4.0 / 9.0).abs() < 1e-12,
            "diversity {} vs expected {}",
            m.cell_diversity,
            4.0 / 9.0
        );
    }

    #[test]
    fn quantized_fingerprint_distinguishes_close_mixes() {
        // Two cells over the same two opcodes (0 and 2) in different ratios
        // (4:12 vs 7:9). The `* QUANT_LEVELS` fingerprint keeps them as two
        // distinct types; a corrupted quantizer (e.g. `+`/`/` instead of `*`)
        // collapses both to one, which this pins.
        let mut a = vec![0u32; 4];
        a.resize(16, 2); // 4 × opcode 0, 12 × opcode 2
        let mut b = vec![0u32; 7];
        b.resize(16, 2); // 7 × opcode 0, 9 × opcode 2
        let w = world_with_cells(&[&a, &b]);
        let m = compute_metrics(&w);

        assert_eq!(m.cells, 2);
        assert_eq!(
            m.unique_types, 2,
            "distinct opcode ratios are distinct types"
        );
    }

    #[test]
    fn even_opcode_spread_approaches_max_entropy() {
        // Slots 0..OPCODE_BINS each appear once → uniform distribution →
        // entropy == log2(OPCODE_BINS).
        let prog: Vec<u32> = (0..OPCODE_BINS as u32).collect();
        let w = SparseWorld::big_bang_with_program(3, OPCODE_BINS as u32, &prog);
        let m = compute_metrics(&w);
        let max = (OPCODE_BINS as f64).log2();
        assert!(
            (m.entropy_bits - max).abs() < 1e-9,
            "uniform genome entropy {} should equal log2({}) = {}",
            m.entropy_bits,
            OPCODE_BINS,
            max
        );
    }
}

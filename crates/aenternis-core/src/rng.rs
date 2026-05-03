//! Deterministic random number generator for the simulation.
//!
//! Aenternis uses **PCG-XSH-RR-64/32** — a small permuted congruential
//! generator with 64 bits of state and 32 bits of output. Reasons:
//!
//! - **deterministic and reproducible**: the same seed yields the same stream
//!   on every host, every run, every tick — period `2^64`
//! - **statistically solid**: passes `BigCrush`, unlike `xorshift32`
//! - **fast and small**: a single 64-bit multiply + xor + rotate per output,
//!   no tables, no SIMD requirements
//! - **splittable**: a generator can be derived from `(world_seed, tick,
//!   coord)` with no shared state, so cell-local randomness is independent
//!   of cell allocation history. This is load-bearing for the sparse world
//!   model — cells that pop in and out of existence must not influence the
//!   randomness seen by other cells.
//!
//! Reference: Melissa O'Neill, "PCG: A Family of Simple Fast Space-Efficient
//! Statistically Good Algorithms for Random Number Generation" (2014).

use crate::Coord;

/// PCG-XSH-RR-64/32 generator.
///
/// Cheap to clone — the entire state is one `u64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Multiplier from the original PCG paper.
    const MUL: u64 = 6_364_136_223_846_793_005;

    /// Increment from the original PCG paper (default stream).
    const INC: u64 = 1_442_695_040_888_963_407;

    /// Build a generator from a 64-bit seed.
    ///
    /// The seed is mixed once before any output is produced, so even adjacent
    /// seeds (`0`, `1`, `2`, …) yield uncorrelated streams.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        // Initial state = seed advanced one PCG step from zero.
        let state = seed.wrapping_add(Self::INC);
        let state = state.wrapping_mul(Self::MUL).wrapping_add(Self::INC);
        Self { state }
    }

    /// Build a generator deterministically derived from a world seed.
    ///
    /// Use for world-level operations that are not associated with any cell
    /// — for example generating the big bang program at world bootstrap.
    #[must_use]
    pub const fn for_world(world_seed: u64) -> Self {
        Self::new(splitmix64(world_seed))
    }

    /// Build a generator deterministic in `(world_seed, tick, coord)`.
    ///
    /// The intended use is "call once at the start of every per-cell
    /// stochastic operation". Two different cells get independent streams;
    /// the same cell at the same tick of the same world always gets the
    /// same stream — regardless of allocation order, regardless of whether
    /// other cells came or went between ticks.
    #[must_use]
    pub const fn for_cell_at_tick(world_seed: u64, tick: u64, coord: Coord) -> Self {
        // Written as a chain of let-bindings (no `let mut`) because mutable
        // const-fn locals stabilized only in 1.83 and workspace MSRV is 1.78.
        let h0 = splitmix64(world_seed);
        let h1 = splitmix64(h0 ^ tick);
        let h2 = splitmix64(h1 ^ (coord.x as u32 as u64));
        let h3 = splitmix64(h2 ^ (coord.y as u32 as u64));
        let h4 = splitmix64(h3 ^ (coord.z as u32 as u64));
        Self::new(h4)
    }

    /// Advance state and return the next 32-bit pseudo-random integer.
    pub fn next_u32(&mut self) -> u32 {
        let old_state = self.state;
        self.state = old_state.wrapping_mul(Self::MUL).wrapping_add(Self::INC);
        // XSH-RR output: xorshift then rotate
        let xorshifted = (((old_state >> 18) ^ old_state) >> 27) as u32;
        let rot = (old_state >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Pseudo-random `f32` in `[0, 1)`.
    ///
    /// Uses 24 high bits of one `next_u32` output divided by `2^24`. That
    /// matches the precision of an `f32` mantissa exactly — no rounding
    /// quirks at the edges. The cast is intentionally a `u24 → f32`; the
    /// `>> 8` shift above already strips the bits an `f32` cannot hold.
    #[allow(clippy::cast_precision_loss)]
    pub fn next_f32(&mut self) -> f32 {
        let bits = self.next_u32() >> 8;
        (bits as f32) * (1.0 / 16_777_216.0)
    }

    /// Stochastic floor: round `value` down with probability equal to the
    /// fractional part, up otherwise. Preserves the expected value across
    /// many calls — small flow rates (well below 1 unit per tick) still
    /// transmit on average, no "freeze" artifact.
    ///
    /// Returns 0 for non-positive or NaN input. Safe `as u32` cast already
    /// saturates `f32` overflow at `u32::MAX`, which is the right behavior
    /// — energy per cell is bounded by `E_total` and fits comfortably under
    /// the saturation point in any realistic world.
    pub fn stochastic_floor(&mut self, value: f32) -> u32 {
        // Reject NaN, infinities, zero, and negatives in one go. Written
        // in positive form to satisfy clippy::neg_cmp_op_on_partial_ord.
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        let whole = value.floor();
        let frac = value - whole;
        let r = self.next_f32();
        (whole as u32).saturating_add(u32::from(r < frac))
    }
}

/// `SplitMix64` finalizer, used to derive cell-local seeds from world seeds.
///
/// Public-domain bit-mixer by Vigna; fast, statistically strong, all-`const`.
#[must_use]
const fn splitmix64(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

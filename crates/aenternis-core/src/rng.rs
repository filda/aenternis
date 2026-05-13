//! Deterministic random number generator for the simulation.
//!
//! Aenternis uses **`xorshift32`** with a per-cell seed hash. This
//! produces a bit-for-bit reproducible per-cell stream across hosts and
//! Rust compiler revisions, and it sidesteps the precision quirks that
//! come with the more conventional PCG generator (32-bit state, integer-
//! only state advance, no modular `splitmix64` chain).
//!
//! Two layers of API:
//!
//! - [`Rng`] is the streaming generator itself — wrap a `u32` seed and
//!   pull `u32` / `f64` outputs via [`Rng::next_u32`] /
//!   [`Rng::next_f64`].
//! - [`Rng::for_cell_at_tick`] derives a per-`(world_seed, tick, coord,
//!   domain)` stream so that two different cells get independent
//!   randomness, and the same cell at the same tick of the same world
//!   always sees the same stream — regardless of allocation history. The
//!   `domain` salt lets independent stochastic operations within one
//!   tick draw from independent streams without mutual interference;
//!   `domain == 0` is the default and is bit-identical to the
//!   pre-domain hash output.
//!
//! ## Frozen reference stream
//!
//! The xorshift32 outputs (and the per-cell hash they sit on) are
//! treated as a **frozen reference stream**: any change to the mixer
//! or to `cell_tick_seed`'s constants reseeds every existing world.
//! Bumping the algorithm therefore requires a new seed namespace, not
//! a silent change. `tests/rng.rs` pins the first few outputs against
//! a recorded reference array; that test is the load-bearing invariant.
//!
//! The lineage trail: the algorithm originated as a port of the JS
//! laboratory prototype 9-B's `makeRng` / `cellSeed` (see
//! `prototypes/09-sparse-world/world.js`). Bit-parity with that JS
//! reference was a working contract during the port and is now
//! released — the Rust core may diverge as future work demands. The
//! frozen-reference clause above is what keeps streams stable now.

use crate::Coord;

/// Pseudo-random number generator. 32-bit state, advanced once per
/// output via the `xorshift32` mixer (`s ^= s << 13; s ^= s >> 17;
/// s ^= s << 5`). Cheap to clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    /// 32-bit xorshift state. Always non-zero — xorshift cannot escape
    /// the all-zeros state.
    state: u32,
}

impl Rng {
    /// Build a new generator from a 32-bit seed. `seed == 0` is forced
    /// to `1` because xorshift cannot leave the all-zeros state.
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        let state = if seed == 0 { 1 } else { seed };
        Self { state }
    }

    /// Build a per-cell-per-tick generator deterministic in
    /// `(world_seed, tick, coord, domain)`.
    ///
    /// The intended use is "call once at the start of every per-cell
    /// stochastic operation". Two different cells get independent
    /// streams; the same cell at the same tick of the same world always
    /// sees the same stream — regardless of allocation order, regardless
    /// of whether other cells came or went between ticks.
    ///
    /// `domain` separates independent stochastic operations within the
    /// same `(world_seed, tick, coord)` context: `domain == 0` is the
    /// default stream (used by [`crate::tick::compute_natural_rates`]),
    /// and other values produce uncorrelated streams for callers that
    /// need their own draws inside the same per-cell-per-tick scope.
    /// `domain == 0` is bit-identical to the pre-domain hash output, so
    /// existing callsites' streams do not shift.
    #[must_use]
    pub const fn for_cell_at_tick(world_seed: u64, tick: u64, coord: Coord, domain: u32) -> Self {
        Self::new(cell_tick_seed(world_seed, tick, coord, domain))
    }

    /// Advance state and return the next 32-bit pseudo-random integer.
    pub const fn next_u32(&mut self) -> u32 {
        // Classic Marsaglia xorshift32 (`13, 17, 5` shift triple): state
        // is updated in place and the new state is returned. Frozen
        // reference stream — see module docs.
        let mut s = self.state;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.state = s;
        s
    }

    /// Pseudo-random `f64` in `[0, 1)` using all 32 bits of one
    /// `next_u32` output divided by `2^32`. Both bits and divisor are
    /// exactly representable in `f64`, so the result preserves full RNG
    /// entropy without any rounding loss.
    pub fn next_f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }

    /// Stochastic floor: round `value` down with probability equal to
    /// the fractional part, up otherwise. Preserves the expected value
    /// across many calls — small flow rates (well below 1 unit per
    /// tick) still transmit on average, no "freeze" artifact.
    ///
    /// Returns 0 for non-positive or NaN input. The integer part is
    /// bounded by world energy (well under `2^24`) in any realistic
    /// simulation, so the trailing `as u32` cast is lossless. The
    /// comparison runs in `f64` end-to-end — frozen choice that keeps
    /// the per-cell stochastic rounding stream stable across hosts.
    pub fn stochastic_floor(&mut self, value: f64) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        let whole = value.floor();
        let frac = value - whole;
        let r = self.next_f64();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole_u32 = whole as u32;
        whole_u32.saturating_add(u32::from(r < frac))
    }
}

/// Per-cell `u32` seed deterministic in `(world_seed, coord)`.
///
/// Three rounds of multiply-xor mixing over the coordinate axes; the
/// multipliers are large primes lifted from the lineage prototype
/// (`world.js`) and kept frozen to preserve stream stability across
/// hosts and revisions.
///
/// `world_seed` is truncated to its low 32 bits before hashing — the
/// frozen hash only ever consumes 32 bits of entropy from the seed.
#[must_use]
pub const fn cell_seed(world_seed: u64, coord: Coord) -> u32 {
    let mut h = (world_seed as u32) ^ 0x9E37_79B9;
    h = h.wrapping_add(coord.x as u32).wrapping_mul(374_761_393);
    h ^= h >> 13;
    h = h.wrapping_add(coord.y as u32).wrapping_mul(668_265_263);
    h ^= h >> 16;
    h = h.wrapping_add(coord.z as u32).wrapping_mul(1_274_126_177);
    h ^= h >> 13;
    if h == 0 {
        1
    } else {
        h
    }
}

/// Per-cell-per-tick `u32` seed: `(world_seed, coord, tick, domain) → u32`,
/// with `domain` salting independent per-cell stochastic operations
/// within one tick.
///
/// Combines the per-cell seed with the tick number through one more
/// multiply-mix pass; the output is the seed handed to xorshift32 for
/// the per-cell-tick stream.
///
/// `domain == 0` skips the salt mix entirely, so the output is bit-
/// equal to the pre-domain hash — that path is the frozen reference
/// stream and must not shift. Non-zero `domain` runs an extra multiply-
/// xor pass that diffuses the salt across all 32 bits of the seed so
/// that two domains' streams are well separated.
#[must_use]
pub const fn cell_tick_seed(world_seed: u64, tick: u64, coord: Coord, domain: u32) -> u32 {
    let base = cell_seed(world_seed, coord);
    let mut h = base.wrapping_add(tick as u32).wrapping_mul(2_246_822_507);
    h ^= h >> 16;
    if domain != 0 {
        h = h.wrapping_add(domain).wrapping_mul(1_597_334_677);
        h ^= h >> 15;
    }
    if h == 0 {
        1
    } else {
        h
    }
}

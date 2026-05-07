//! Deterministic random number generator for the simulation.
//!
//! Aenternis uses **`xorshift32`** with the per-cell seed hash ported
//! from JS prototype 9-B. This produces a bit-for-bit reproducible
//! per-cell stream so the Rust core stays in lockstep with the JS
//! prototype's output, and it sidesteps the precision quirks that come
//! with the more conventional PCG generator (32-bit state, integer-only
//! state advance, no modular `splitmix64` chain).
//!
//! Two layers of API:
//!
//! - [`Rng`] is the streaming generator itself — wrap a `u32` seed and
//!   pull `u32` / `f64` outputs via [`Rng::next_u32`] /
//!   [`Rng::next_f64`].
//! - [`Rng::for_cell_at_tick`] derives a per-`(world_seed, tick, coord)`
//!   stream so that two different cells get independent randomness, and
//!   the same cell at the same tick of the same world always sees the
//!   same stream — regardless of allocation history.
//!
//! Reference: the JS port of cellSeed/cellTickSeed lives in
//! `prototypes/09-sparse-world/world.js`. Output bytes match.
//!
//! ## Why xorshift32
//!
//! Earlier revisions kept a PCG-XSH-RR-64/32 backend alongside this one
//! so the Aenternis-native and JS-9B-compat paths could be compared
//! head-to-head; the comparison work is done and we always run in 9-B
//! parity now, so PCG was deleted along with its `splitmix64` keying
//! chain.

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
    /// Build a new generator from a 32-bit seed. Matches JS `makeRng(seed)`
    /// from prototype 9-B's `world.js`. `seed == 0` is forced to `1` because
    /// xorshift cannot leave the all-zeros state.
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        let state = if seed == 0 { 1 } else { seed };
        Self { state }
    }

    /// Build a per-cell-per-tick generator deterministic in
    /// `(world_seed, tick, coord)`.
    ///
    /// The intended use is "call once at the start of every per-cell
    /// stochastic operation". Two different cells get independent
    /// streams; the same cell at the same tick of the same world always
    /// sees the same stream — regardless of allocation order, regardless
    /// of whether other cells came or went between ticks.
    #[must_use]
    pub const fn for_cell_at_tick(world_seed: u64, tick: u64, coord: Coord) -> Self {
        Self::new(cell_tick_seed(world_seed, tick, coord))
    }

    /// Advance state and return the next 32-bit pseudo-random integer.
    pub const fn next_u32(&mut self) -> u32 {
        // Match JS prototype 9-B's xorshift32 exactly: state is updated
        // in place and the new state is returned.
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
    /// entropy without any rounding loss — matches JS prototype 9-B's
    /// `rngFloat = rng() / 0x100000000` to bit precision.
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
    /// comparison runs in `f64` end-to-end to match JS prototype 9-B's
    /// `Number`-native arithmetic.
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

/// JS `cellSeed(worldSeed, x, y, z)` ported verbatim. Used to produce
/// a per-cell `u32` seed deterministic in `(world_seed, coord)`. Output
/// matches the JS stream exactly.
///
/// `world_seed` is truncated to its low 32 bits before hashing (the JS
/// implementation only ever sees 32 bits — it casts via `>>>` everywhere).
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

/// JS `cellTickSeed` ported verbatim — `(world_seed, coord, tick) → u32`.
///
/// Combines the per-cell seed with the tick number through one more
/// multiply-mix pass; the output is the seed handed to xorshift32 for
/// the per-cell-tick stream.
#[must_use]
pub const fn cell_tick_seed(world_seed: u64, tick: u64, coord: Coord) -> u32 {
    let base = cell_seed(world_seed, coord);
    let mut h = base.wrapping_add(tick as u32).wrapping_mul(2_246_822_507);
    h ^= h >> 16;
    if h == 0 {
        1
    } else {
        h
    }
}

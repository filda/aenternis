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

/// Which RNG backend to use.
///
/// Aenternis was originally specified with PCG (cleaner statistics, longer
/// period) and the JS prototype 9-B used `xorshift32` (smaller, simpler).
/// To compare prototype output bit-for-bit against the Rust implementation
/// we need both available; this enum picks one at world construction time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RngKind {
    /// PCG-XSH-RR-64/32 — Aenternis default. Statistically clean, 64-bit
    /// state, splittable per `(seed, tick, coord)` via `splitmix64`.
    #[default]
    Pcg,
    /// `xorshift32` matching JS prototype 9-B exactly. 32-bit state,
    /// per-cell streams keyed via the JS `cellSeed` / `cellTickSeed` hash
    /// chain (multipliers 374761393, 668265263, 1274126177, 2246822507).
    Xorshift32,
}

/// Pseudo-random number generator. Two backends share the same API.
///
/// Cheap to clone — both backends fit in 8 bytes of state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rng {
    /// PCG-XSH-RR-64/32 with 64-bit state.
    Pcg {
        /// 64-bit LCG state, advanced once per output via PCG step.
        state: u64,
    },
    /// `xorshift32` with 32-bit state. Initial state is forced non-zero
    /// (xorshift cannot escape from `0`).
    Xorshift32 {
        /// 32-bit xorshift state, advanced once per output.
        state: u32,
    },
}

impl Rng {
    /// Multiplier from the original PCG paper.
    const PCG_MUL: u64 = 6_364_136_223_846_793_005;

    /// Increment from the original PCG paper (default stream).
    const PCG_INC: u64 = 1_442_695_040_888_963_407;

    /// Build a PCG generator from a 64-bit seed.
    ///
    /// The seed is mixed once before any output is produced, so even adjacent
    /// seeds (`0`, `1`, `2`, …) yield uncorrelated streams.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        // Initial state = seed advanced one PCG step from zero.
        let state = seed.wrapping_add(Self::PCG_INC);
        let state = state
            .wrapping_mul(Self::PCG_MUL)
            .wrapping_add(Self::PCG_INC);
        Self::Pcg { state }
    }

    /// Build an `xorshift32` generator from a 32-bit seed. Matches JS
    /// `makeRng(seed)` from prototype 9-B's `world.js`. `seed == 0` is
    /// forced to `1` because xorshift cannot leave the all-zeros state.
    #[must_use]
    pub const fn new_xs32(seed: u32) -> Self {
        let state = if seed == 0 { 1 } else { seed };
        Self::Xorshift32 { state }
    }

    /// Build a PCG generator deterministically derived from a world seed.
    ///
    /// Use for world-level operations that are not associated with any cell
    /// — for example generating the big bang program at world bootstrap.
    #[must_use]
    pub const fn for_world(world_seed: u64) -> Self {
        Self::new(splitmix64(world_seed))
    }

    /// Build a PCG generator deterministic in `(world_seed, tick, coord)`.
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

    /// Build a per-cell-per-tick generator using the requested backend.
    ///
    /// `Pcg` path: identical to [`Self::for_cell_at_tick`] (splitmix64 chain
    /// → PCG init).
    ///
    /// `Xorshift32` path: matches JS prototype 9-B's `cellTickSeed` hash
    /// (custom u32-arithmetic chain with prime multipliers) → `xorshift32`
    /// init. Bit-for-bit reproduces the JS stream for the same inputs.
    #[must_use]
    pub const fn for_cell_at_tick_with_kind(
        kind: RngKind,
        world_seed: u64,
        tick: u64,
        coord: Coord,
    ) -> Self {
        match kind {
            RngKind::Pcg => Self::for_cell_at_tick(world_seed, tick, coord),
            RngKind::Xorshift32 => Self::new_xs32(cell_tick_seed_xs32(world_seed, tick, coord)),
        }
    }

    /// Advance state and return the next 32-bit pseudo-random integer.
    pub fn next_u32(&mut self) -> u32 {
        match self {
            Self::Pcg { state } => {
                let old_state = *state;
                *state = old_state
                    .wrapping_mul(Self::PCG_MUL)
                    .wrapping_add(Self::PCG_INC);
                // XSH-RR output: xorshift then rotate
                let xorshifted = (((old_state >> 18) ^ old_state) >> 27) as u32;
                let rot = (old_state >> 59) as u32;
                xorshifted.rotate_right(rot)
            }
            Self::Xorshift32 { state } => {
                // Match JS prototype 9-B's xorshift32 exactly: state is
                // updated in place and the new state is returned.
                let mut s = *state;
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                *state = s;
                s
            }
        }
    }

    /// Pseudo-random `f32` in `[0, 1)`.
    ///
    /// Backend-dependent precision:
    ///
    /// - `Pcg`: 24 high bits / `2^24` (matches `f32` mantissa exactly).
    /// - `Xorshift32`: full 32 bits / `2^32` via `f64` intermediate (matches
    ///   JS `rngFloat = rng() / 0x100000000` to bit precision).
    ///
    /// The two paths produce different distributions at the very tail —
    /// tied to the bit-identity contract with prototype 9-B.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn next_f32(&mut self) -> f32 {
        match self {
            Self::Pcg { .. } => {
                let bits = self.next_u32() >> 8;
                (bits as f32) * (1.0 / 16_777_216.0)
            }
            Self::Xorshift32 { .. } => {
                let bits = self.next_u32();
                (f64::from(bits) / 4_294_967_296.0) as f32
            }
        }
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

/// JS `cellSeed(worldSeed, x, y, z)` ported verbatim. Used by the
/// `Xorshift32` backend to produce a per-cell u32 seed deterministic in
/// `(world_seed, coord)`. Output matches the JS stream exactly.
///
/// `world_seed` is truncated to its low 32 bits before hashing (the JS
/// implementation only ever sees 32 bits — it casts via `>>>` everywhere).
#[must_use]
pub const fn cell_seed_xs32(world_seed: u64, coord: Coord) -> u32 {
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
/// multiply-mix pass; the output is the seed handed to `xorshift32` for
/// the per-cell-tick stream.
#[must_use]
pub const fn cell_tick_seed_xs32(world_seed: u64, tick: u64, coord: Coord) -> u32 {
    let base = cell_seed_xs32(world_seed, coord);
    let mut h = base.wrapping_add(tick as u32).wrapping_mul(2_246_822_507);
    h ^= h >> 16;
    if h == 0 {
        1
    } else {
        h
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

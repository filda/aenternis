//! Reference state hashes — bit-parity baseline for refactors that
//! must preserve `apply_outflow` semantics (e.g. the rope-based merge
//! that replaced the per-insert `Vec::splice`; see
//! `docs/optimalizace-2026-05.md`).
//!
//! Each scenario runs a deterministic `SparseWorld` for a fixed number
//! of ticks. After each tick the full per-cell state is hashed with
//! `FxHasher`, and the resulting sequence is compared against an array
//! of expected hashes frozen in source.
//!
//! ## Updating the expected values
//!
//! When an **intentional** semantic change is made (and the new output
//! is verified by other means), run:
//!
//! ```sh
//! UPDATE_BASELINE=1 cargo test --test apply_outflow_bit_parity -- --nocapture
//! ```
//!
//! The printed hash arrays can be pasted back into this file. Never
//! update the expected values just to make a failing test pass without
//! a clear story about which semantic change drove the drift.

#![allow(clippy::unreadable_literal)]

use std::hash::{Hash, Hasher};

use aenternis_core::{tick, Coord, Rng, SparseWorld};
use rustc_hash::FxHasher;

/// Build a cubic dense grid of `side^3` cells with `cell_energy` slots
/// of RNG-derived memory each. The bit-parity baseline tests use this
/// to construct worlds that immediately exceed the
/// [`crate::parallel::par_or_seq_iter_mut!`] threshold (8 192 cells),
/// so the rayon parallel branch fires from tick 0. Mirrors the helper
/// of the same shape in `benches/tick.rs`.
fn dense_grid_world(seed: u64, side: i32, cell_energy: u32) -> SparseWorld {
    let half = side / 2;
    let mut world = SparseWorld::new(seed);
    let mut rng = Rng::new(seed as u32);
    for x in -half..(side - half) {
        for y in -half..(side - half) {
            for z in -half..(side - half) {
                let mut memory = Vec::with_capacity(cell_energy as usize);
                for _ in 0..cell_energy {
                    memory.push(rng.next_u32());
                }
                world.insert_with_memory(Coord::new(x, y, z), &memory);
            }
        }
    }
    world
}

/// Hash every observable field of every cell, in coordinate-sorted
/// order. Captures everything `apply_outflow` can affect: memory
/// contents, inflow counters, `pc`, `origin_tag`, plus the rest of the
/// per-cell state for completeness.
fn hash_world(w: &SparseWorld) -> u64 {
    let mut cells: Vec<_> = w.iter().collect();
    cells.sort_by_key(|(c, _)| (c.x, c.y, c.z));

    let mut h = FxHasher::default();
    w.tick.hash(&mut h);
    w.total_energy().hash(&mut h);
    cells.len().hash(&mut h);
    for (coord, cell) in &cells {
        coord.x.hash(&mut h);
        coord.y.hash(&mut h);
        coord.z.hash(&mut h);
        cell.pc.hash(&mut h);
        cell.origin_tag.hash(&mut h);
        cell.appearance.hash(&mut h);
        cell.memory(w.arena()).hash(&mut h);
        cell.rates.hash(&mut h);
        cell.active_outflow.hash(&mut h);
        cell.pointers.hash(&mut h);
        cell.pointer_override.hash(&mut h);
        cell.inflow.hash(&mut h);
    }
    h.finish()
}

/// Run `ticks` of `tick::step` on `world`, hashing state after each tick.
/// If `UPDATE_BASELINE=1` is set in the environment, print the captured
/// hashes in a paste-friendly format and skip the assertion.
fn run_scenario(world: &mut SparseWorld, coeff: f64, k: u32, ticks: usize, expected: &[u64]) {
    assert_eq!(
        expected.len(),
        ticks,
        "expected hash array length must match tick count",
    );

    let mut actual = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        tick::step(world, coeff, k);
        actual.push(hash_world(world));
    }

    if std::env::var("UPDATE_BASELINE").is_ok() {
        eprintln!("captured baseline hashes:");
        eprintln!("&[");
        for h in &actual {
            eprintln!("    0x{h:016x},");
        }
        eprintln!("]");
        return;
    }

    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            a,
            e,
            "state hash drift at tick {} — bit-parity break in apply_outflow / tick::step",
            i + 1,
        );
    }
}

/// Sparse big-bang from origin. Exercises the expansion regime where
/// cells alloc-on-write into void neighbors and memory grows over time.
/// 50 ticks at 1k energy.
#[test]
fn bit_parity_sparse_big_bang() {
    let mut w = SparseWorld::big_bang(42, 1_000);
    let expected: &[u64] = &[
        0x6dc22ccea9784430,
        0xa49ac5944383b0ca,
        0x02d6a23d316dd9ac,
        0x00089b20b0e1594b,
        0x354b74f2450784db,
        0x4436e11e83c358cb,
        0x965b630bb7419024,
        0xb345bc3c0f5175d1,
        0x6693ab688371c871,
        0x50a03b41de1440c5,
        0x595c00cadbaf89a7,
        0x9a2dcdd88ac8fdc1,
        0x2e3fe1396e79b56c,
        0xfbeb465aa8b24596,
        0x5fa70edee2f4c078,
        0xe8b44e38f30ddc42,
        0x047cd280ee8da9f0,
        0x6c1fe0268f65dfa8,
        0x47e3f6d4c4ca12f3,
        0xa76235ff0bed24c1,
        0x4ba67250e3b72be9,
        0xbfeac2c2c3fa0751,
        0x12a9f47b7e30beeb,
        0x70362f97e2849d3e,
        0xfbda974f620cdd42,
        0xd8dd1e6fdace0677,
        0x4d1f12c9da8cedbf,
        0x6f24e07cc6193258,
        0x64706d4308e7f21d,
        0x17acfae9bc7d39ab,
        0x7a88119d9e328b5f,
        0x4ea51fd744fb7dab,
        0xa127524e7f0f68a1,
        0xe7e31a0cbcb2d75e,
        0xc483dd2a64f5259a,
        0x1ddc3ed17889c04f,
        0x046acd5e083faf83,
        0x8e5adf5680dd8b4d,
        0x5fac112004d76a9e,
        0xd7a7fc63425d38fc,
        0x51f1d5160ceeed48,
        0x9e10725a74e05d4e,
        0xaa5cf879e2135913,
        0x80d48fbcafa205fd,
        0x58289de5fdcd7ff5,
        0xaee5198f06a157b7,
        0xffb49c9de9f567e8,
        0xfe97f168043bf6a5,
        0xcb0aa2784956355e,
        0x708a9d4a4aa02f64,
    ];
    run_scenario(&mut w, 0.20, 1, 50, expected);
}

/// Aggressive dominance / intrusion: `move_threshold = 0.5` makes every
/// inflow with `dominance >= 0.5` body-snatch the target. Exercises the
/// splice-at-`write_start` path with high `intrusion_depth` values.
/// 40 ticks at 5k energy.
#[test]
fn bit_parity_aggressive_dominance() {
    let mut w = SparseWorld::big_bang(99, 5_000);
    w.move_threshold = 0.5;
    let expected: &[u64] = &[
        0xd12496d0b4c8cf10,
        0xd80bc10a7831fecb,
        0x17eab7d1e62b4fd4,
        0x2063596e4454d5e7,
        0xcc9ead69c7513aef,
        0x195ec77a7e46381e,
        0x34409c7821b4d96e,
        0xc6f77e63ac4cd9b7,
        0xcafef1d281864399,
        0xd8fcb54f2005bfcd,
        0x74660c67ab60a2c9,
        0xd0d26d9bf74728b7,
        0x8d3eb8511b6edd98,
        0x71720f3af2365ad4,
        0x8e14acd605b82b9a,
        0xdec52a4e1e4a4751,
        0x060dc8faff55ae0e,
        0x264412043e6643f7,
        0x7f97d4ca85562a67,
        0x491d01de8dca55e7,
        0xb03d9e5ea0fb4430,
        0xb4bbeb5f27247e26,
        0x0e5a105dd0cc979f,
        0xfb8a694a76da32ea,
        0xd9cbba8a964f45bf,
        0x83f93fc635c6d0dd,
        0xf5815758dc17d937,
        0xf99f37ca336cf84b,
        0x041a44173388c605,
        0x2efe6d39c80a6354,
        0x7719b53cb36ecf48,
        0xe205f2a857ab73db,
        0x80ddcd42fc0cec70,
        0x7f9781d253eeb0ff,
        0x829c4862b3b55bf6,
        0xf980eb30d674397f,
        0xae00001b9ec838e9,
        0xd12ee8dfb89da703,
        0x6443c3e19646235d,
        0x602099a24c6dc39f,
    ];
    run_scenario(&mut w, 0.25, 1, 40, expected);
}

/// CPU-active scenario: self-xp-replicator program injected at origin.
/// Programs accumulate `active_outflow` via `port`, which feeds the
/// sub-tick reflow and the `apply_outflow` splice with non-trivial slot
/// counts. 30 ticks at 65k energy.
#[test]
fn bit_parity_with_injected_program() {
    // setp xp, 0; jmp 0 — same program as `dump_state_for_diff`.
    let program: [u32; 5] = [0x09, 0, 0, 0x07, 0];
    let mut w = SparseWorld::big_bang_with_program(1234, 65_536, &program);
    w.move_threshold = 1.0;
    let expected: &[u64] = &[
        0x7c52f4626ea419d3,
        0x78c0bd8a8d1b4830,
        0xb422486b04c0ad30,
        0x2ed1b4b9bd1ecef9,
        0xb70a626efefe8f0a,
        0xc83832e1731e29e1,
        0xf3b6bfe225da38df,
        0x26023a667c1f3c53,
        0x7ad5cd4195e60aaa,
        0x062dcde49cdfefc2,
        0x855e9eaf4b79f1be,
        0xe4c96c5ee10144b0,
        0xf67eeb8beeaf5962,
        0x20f120f7bdf3d6d5,
        0xbdfc09da011ed160,
        0xfc27c503b139fde3,
        0x1e5917669c97a092,
        0xef2f68b0b9134272,
        0xa63eec60aa0a8d32,
        0x371f6fcc21a285e3,
        0x31cfb1bf5e44758b,
        0xd9a63cd3e37fe905,
        0xd607602ae889b016,
        0x90d0439ba8550715,
        0x754c0671ccef1749,
        0x255706e951dd3fa6,
        0x25dd8c6ffa142479,
        0x08211246e26d61e5,
        0x715887a5e9c921f4,
        0xa95d274d644dbc9c,
    ];
    run_scenario(&mut w, 0.15, 1, 30, expected);
}

/// Rayon parallel-path coverage. Builds a 22³ = 10 648 dense grid of
/// cells, which immediately exceeds the
/// [`crate::parallel::par_or_seq_iter_mut!`] threshold (8 192 cells),
/// so every per-tick phase runs through `par_iter_mut().for_each(...)`
/// rather than the sequential fallback from tick 0. Pinning hashes
/// here guards multi-thread determinism: if a future refactor
/// accidentally introduces a read/write race across cells under rayon
/// work-stealing, this drifts deterministically across runs.
///
/// Native rayon and the `wasm-threads` WASM bundle share the same
/// per-cell-independent body, so passing here also covers the browser
/// threaded path by inheritance — the determinism argument doesn't
/// depend on which backend dispatches the threads.
#[test]
fn bit_parity_rayon_parallel_path() {
    let mut w = dense_grid_world(7, 22, 32);
    assert!(
        w.len() >= 8_192,
        "test setup must exceed par_or_seq_iter_mut threshold to exercise the rayon parallel path; got {} cells",
        w.len(),
    );
    let expected: &[u64] = &[
        0x161155ae7d07a1f2,
        0xcf02982830e33a94,
        0x4d6f48a2ff0c03d7,
        0x605dddc5410bfa1f,
        0x8616f2a0f33e8a4f,
        0xaed11f00f1eec711,
        0xa594c8fb37acc594,
        0xced1de8c498c2f3b,
        0x67dbaad43d12d1c6,
        0x09d8761bbe2589ee,
    ];
    run_scenario(&mut w, 0.20, 1, 10, expected);
}

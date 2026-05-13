//! Reference state hashes — bit-parity baseline for refactors that
//! must preserve `apply_outflow` semantics (e.g.
//! `docs/plan-apply-outflow-splice.md`, which replaces the per-insert
//! `Vec::splice` with a rope-based merge).
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

use aenternis_core::{tick, Cell, Coord, Rng, SparseWorld};
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
                world.insert(Coord::new(x, y, z), Cell::with_memory(memory));
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
        cell.memory.hash(&mut h);
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
        0x7690d7325ed1e325,
        0x55e743cfb34b575b,
        0xaf61ad43bd334ee1,
        0x6e9c47ced024d442,
        0xd8e220d4b9f9109f,
        0x04b6690f89c8aa30,
        0x1c4a4b49443fdf08,
        0x7b7e2d6841c97a17,
        0x36db0ded8d042675,
        0x9ea1a3338d622961,
        0x5f91d844b46dc3ca,
        0x2db8610ca79c45fc,
        0x4bd7fef4b7820aa2,
        0x6ef316b3591a4195,
        0x767e1ef1a11e44bc,
        0xd8eea5fdc586db9c,
        0x591873ee0806194d,
        0xb4727d20f71c1318,
        0x07c58cb83d8b4ee9,
        0x9e06a58fbfab29bc,
        0x42fc189ca2bfd235,
        0x89642d57a2a40923,
        0x0051a3df1e7a31ad,
        0x4a5ef2e4b2bdec09,
        0x35974238b3ee9296,
        0xa937626e80c09c23,
        0x2891d80f36efcf97,
        0x87be57a02f481f74,
        0x4f4fcd48a2c493c1,
        0x3e2d7cd7eb2f15b4,
        0x1d501671cc02ef47,
        0x1a45acb55c5b5476,
        0x184b922beb52031d,
        0xc74462b53d2b3feb,
        0x5ad696c98b8f086b,
        0x0df4986dedc3989b,
        0xa16a69945202a191,
        0xb2189fa8b66e8e28,
        0x981ee46a337149e4,
        0x652237600f226766,
        0x8788cbd2aa30a974,
        0xf03e945acfa54bc4,
        0x71f39bf74ea0c98b,
        0x16c3f2602b82f254,
        0x89ae14ae3c697913,
        0xb7e7bd902edf8342,
        0x3a75359e9da86721,
        0x18e16cf4e0ae1da5,
        0x7e9085552a9bc624,
        0x47d2d7bb2e230d69,
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
        0x60517229541459aa,
        0xbfb8c9bb2be8a462,
        0xeba0ab6c10003f9b,
        0xdc3036b7362f0237,
        0x5485f9a3ba7f68c7,
        0xc977132d8d108b25,
        0x870a66e8b1138cb7,
        0xc5abcc65082b3bdd,
        0xe9c026a53db365ee,
        0x1e04845efb295c69,
        0xa9fd46f615758346,
        0xdb6f4bbcbdb75ba6,
        0xfc54e4c8c86c3c38,
        0xecb7c02386d6bd9f,
        0xfa40d47688740f31,
        0x645f260f95973029,
        0x69988ba87e508796,
        0x71f78a9cda1a2f9d,
        0x6f7afd394997bfdb,
        0xd46e9e56f41e988d,
        0xde403cad53d3311f,
        0x9cc77c8ab89e6592,
        0x766b325d06f8a233,
        0x82c15596bcd0945b,
        0x30c149757d2c2b2c,
        0xe71ee7a800a60509,
        0x0380b836d0868ccb,
        0x518cf40f43230f3c,
        0x2b6502179b5ee5d5,
        0x3e1b8921b0a0657e,
        0xf0e1e79d603340dc,
        0x834c1f2d70ebb6d0,
        0x09b17cdbbc179f7f,
        0xd9543c83f66784d1,
        0x0e245719a36db12d,
        0x7d8274b5b3178e17,
        0xf078b707d1df867e,
        0x93adc6e3ae41e4a4,
        0xf51b018501139671,
        0x6212fb4bbf5dc47a,
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
        0x1c2b2f68fb3abcd8,
        0x8319c670dcf4bfc8,
        0x325871d658c73599,
        0x9592693f42b08942,
        0xe44618a3756e1e77,
        0x9c9edbc6229a4e53,
        0xec24dae777fa00fc,
        0x6b3415ff96faeb57,
        0x88c49f95ddc73dcb,
        0x10912521f38a4203,
        0xa497715ee6fc2141,
        0xd5002dba91c8b1d0,
        0x85bd282b4fa59fb2,
        0xae506c57268366e3,
        0x0d4c1b15f1286cfc,
        0x413c91ef4cd3038e,
        0x4c6f2cfa5fc5c780,
        0x06e97573bede7daf,
        0xffffa7813d3e1f52,
        0xdc91f1d6a4eb4c9a,
        0x447028cd7d4a36d5,
        0x2a32bd6ea6b640bb,
        0x0b7fdd57fcf139b4,
        0x940e050ecc7367c2,
        0x4e518a0482b94621,
        0xe227e17c3cb05514,
        0x3b1b6fb62eb4b519,
        0xf017bd879dbaf763,
        0xbb31c62273420ecc,
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
        0x6012682b0dc3752d,
        0xe3662713753ba1a5,
        0x6d46b76b7ebe7e08,
        0x24368835073efed4,
        0x7fb2af5576b89afc,
        0x64ca1f0c62e83aba,
        0xac4dcb6384129105,
        0x37b56b6f4c1d9aeb,
        0x8904a45e111a7b48,
        0x96f3141c20103eb5,
    ];
    run_scenario(&mut w, 0.20, 1, 10, expected);
}

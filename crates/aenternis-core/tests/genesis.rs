//! Integration tests for procedural genesis and the `(base, overlay)`
//! big-bang API. See `docs/genesis-plan.md`.

use aenternis_core::rng::cell_seed;
use aenternis_core::{Base, Coord, GenesisConfig, Rng, SparseWorld};

const SEED: u64 = 0xA1A2_A3A4;
const ENERGY: u32 = 4096;

fn origin_memory(world: &SparseWorld) -> Vec<u32> {
    world
        .cell_memory(Coord::ORIGIN)
        .expect("origin cell exists")
        .to_vec()
}

// --- Macro genesis ---------------------------------------------------------

#[test]
fn macro_genesis_is_deterministic() {
    let a = SparseWorld::big_bang_macros(SEED, ENERGY);
    let b = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_eq!(origin_memory(&a), origin_memory(&b));
}

#[test]
fn macro_genesis_differs_by_seed() {
    let a = SparseWorld::big_bang_macros(SEED, ENERGY);
    let b = SparseWorld::big_bang_macros(SEED ^ 0xFFFF, ENERGY);
    assert_ne!(origin_memory(&a), origin_memory(&b));
}

#[test]
fn macro_genesis_fills_whole_memory_and_conserves_energy() {
    let w = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_eq!(w.len(), 1, "exactly one origin cell");
    assert_eq!(w.total_energy(), u64::from(ENERGY));
    assert_eq!(origin_memory(&w).len(), ENERGY as usize);
}

#[test]
fn macro_genesis_is_not_pure_noise() {
    // The generated program must differ from the raw noise fill for the
    // same seed — otherwise the generator isn't doing anything.
    let macros = SparseWorld::big_bang_macros(SEED, ENERGY);
    let noise = SparseWorld::big_bang(SEED, ENERGY);
    assert_ne!(origin_memory(&macros), origin_memory(&noise));
}

#[test]
fn big_bang_with_config_default_matches_macros() {
    // The default config must reproduce the plain macro genesis, proving
    // `big_bang_with` and `big_bang_with_config` share one code path.
    let cfg = GenesisConfig::default();
    let cfgd = SparseWorld::big_bang_with_config(SEED, ENERGY, Base::Macros, &[], &cfg);
    let plain = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_eq!(origin_memory(&cfgd), origin_memory(&plain));
}

#[test]
fn big_bang_with_config_window_threads_through() {
    // A non-default window must reshape the program — guards against the
    // generator silently ignoring the passed config.
    let narrow = GenesisConfig {
        window: 8,
        fertility: 1.0,
    };
    let a = SparseWorld::big_bang_with_config(SEED, ENERGY, Base::Macros, &[], &narrow);
    let b = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_ne!(origin_memory(&a), origin_memory(&b));
}

#[test]
fn big_bang_with_config_fertility_threads_through() {
    let barren = GenesisConfig {
        window: 256,
        fertility: 0.0,
    };
    let a = SparseWorld::big_bang_with_config(SEED, ENERGY, Base::Macros, &[], &barren);
    let b = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_ne!(origin_memory(&a), origin_memory(&b));
}

// --- Overlay (player program) ----------------------------------------------

#[test]
fn overlay_prefix_is_written_verbatim() {
    let prefix = [0xDEAD_BEEF, 0x0000_002A, 0xFFFF_0000];
    let w = SparseWorld::big_bang_with(SEED, ENERGY, Base::Macros, &prefix);
    let mem = origin_memory(&w);
    assert_eq!(&mem[..prefix.len()], &prefix);
}

#[test]
fn overlay_tail_is_independent_of_prefix() {
    // The base is generated for the whole memory and the prefix overlays
    // it, so the tail must be identical to the no-overlay genesis.
    let prefix = [1u32, 2, 3, 4, 5];
    let with = SparseWorld::big_bang_with(SEED, ENERGY, Base::Macros, &prefix);
    let without = SparseWorld::big_bang_macros(SEED, ENERGY);
    assert_eq!(
        &origin_memory(&with)[prefix.len()..],
        &origin_memory(&without)[prefix.len()..],
        "macro tail must not depend on the overlay"
    );
}

// --- Back-compat: Base::Noise reproduces the legacy fill -------------------

#[test]
fn noise_base_matches_raw_xorshift_stream() {
    let w = SparseWorld::big_bang(SEED, ENERGY);
    let mem = origin_memory(&w);
    let mut rng = Rng::new(cell_seed(SEED, Coord::ORIGIN));
    for (i, slot) in mem.iter().enumerate() {
        assert_eq!(*slot, rng.next_u32(), "noise slot {i} mismatch");
    }
}

#[test]
fn noise_with_program_keeps_prefix_then_noise() {
    let prefix = [0xAAAA_AAAA, 0xBBBB_BBBB];
    let w = SparseWorld::big_bang_with_program(SEED, ENERGY, &prefix);
    let mem = origin_memory(&w);
    assert_eq!(&mem[..2], &prefix);
    // Tail is the fresh xorshift stream starting at slot 2.
    let mut rng = Rng::new(cell_seed(SEED, Coord::ORIGIN));
    for slot in &mem[2..] {
        assert_eq!(*slot, rng.next_u32());
    }
}

#[test]
fn noise_tail_independent_of_prefix_content_same_length() {
    let a = SparseWorld::big_bang_with_program(SEED, ENERGY, &[1, 2, 3]);
    let b = SparseWorld::big_bang_with_program(SEED, ENERGY, &[9, 8, 7]);
    assert_eq!(&origin_memory(&a)[3..], &origin_memory(&b)[3..]);
}

#[test]
fn big_bang_with_program_truncates_oversized_program() {
    let program = [1u32, 2, 3, 4, 5];
    let w = SparseWorld::big_bang_with_program(SEED, 3, &program);
    assert_eq!(origin_memory(&w), &program[..3]);
}

// --- Zero energy edge -------------------------------------------------------

#[test]
fn zero_energy_is_empty_for_every_base() {
    assert!(SparseWorld::big_bang_macros(SEED, 0).is_empty());
    assert!(SparseWorld::big_bang_with(SEED, 0, Base::Macros, &[1, 2, 3]).is_empty());
}

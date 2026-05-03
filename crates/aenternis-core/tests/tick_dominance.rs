//! Integration tests for the dominance / intrusion mechanic in
//! `apply_outflow`.
//!
//! Properties verified:
//!
//! - **Dominance computation** — high-energy attacker against low-energy
//!   target produces dominance close to 1; equal energies produce 0.
//! - **Intrusion depth** — slots inserted at `memSize - dominance *
//!   memSize`. High dominance pushes deep, low dominance stays at the
//!   membrane.
//! - **Origin-tag inheritance** — only fires when top dominance ≥ 0.5.
//! - **Sort order** — strongest inflow goes deepest; tie-break by
//!   canonical direction.
//! - **Conservation** — total slots before == total slots after, just
//!   like the no-dominance variant.

use aenternis_core::tick::{apply_outflow, collect_outflow, Outflow};
use aenternis_core::{Cell, Coord, Direction, SparseWorld};

// ----- helpers ---------------------------------------------------------------

fn one_directional_outflow(slots: Vec<u32>, dir: Direction) -> [Vec<u32>; Direction::COUNT] {
    let mut per_dir: [Vec<u32>; Direction::COUNT] = Default::default();
    per_dir[dir.index()] = slots;
    per_dir
}

// ----- dominance computation -------------------------------------------------

#[test]
fn weak_attacker_against_strong_target_produces_low_dominance() {
    // Attacker: 3 energy, will emit 1 slot → post_burn = 2.
    // Target:  100 energy, no outflow → post_outflow = 100.
    // r = 100 / 2 = 50; dominance = clamp(1 - 50/2.0, 0, 1) = 0.
    // Result: inflow stacks at the membrane (writeStart = memSize), no
    // origin-tag inheritance.
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![10, 20, 30]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA_AAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = Cell::with_memory(vec![99u32; 100]);
    target.origin_tag = 0xBBBB_BBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    // memSize_before_inflow = 100, dominance ≈ 0, write_start = 100,
    // so the inflow slot lands at the very end.
    assert_eq!(target.memory.len(), 101);
    assert_eq!(target.memory[100], 10);
    // Origin tag preserved (dominance < 0.5).
    assert_eq!(target.origin_tag, 0xBBBB_BBBB);
}

#[test]
fn strong_attacker_against_weak_target_produces_full_dominance() {
    // Attacker: 100 energy emitting 1 slot → post_burn ≈ 99.
    // Target: 1 energy, no outflow → post_outflow = 1.
    // r = 1 / 99 ≈ 0.01; dominance = 1 - 0.01/2 ≈ 0.995, clamped to 1.
    // Inflow drives all the way to write_start = 0 (deep core).
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![1; 100]);
    attacker.memory[0] = 0xAAAA;
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA_AAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = Cell::with_memory(vec![0xBBBB]);
    target.origin_tag = 0xBBBB_BBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    // memSize_before_inflow = 1, dominance ≈ 1, write_start = 0.
    // new_mem = [] ++ slots ++ old = [0xAAAA, 0xBBBB]
    assert_eq!(target.memory, vec![0xAAAA, 0xBBBB]);
    // Top dominance ≥ 0.5, target inherits attacker's origin tag.
    assert_eq!(target.origin_tag, 0xAAAA_AAAA);
}

#[test]
fn void_target_gets_full_dominance() {
    // Target doesn't exist (void). target_E_post = 0, attacker_post = N.
    // r = 0; dominance = 1.
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![1, 2, 3, 4, 5]);
    attacker.memory[0] = 0xCAFE;
    attacker.rates[Direction::Xp.index()] = 2;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xDEAD_BEEF;
    w.insert(Coord::ORIGIN, attacker);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w
        .get(Coord::new(1, 0, 0))
        .expect("void target should be alloc-on-written");
    // Empty target, dominance 1, write_start = 0. Memory == slots.
    assert_eq!(target.memory, vec![0xCAFE, 2]);
    // Inherited origin tag.
    assert_eq!(target.origin_tag, 0xDEAD_BEEF);
}

// ----- intrusion depth -------------------------------------------------------

#[test]
fn intrusion_at_intermediate_dominance_inserts_in_the_middle() {
    // Pick energies so the dominance lands near 0.5.
    // Attacker: 6 energy, emits 1 → post_burn = 5.
    // Target: 5 energy, no outflow → post_outflow = 5.
    // r = 5/5 = 1.0; dominance = clamp(1 - 1.0/2.0, 0, 1) = 0.5.
    // memSize_before_inflow = 5, intrusion_depth = floor(0.5 * 5) = 2,
    // write_start = 5 - 2 = 3.
    // new_mem = [t0, t1, t2] ++ [a0] ++ [t3, t4]
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![0xA0, 1, 2, 3, 4, 5]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = Cell::with_memory(vec![0xB0, 0xB1, 0xB2, 0xB3, 0xB4]);
    target.origin_tag = 0xBBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory, vec![0xB0, 0xB1, 0xB2, 0xA0, 0xB3, 0xB4]);
    // dominance ≥ 0.5 (exactly 0.5), origin_tag inherited from attacker
    // (which is 0 by default since we didn't set it).
    assert_eq!(target.origin_tag, 0);
}

// ----- origin-tag inheritance ------------------------------------------------

#[test]
fn origin_tag_preserved_below_threshold() {
    // Construct a 0.4 dominance scenario by tuning energies.
    // r = 1 - 0.4 * move_threshold = 1 - 0.4 * 2 = 0.2 backwards … solve:
    // dominance = 1 - r/2 = 0.4 → r = 1.2 → target_post = 1.2 *
    // attacker_post.
    // Attacker 6 energy, emit 1 → post_burn = 5. Target post = 6.
    // Use target with 6 slots, no outflow.
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![1, 2, 3, 4, 5, 6]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = Cell::with_memory(vec![10, 20, 30, 40, 50, 60]);
    target.origin_tag = 0xBBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    // dominance = 1 - 6/(5*2) = 0.4 < 0.5 → origin tag stays 0xBBBB.
    assert_eq!(w.get(Coord::new(1, 0, 0)).unwrap().origin_tag, 0xBBBB);
}

// ----- sort order ------------------------------------------------------------

#[test]
fn multiple_inflows_apply_strongest_first() {
    // A target receives inflow from both -x (weak attacker) and +x
    // (strong attacker) in the same tick. Strong should sort to the
    // top and write deeper into memory.
    //
    // Setup:
    //   weak attacker at (1, 0, 0): 3 energy, emits 1 toward -x
    //   strong attacker at (-1, 0, 0): 100 energy, emits 1 toward +x
    //   target at origin: 5 slots, no outflow
    //
    // Both write into target. Strong gets dominance ≈ 1, intrusion 5
    // → write_start = 0 → inserted at the front. Weak gets dominance
    // ≈ 0, write_start = memSize (post-strong) → stacked at end.
    //
    // Final memory ordering: [strong] ++ [old_target] ++ [weak]
    let mut w = SparseWorld::new(0);

    let mut weak = Cell::with_memory(vec![0xCC, 0, 0]);
    weak.rates[Direction::Xn.index()] = 1;
    weak.pointers[Direction::Xn.index()] = 0;
    w.insert(Coord::new(1, 0, 0), weak);

    let mut strong = Cell::with_memory(vec![0xAA; 100]);
    strong.memory[0] = 0xAAAA;
    strong.rates[Direction::Xp.index()] = 1;
    strong.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::new(-1, 0, 0), strong);

    let target = Cell::with_memory(vec![0xB0, 0xB1, 0xB2, 0xB3, 0xB4]);
    w.insert(Coord::ORIGIN, target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::ORIGIN).unwrap();
    // Strong inflow (0xAAAA) is at the very front.
    assert_eq!(target.memory[0], 0xAAAA);
    // Weak inflow (0xCC) is at the very end.
    assert_eq!(*target.memory.last().unwrap(), 0xCC);
    // Total memory size = 5 (old) + 1 (strong) + 1 (weak) = 7.
    assert_eq!(target.memory.len(), 7);
}

// ----- conservation ----------------------------------------------------------

#[test]
fn dominance_apply_still_conserves_total_slots() {
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    cell.rates = [1, 1, 1, 1, 1, 1];
    cell.pointers = [0, 1, 2, 3, 4, 5];
    w.insert(Coord::ORIGIN, cell);

    let before = w.total_energy();
    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);
    assert_eq!(w.total_energy(), before, "total slots must be preserved");
}

// ----- raw direct-call helper for the missing-source path --------------------

#[test]
fn outflow_with_missing_source_treats_attacker_pre_as_zero() {
    // Outflow map references a coord that doesn't exist in the world.
    // attacker_pre = 0, attacker_total = N, post_burn = max(1, 0-N) = 1.
    // Target is void → target_post = 0 → r = 0 → dominance = 1.
    // Inflow lands at the void target with full dominance.
    let mut w = SparseWorld::new(0);

    let mut outflow = Outflow::new();
    outflow.insert(
        Coord::ORIGIN,
        one_directional_outflow(vec![42], Direction::Xp),
    );

    apply_outflow(&mut w, &outflow);

    assert!(!w.contains(Coord::ORIGIN));
    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory, vec![42]);
}

// ----- move_threshold knob ---------------------------------------------------

#[test]
fn lowering_move_threshold_raises_dominance() {
    // Same energies, two move_threshold values; the lower threshold
    // gives higher dominance for the same r.
    fn dom_for_threshold(mt: f32) -> usize {
        let mut w = SparseWorld::new(0);
        w.move_threshold = mt;

        let mut attacker = Cell::with_memory(vec![1, 2, 3, 4, 5, 6]);
        attacker.rates[Direction::Xp.index()] = 1;
        attacker.pointers[Direction::Xp.index()] = 0;
        w.insert(Coord::ORIGIN, attacker);

        let target = Cell::with_memory(vec![0; 5]);
        w.insert(Coord::new(1, 0, 0), target);

        let outflow = collect_outflow(&w);
        apply_outflow(&mut w, &outflow);

        // Find where the attacker's slot landed: the position of the
        // first non-zero entry tells us write_start, which is
        // monotonically lower as dominance grows.
        let target = w.get(Coord::new(1, 0, 0)).unwrap();
        target
            .memory
            .iter()
            .position(|&v| v != 0)
            .unwrap_or(target.memory.len())
    }

    let high_threshold_pos = dom_for_threshold(4.0); // lower dominance, write further from start
    let low_threshold_pos = dom_for_threshold(1.0); // higher dominance, write closer to start
    assert!(
        low_threshold_pos < high_threshold_pos,
        "low move_threshold should put attacker slot earlier in memory"
    );
}

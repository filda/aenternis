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
    // Target: 50 slots, no outflow → post_outflow = 50.
    // r = 50/99 ≈ 0.505; dominance = 1 - 0.505/2 ≈ 0.747.
    // intrusion = floor(0.747 * 50) = 37; write_start = 50 - 37 = 13.
    //
    // We use a 50-slot target so the floor-rounded intrusion lands far
    // from the membrane (write_start ≪ memSize). With a 1-slot target
    // intrusion would round to zero and the attacker's slot would land
    // at the very end despite high dominance — that's a real edge case
    // of integer truncation, not the property we're testing here.
    let mut w = SparseWorld::new(0);
    let mut attacker = Cell::with_memory(vec![1; 100]);
    attacker.memory[0] = 0xAAAA;
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA_AAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = Cell::with_memory(vec![0xBBBB; 50]);
    target.origin_tag = 0xBBBB_BBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory.len(), 51);
    // Attacker slot lands at index 13 (= write_start), pushing the
    // last 37 of the original 0xBBBB block one slot up.
    assert_eq!(target.memory[13], 0xAAAA);
    // Bookend assertions: positions 0..13 and 14..51 are still 0xBBBB.
    assert_eq!(target.memory[0], 0xBBBB);
    assert_eq!(target.memory[12], 0xBBBB);
    assert_eq!(target.memory[14], 0xBBBB);
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
    // top of the dominance order and end up *earlier* in memory than
    // weak, even though `floor()` rounding keeps it from reaching
    // index 0 exactly.
    //
    // Setup:
    //   weak attacker at (1, 0, 0): 3 energy, emits 1 toward -x
    //   strong attacker at (-1, 0, 0): 100 energy, emits 1 toward +x
    //   target at origin: 5 slots, no outflow
    //
    // Strong: r = 5/99 ≈ 0.05, dom ≈ 0.974 → intrusion = floor(0.974*5)
    //   = 4 → write_start = 1. Memory becomes
    //   [0xB0, 0xAAAA, 0xB1, 0xB2, 0xB3, 0xB4]  (size 6)
    // Weak: r = 5/2 = 2.5, dom = clamp(1 - 1.25, 0, 1) = 0 →
    //   intrusion = 0 → write_start = 6. Memory becomes
    //   [0xB0, 0xAAAA, 0xB1, 0xB2, 0xB3, 0xB4, 0xCC]  (size 7)
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
    assert_eq!(target.memory.len(), 7);
    // Strong inflow is at index 1 (just inside the membrane).
    assert_eq!(target.memory[1], 0xAAAA);
    // Weak inflow is at the very end.
    assert_eq!(*target.memory.last().unwrap(), 0xCC);
    // Strong's index < weak's index — the load-bearing property.
    let strong_idx = target.memory.iter().position(|&v| v == 0xAAAA).unwrap();
    let weak_idx = target.memory.iter().position(|&v| v == 0xCC).unwrap();
    assert!(
        strong_idx < weak_idx,
        "strong should land earlier than weak ({strong_idx} vs {weak_idx})"
    );
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

// ----- inflow tracking (for the `sinflow` opcode) ----------------------------

#[test]
fn apply_outflow_populates_target_inflow_per_direction() {
    // Two attackers writing into the same target from different sides.
    // After apply_outflow, target.inflow should reflect both:
    //   inflow[Xn] = slots received from -x (= attacker at +1 emitting toward -x)
    //   inflow[Xp] = slots received from +x (= attacker at -1 emitting toward +x)
    let mut w = SparseWorld::new(0);

    let mut from_plus_x = Cell::with_memory(vec![0xCC, 0, 0]);
    from_plus_x.rates[Direction::Xn.index()] = 1;
    from_plus_x.pointers[Direction::Xn.index()] = 0;
    w.insert(Coord::new(1, 0, 0), from_plus_x);

    let mut from_minus_x = Cell::with_memory(vec![0xAA, 0, 0, 0, 0]);
    from_minus_x.rates[Direction::Xp.index()] = 2;
    from_minus_x.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::new(-1, 0, 0), from_minus_x);

    let target = Cell::with_memory(vec![0xB0; 5]);
    w.insert(Coord::ORIGIN, target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::ORIGIN).unwrap();
    // `inflow[d]` counts slots that arrived through the d-face. A
    // neighbor at +x emitting toward -x reaches the target through
    // the target's +x face → `inflow[Xp]`. A neighbor at -x emitting
    // toward +x reaches the target through the target's -x face →
    // `inflow[Xn]`.
    assert_eq!(
        target.inflow[Direction::Xp.index()],
        1,
        "+x neighbor → through Xp face"
    );
    assert_eq!(
        target.inflow[Direction::Xn.index()],
        2,
        "-x neighbor → through Xn face"
    );
    // Other directions stay zero.
    assert_eq!(target.inflow[Direction::Yp.index()], 0);
    assert_eq!(target.inflow[Direction::Yn.index()], 0);
    assert_eq!(target.inflow[Direction::Zp.index()], 0);
    assert_eq!(target.inflow[Direction::Zn.index()], 0);
}

#[test]
fn apply_outflow_clears_inflow_from_previous_tick() {
    // Cell starts with stale inflow from a hypothetical earlier tick.
    // After apply_outflow with no inflow this tick, inflow must be
    // zeroed — `sinflow` semantics is "last tick", not "ever received".
    let mut w = SparseWorld::new(0);
    let mut cell = Cell::with_memory(vec![1, 2, 3]);
    cell.inflow = [99, 99, 99, 99, 99, 99];
    w.insert(Coord::ORIGIN, cell);

    // Empty outflow → no inflows applied this tick.
    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.inflow, [0; 6]);
}

// ----- move_threshold knob ---------------------------------------------------

#[test]
fn raising_move_threshold_raises_dominance() {
    // The dominance formula is `dom = 1 - r / move_threshold`. For a
    // fixed r, *raising* `move_threshold` makes the divisor larger,
    // which makes the quotient smaller, which makes `dom` larger.
    // Higher dominance ⇒ deeper intrusion ⇒ attacker slot lands
    // earlier (lower index) in the target.
    //
    // Setup: attacker (6 energy, emit 1) → post_burn = 5. Target 5
    // empty slots → r = 5/5 = 1.
    //   move_threshold = 1.0 → dom = 0   → intrusion 0 → idx 5
    //   move_threshold = 4.0 → dom = 0.75 → intrusion 3 → idx 2
    fn position_for_threshold(mt: f32) -> usize {
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

        let target = w.get(Coord::new(1, 0, 0)).unwrap();
        target
            .memory
            .iter()
            .position(|&v| v != 0)
            .unwrap_or(target.memory.len())
    }

    let pos_low_threshold = position_for_threshold(1.0); // dom 0   → late
    let pos_high_threshold = position_for_threshold(4.0); // dom 0.75 → early
    assert!(
        pos_high_threshold < pos_low_threshold,
        "higher move_threshold should put attacker slot earlier in memory \
         (got pos@mt=4.0 = {pos_high_threshold}, pos@mt=1.0 = {pos_low_threshold})"
    );
}

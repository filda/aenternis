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
    let mut attacker = w.alloc_cell(&[10, 20, 30]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA_AAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = w.alloc_cell(&[99u32; 100]);
    target.origin_tag = 0xBBBB_BBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    // memSize_before_inflow = 100, dominance ≈ 0, write_start = 100,
    // so the inflow slot lands at the very end.
    assert_eq!(target.memory_len(), 101);
    assert_eq!(target.memory(w.arena())[100], 10);
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
    let mut attacker = w.alloc_cell(&[1; 100]);
    attacker.set_memory_slot(w.arena_mut(), 0, 0xAAAA);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA_AAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = w.alloc_cell(&[0xBBBB; 50]);
    target.origin_tag = 0xBBBB_BBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory_len(), 51);
    // Attacker slot lands at index 13 (= write_start), pushing the
    // last 37 of the original 0xBBBB block one slot up.
    assert_eq!(target.memory(w.arena())[13], 0xAAAA);
    // Bookend assertions: positions 0..13 and 14..51 are still 0xBBBB.
    assert_eq!(target.memory(w.arena())[0], 0xBBBB);
    assert_eq!(target.memory(w.arena())[12], 0xBBBB);
    assert_eq!(target.memory(w.arena())[14], 0xBBBB);
    // Top dominance ≥ 0.5, target inherits attacker's origin tag.
    assert_eq!(target.origin_tag, 0xAAAA_AAAA);
}

#[test]
fn void_target_gets_full_dominance() {
    // Target doesn't exist (void). target_E_post = 0, attacker_post = N.
    // r = 0; dominance = 1.
    let mut w = SparseWorld::new(0);
    let mut attacker = w.alloc_cell(&[1, 2, 3, 4, 5]);
    attacker.set_memory_slot(w.arena_mut(), 0, 0xCAFE);
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
    assert_eq!(target.memory(w.arena()), vec![0xCAFE, 2]);
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
    let mut attacker = w.alloc_cell(&[0xA0, 1, 2, 3, 4, 5]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = w.alloc_cell(&[0xB0, 0xB1, 0xB2, 0xB3, 0xB4]);
    target.origin_tag = 0xBBBB;
    w.insert(Coord::new(1, 0, 0), target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory(w.arena()), vec![0xB0, 0xB1, 0xB2, 0xA0, 0xB3, 0xB4]);
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
    let mut attacker = w.alloc_cell(&[1, 2, 3, 4, 5, 6]);
    attacker.rates[Direction::Xp.index()] = 1;
    attacker.pointers[Direction::Xp.index()] = 0;
    attacker.origin_tag = 0xAAAA;
    w.insert(Coord::ORIGIN, attacker);

    let mut target = w.alloc_cell(&[10, 20, 30, 40, 50, 60]);
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

    let mut weak = w.alloc_cell(&[0xCC, 0, 0]);
    weak.rates[Direction::Xn.index()] = 1;
    weak.pointers[Direction::Xn.index()] = 0;
    w.insert(Coord::new(1, 0, 0), weak);

    let mut strong = w.alloc_cell(&[0xAA; 100]);
    strong.set_memory_slot(w.arena_mut(), 0, 0xAAAA);
    strong.rates[Direction::Xp.index()] = 1;
    strong.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::new(-1, 0, 0), strong);

    let target = w.alloc_cell(&[0xB0, 0xB1, 0xB2, 0xB3, 0xB4]);
    w.insert(Coord::ORIGIN, target);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(target.memory_len(), 7);
    // Strong inflow is at index 1 (just inside the membrane).
    assert_eq!(target.memory(w.arena())[1], 0xAAAA);
    // Weak inflow is at the very end.
    assert_eq!(*target.memory(w.arena()).last().unwrap(), 0xCC);
    // Strong's index < weak's index — the load-bearing property.
    let strong_idx = target.memory(w.arena()).iter().position(|&v| v == 0xAAAA).unwrap();
    let weak_idx = target.memory(w.arena()).iter().position(|&v| v == 0xCC).unwrap();
    assert!(
        strong_idx < weak_idx,
        "strong should land earlier than weak ({strong_idx} vs {weak_idx})"
    );
}

#[test]
fn six_inflows_with_equal_dominance_apply_in_canonical_direction_order() {
    // Six attackers, one per cardinal neighbor of origin, all with the
    // same dominance — tied scores force the secondary sort key
    // (`dir_from_target` ascending = canonical `Direction::ALL`
    // order: Xp, Xn, Yp, Yn, Zp, Zn) to determine processing order.
    //
    // Without the sort (i.e., if the `entries.len() <= 1` fast-skip is
    // mutated to skip multi-entry vectors instead), the apply order
    // falls back to insertion order — which is the FxHashMap
    // iteration order over the six attacker coords. That order is
    // hash-derived and almost never matches canonical Direction
    // order for these specific coords, so the resulting memory layout
    // diverges from the expected one and this test fires. The
    // existing two-attacker `multiple_inflows_apply_strongest_first`
    // can survive the same mutation when FxHashMap happens to iterate
    // in dominance order; six entries spread across well-mixed coord
    // hashes makes that coincidence vanishingly unlikely.
    //
    // Setup:
    //   target at origin: 5 slots [B0..B4], no outflow → target_post = 5
    //   each attacker: 6 energy, emits 1 toward origin → post_burn = 5
    //   r = target_post / attacker_post_burn = 5 / 5 = 1.0
    //   dominance = clamp(1 - 1.0/2.0, 0, 1) = 0.5 (move_threshold = 2)
    //
    // Sorted apply trace (intrusion_depth = floor(0.5 * mem_size)):
    //   start mem=5: A0 (dir_from_target=Xp) intrudes 2, write_start 3
    //     [B0,B1,B2,A0,B3,B4] mem=6
    //   A1 (Xn) intrudes 3, write_start 3
    //     [B0,B1,B2,A1,A0,B3,B4] mem=7
    //   A2 (Yp) intrudes 3 (floor(0.5*7)=3), write_start 4
    //     [B0,B1,B2,A1,A2,A0,B3,B4] mem=8
    //   A3 (Yn) intrudes 4, write_start 4
    //     [B0,B1,B2,A1,A3,A2,A0,B3,B4] mem=9
    //   A4 (Zp) intrudes 4 (floor(0.5*9)=4), write_start 5
    //     [B0,B1,B2,A1,A3,A4,A2,A0,B3,B4] mem=10
    //   A5 (Zn) intrudes 5, write_start 5
    //     [B0,B1,B2,A1,A3,A5,A4,A2,A0,B3,B4] mem=11
    let mut w = SparseWorld::new(0);

    let target = w.alloc_cell(&[0xB0, 0xB1, 0xB2, 0xB3, 0xB4]);
    w.insert(Coord::ORIGIN, target);

    // Attacker layout: each at the cardinal neighbor of origin in
    // direction `face`, emitting back toward origin via the opposite
    // direction `face.opposite()`. Token value encodes which attacker
    // it came from (0xA0..0xA5 in canonical Direction order).
    let attackers: [(Direction, u32); 6] = [
        (Direction::Xp, 0x00A0),
        (Direction::Xn, 0x00A1),
        (Direction::Yp, 0x00A2),
        (Direction::Yn, 0x00A3),
        (Direction::Zp, 0x00A4),
        (Direction::Zn, 0x00A5),
    ];
    for &(face, token) in &attackers {
        let coord = Coord::ORIGIN.neighbor(face);
        // Memory: 5 filler slots + 1 token at the end. The emit pointer
        // points at the token so it's the slot that flows out.
        let mut mem = vec![0u32; 5];
        mem.push(token);
        let mut a = w.alloc_cell(&mem);
        let emit_dir = face.opposite();
        a.rates[emit_dir.index()] = 1;
        a.pointers[emit_dir.index()] = 5;
        w.insert(coord, a);
    }

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let target = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(
        target.memory(w.arena()),
        vec![0xB0, 0xB1, 0xB2, 0x00A1, 0x00A3, 0x00A5, 0x00A4, 0x00A2, 0x00A0, 0xB3, 0xB4,],
        "memory layout must reflect canonical-direction tie-break order — \
         any divergence indicates the multi-inflow sort was skipped",
    );
}

#[test]
fn pc_wraps_into_range_when_shrink_outpaces_inflow_for_dual_role_cell() {
    // A cell is BOTH source (phase 1 shrink) AND target (phase 3 grow)
    // in the same tick. If the shrink is bigger than the grow and the
    // pre-tick pc was at the tail, pc ends up greater than the new
    // memory length — the trailing `target.pc %= memory.len()` in
    // `apply_outflow`'s body brings it back into range.
    //
    // The defensive comment in the source says this scenario only
    // arises if a future change adds shrink mid-step; the dual-role
    // case constructed here triggers it today, so the modulo is
    // observable and `%=` mutated to `/=` or `+=` produces a
    // different pc.
    //
    // Setup:
    //   A at origin: memory = [0..11] (12 slots), pc = 10. Emits 6
    //     slots toward +x → phase 1 shrinks A to 6 slots, pc stays 10.
    //   B at (1, 0, 0): memory = [0x99] (1 slot), pc = 0. Emits 1 slot
    //     toward -x (origin) → phase 1 shrinks B to 0, the 1 slot
    //     becomes inflow for A.
    //
    // Phase 3 body for A:
    //   - dominance check: A_post = 6, B_post_burn = max(1, 0) = 1,
    //     r = 6.0, dom = clamp(1 - 6/2, 0, 1) = 0.
    //   - intrusion_depth = 0 * 6 = 0, write_start = 6 (tail).
    //   - splice [0x99] at 6 → A.memory = [0..5, 0x99], len = 7.
    //   - pc was 10; correct: 10 % 7 = 3. mutants:
    //       /= → 10 / 7 = 1
    //       += → 10 + 7 = 17
    let mut w = SparseWorld::new(0);

    let mem: Vec<u32> = (0u32..12).collect();
    let mut a = w.alloc_cell(&mem);
    a.pc = 10;
    a.rates[Direction::Xp.index()] = 6;
    a.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::ORIGIN, a);

    let mut b = w.alloc_cell(&[0x99]);
    b.rates[Direction::Xn.index()] = 1;
    b.pointers[Direction::Xn.index()] = 0;
    w.insert(Coord::new(1, 0, 0), b);

    let outflow = collect_outflow(&w);
    apply_outflow(&mut w, &outflow);

    let a = w.get(Coord::ORIGIN).expect("origin must still exist");
    assert_eq!(
        a.memory_len(),
        7,
        "A grew from 6 (post-shrink) to 7 via inflow"
    );
    assert_eq!(a.pc, 3, "pc must wrap via modulo: 10 % 7 == 3");
}

// ----- conservation ----------------------------------------------------------

#[test]
fn dominance_apply_still_conserves_total_slots() {
    let mut w = SparseWorld::new(0);
    let mut cell = w.alloc_cell(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
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

    let mut outflow = Outflow::default();
    outflow.insert(
        Coord::ORIGIN,
        one_directional_outflow(vec![42], Direction::Xp),
    );

    apply_outflow(&mut w, &outflow);

    assert!(!w.contains(Coord::ORIGIN));
    let target = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(target.memory(w.arena()), vec![42]);
}

// ----- inflow tracking (for the `sinflow` opcode) ----------------------------

#[test]
fn apply_outflow_populates_target_inflow_per_direction() {
    // Two attackers writing into the same target from different sides.
    // After apply_outflow, target.inflow should reflect both:
    //   inflow[Xn] = slots received from -x (= attacker at +1 emitting toward -x)
    //   inflow[Xp] = slots received from +x (= attacker at -1 emitting toward +x)
    let mut w = SparseWorld::new(0);

    let mut from_plus_x = w.alloc_cell(&[0xCC, 0, 0]);
    from_plus_x.rates[Direction::Xn.index()] = 1;
    from_plus_x.pointers[Direction::Xn.index()] = 0;
    w.insert(Coord::new(1, 0, 0), from_plus_x);

    let mut from_minus_x = w.alloc_cell(&[0xAA, 0, 0, 0, 0]);
    from_minus_x.rates[Direction::Xp.index()] = 2;
    from_minus_x.pointers[Direction::Xp.index()] = 0;
    w.insert(Coord::new(-1, 0, 0), from_minus_x);

    let target = w.alloc_cell(&[0xB0; 5]);
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
    let mut cell = w.alloc_cell(&[1, 2, 3]);
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

        let mut attacker = w.alloc_cell(&[1, 2, 3, 4, 5, 6]);
        attacker.rates[Direction::Xp.index()] = 1;
        attacker.pointers[Direction::Xp.index()] = 0;
        w.insert(Coord::ORIGIN, attacker);

        let target = w.alloc_cell(&[0; 5]);
        w.insert(Coord::new(1, 0, 0), target);

        let outflow = collect_outflow(&w);
        apply_outflow(&mut w, &outflow);

        let target = w.get(Coord::new(1, 0, 0)).unwrap();
        target
            .memory(w.arena())
            .iter()
            .position(|&v| v != 0)
            .unwrap_or(target.memory_len())
    }

    let pos_low_threshold = position_for_threshold(1.0); // dom 0   → late
    let pos_high_threshold = position_for_threshold(4.0); // dom 0.75 → early
    assert!(
        pos_high_threshold < pos_low_threshold,
        "higher move_threshold should put attacker slot earlier in memory \
         (got pos@mt=4.0 = {pos_high_threshold}, pos@mt=1.0 = {pos_low_threshold})"
    );
}

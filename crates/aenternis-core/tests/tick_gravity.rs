//! Integration tests for the gravity + pressure terms in
//! `compute_natural_rates` (see `docs/gravity-plan.md`).
//!
//! The hard invariants these lock in:
//!
//! 1. **Zero-default identity** — with `gravity == 0` and `pressure == 0`
//!    the rate path is the frozen fast path; and even on the *active*
//!    path, `alpha == 0` + `pressure == 0` must reduce bit-for-bit to the
//!    fast path (mass and pressure contribute nothing).
//! 2. **Conservation** — gravity/pressure only reshape per-direction
//!    rates; total outflow per cell is still clamped to its energy, so
//!    `total_energy` is invariant across ticks.
//! 3. **Determinism** — rates depend only on `(seed, tick, coord)` plus
//!    the snapshot, never on iteration order (sequential or parallel).
//! 4. **Direction of the gravity force** — energy flows *toward* mass
//!    (accretion) and is *held back* from the open void boundary. A sign
//!    flip in the gravity term reverses both and fails these.

use aenternis_core::tick::{compute_natural_rates, step};
use aenternis_core::{Coord, Direction, SparseWorld};

// ----- zero-default identity -------------------------------------------------

#[test]
fn active_path_with_no_mass_no_pressure_equals_fast_path() {
    // gravity != 0 takes the active path, but with alpha = 0 (no mass)
    // and pressure = 0 the drive collapses to `coeff*(E_c - E_nbr)` —
    // bit-for-bit the fast path, including the RNG draw schedule. A
    // two-cell world exercises both the `E_c > E_nbr` (draw) and
    // `E_c <= E_nbr` (no draw) branches.
    let build = |gravity: f64| {
        let mut w = SparseWorld::new(0xA11CE);
        w.gravity = gravity; // alpha stays 0, pressure stays 0
        w.insert_with_memory(Coord::new(0, 0, 0), &[1; 1000]);
        w.insert_with_memory(Coord::new(1, 0, 0), &[1; 10]);
        compute_natural_rates(&mut w, 0.15);
        (
            w.get(Coord::new(0, 0, 0)).unwrap().rates,
            w.get(Coord::new(1, 0, 0)).unwrap().rates,
        )
    };

    let fast = build(0.0);
    let active = build(0.5);
    assert_eq!(
        fast, active,
        "active path with alpha=0, pressure=0 must equal the fast path"
    );
}

// ----- direction of the gravity force: accretion -----------------------------

#[test]
fn gravity_makes_a_cell_emit_toward_higher_local_mass() {
    // H(200) — C(100) — D(100) along the x axis. C and D have equal
    // energy, so radiation between them is zero. But C sits next to the
    // dense H, so C's neighborhood mass (M_C = alpha*(200+100)) exceeds
    // D's (M_D = alpha*100). Gravity therefore drives D toward C even
    // though there is no energy gradient: D.rates[Xn] flips from 0 to
    // positive. A `M_c - M_nbr` sign flip would keep it at 0.
    let build = |gravity: f64, alpha: f64| {
        let mut w = SparseWorld::new(7);
        w.gravity = gravity;
        w.gravity_alpha = alpha;
        w.insert_with_memory(Coord::new(-1, 0, 0), &[1; 200]); // H
        w.insert_with_memory(Coord::new(0, 0, 0), &[1; 100]); // C
        w.insert_with_memory(Coord::new(1, 0, 0), &[1; 100]); // D
        compute_natural_rates(&mut w, 0.15);
        w.get(Coord::new(1, 0, 0)).unwrap().rates
    };

    let without = build(0.0, 0.0);
    let with = build(0.1, 0.05);

    assert_eq!(
        without[Direction::Xn.index()],
        0,
        "no energy gradient → D must not emit toward C without gravity"
    );
    // drive = 0 + 0 + 0.1*(alpha*300 - alpha*100) = 0.1*(15-5) = 1.0 → rate 1.
    assert_eq!(
        with[Direction::Xn.index()],
        1,
        "gravity must drive D toward the denser-neighborhood cell C"
    );
}

// ----- direction of the gravity force: void suppression ----------------------

#[test]
fn gravity_suppresses_outflow_into_the_void() {
    // H(300) — O(100) — void. Without gravity O emits 100*0.15 = 15 into
    // the void on +x. With gravity, O has mass-bearing neighbor H so
    // M_O = alpha*300 while the void has M = 0; the gravity term
    // 0.1*(0 - M_O) pulls the drive down, so O leaks *less* into the
    // void. A sign flip would make it leak *more*.
    let build = |gravity: f64, alpha: f64| {
        let mut w = SparseWorld::new(11);
        w.gravity = gravity;
        w.gravity_alpha = alpha;
        w.insert_with_memory(Coord::new(0, 0, 0), &[1; 300]); // H
        w.insert_with_memory(Coord::new(1, 0, 0), &[1; 100]); // O
        compute_natural_rates(&mut w, 0.15);
        w.get(Coord::new(1, 0, 0)).unwrap().rates[Direction::Xp.index()]
    };

    let without = build(0.0, 0.0);
    let with = build(0.1, 0.1);

    assert_eq!(without, 15, "100 * 0.15 = 15 into the void without gravity");
    // drive = 15 + 0 + 0.1*(0 - 0.1*300) = 15 - 3 = 12 → rate 12.
    assert_eq!(with, 12, "gravity must hold energy back from the void");
    assert!(with < without);
}

// ----- pressure pushes outward ----------------------------------------------

#[test]
fn pressure_adds_outward_flow_beyond_radiation() {
    // A dense cell next to a sparser one. Pressure Π ∝ E^γ is larger for
    // the denser cell, so (Π_self - Π_nbr) > 0 adds to the outward drive
    // on top of plain radiation: the dense cell emits *more* toward the
    // sparse neighbor with pressure on than off.
    let build = |pressure: f64| {
        let mut w = SparseWorld::new(0xF00D);
        w.pressure = pressure;
        w.pressure_eref = 1.0;
        w.pressure_gamma = 2.0;
        w.insert_with_memory(Coord::new(0, 0, 0), &[1; 100]);
        w.insert_with_memory(Coord::new(1, 0, 0), &[1; 40]);
        compute_natural_rates(&mut w, 0.15);
        w.get(Coord::new(0, 0, 0)).unwrap().rates[Direction::Xp.index()]
    };

    let without = build(0.0);
    let with = build(0.01);
    assert!(
        with > without,
        "pressure should increase outward flow from dense to sparse \
         (with={with}, without={without})"
    );
}

#[test]
fn pressure_is_a_difference_not_a_sum_between_equal_neighbors() {
    // Two equal-energy cells side by side, pressure on. Π is identical
    // for both, so the pressure term Π_self − Π_nbr is exactly zero and
    // there is no flow between them (no energy gradient either). A
    // `Π_self + Π_nbr` sign flip would make it 2·Π > 0 and open a
    // spurious flow — so this pins the subtraction, which a
    // direction-only test (dense vs sparse) cannot.
    let mut w = SparseWorld::new(0x9E11);
    w.pressure = 0.01;
    w.pressure_eref = 1.0;
    w.pressure_gamma = 2.0;
    w.insert_with_memory(Coord::new(0, 0, 0), &[1; 50]);
    w.insert_with_memory(Coord::new(1, 0, 0), &[1; 50]);
    compute_natural_rates(&mut w, 0.15);

    let a = w.get(Coord::new(0, 0, 0)).unwrap();
    let b = w.get(Coord::new(1, 0, 0)).unwrap();
    assert_eq!(
        a.rates[Direction::Xp.index()],
        0,
        "equal-energy neighbors must not flow under pressure (Π_self − Π_nbr = 0)"
    );
    assert_eq!(b.rates[Direction::Xn.index()], 0);
}

// ----- conservation ----------------------------------------------------------

#[test]
fn conservation_holds_with_gravity_over_many_ticks() {
    let mut w = SparseWorld::big_bang(0x00C0_FFEE, 50_000);
    w.gravity = 0.2;
    w.gravity_alpha = 0.05;
    let e0 = w.total_energy();
    for _ in 0..40 {
        step(&mut w, 0.15, 1);
        assert_eq!(w.total_energy(), e0, "gravity must conserve energy");
    }
}

#[test]
fn conservation_holds_with_pressure_over_many_ticks() {
    let mut w = SparseWorld::big_bang(0xBEEF, 50_000);
    w.pressure = 0.02;
    w.pressure_eref = 4.0;
    w.pressure_gamma = 2.0;
    let e0 = w.total_energy();
    for _ in 0..40 {
        step(&mut w, 0.15, 1);
        assert_eq!(w.total_energy(), e0, "pressure must conserve energy");
    }
}

#[test]
fn conservation_holds_with_gravity_and_pressure_combined() {
    let mut w = SparseWorld::big_bang(0x0D15_EA5E, 50_000);
    w.gravity = 0.15;
    w.gravity_alpha = 0.05;
    w.pressure = 0.02;
    w.pressure_eref = 4.0;
    w.pressure_gamma = 2.5;
    let e0 = w.total_energy();
    for _ in 0..40 {
        step(&mut w, 0.2, 1);
        assert_eq!(w.total_energy(), e0);
    }
}

// ----- determinism -----------------------------------------------------------

#[test]
fn gravity_run_is_deterministic() {
    let run = || {
        let mut w = SparseWorld::big_bang(0x5EED, 30_000);
        w.gravity = 0.2;
        w.gravity_alpha = 0.05;
        w.pressure = 0.01;
        w.pressure_eref = 4.0;
        for _ in 0..30 {
            step(&mut w, 0.15, 1);
        }
        // Canonical per-cell fingerprint, order-independent.
        let mut fp: Vec<(Coord, u32, u32)> = w
            .iter()
            .map(|(c, cell)| (*c, cell.energy(), cell.pc))
            .collect();
        fp.sort_unstable();
        fp
    };
    assert_eq!(run(), run(), "gravity+pressure run must be reproducible");
}

#[test]
fn gravity_is_deterministic_across_the_parallel_threshold() {
    // A dense 22^3 = 10 648-cell grid exceeds PAR_THRESHOLD (8192), so
    // the rate + outflow passes dispatch through rayon. Two independent
    // runs must still produce byte-identical state — any race introduced
    // by the gravity/mass scratch pass would diverge here.
    let run = || {
        let mut w = SparseWorld::new(0x6406);
        for x in 0..22 {
            for y in 0..22 {
                for z in 0..22 {
                    w.insert_with_memory(Coord::new(x, y, z), &[1; 3]);
                }
            }
        }
        w.gravity = 0.2;
        w.gravity_alpha = 0.05;
        w.rebuild_indices_if_dirty();
        for _ in 0..3 {
            step(&mut w, 0.15, 1);
        }
        let mut fp: Vec<(Coord, u32)> = w.iter().map(|(c, cell)| (*c, cell.energy())).collect();
        fp.sort_unstable();
        fp
    };
    assert_eq!(run(), run(), "parallel gravity path must be deterministic");
}

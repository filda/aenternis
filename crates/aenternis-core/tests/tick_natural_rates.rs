//! Integration tests for `compute_natural_rates`.
//!
//! Three properties we explicitly verify:
//!
//! 1. **Determinism** — same `(seed, tick, world state, coeff)` always
//!    produces the same rates, regardless of cell allocation order.
//! 2. **Locality** — a cell only sees its six face neighbors; void
//!    neighbors (= no cell) act as `E = 0` for gradient purposes.
//! 3. **Conservation** — total rate per cell never exceeds its energy
//!    after the proportional clamp.

use aenternis_core::tick::compute_natural_rates;
use aenternis_core::{Cell, Coord, Direction, SparseWorld};

// ----- structural sanity -----

#[test]
fn empty_world_is_a_noop() {
    let mut w = SparseWorld::new(0);
    compute_natural_rates(&mut w, 0.15);
    assert!(w.is_empty());
}

#[test]
fn empty_cell_gets_all_zero_rates() {
    let mut w = SparseWorld::new(0);
    w.insert(Coord::ORIGIN, Cell::new()); // energy = 0
    compute_natural_rates(&mut w, 0.15);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.rates, [0; 6]);
}

// ----- gradient and locality -----

#[test]
fn isolated_cell_emits_into_void_in_every_direction() {
    // A single cell with energy 1000 has every neighbor at E=0 (void).
    // With coeff = 0.15 the rate per direction is stochastic_floor(150);
    // we can't predict the exact value, but every direction should be
    // strictly positive when myE is large enough that 1000 * 0.15 = 150
    // overwhelms the per-call randomness.
    let mut w = SparseWorld::big_bang(7, 1000);
    compute_natural_rates(&mut w, 0.15);
    let cell = w.get(Coord::ORIGIN).unwrap();
    for &d in &Direction::ALL {
        assert!(
            cell.rates[d.index()] > 0,
            "expected positive rate for {d:?}, got {}",
            cell.rates[d.index()]
        );
    }
}

#[test]
fn equal_energy_neighbors_get_zero_rate_between_them() {
    // Two cells with identical energy, side by side along +x.
    // Rate from A to B = (10 - 10) * coeff = 0. Same in the other
    // direction. Other (void) neighbors still emit normally.
    let mut w = SparseWorld::new(42);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1; 10]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1; 10]));
    compute_natural_rates(&mut w, 0.15);

    let a = w.get(Coord::new(0, 0, 0)).unwrap();
    let b = w.get(Coord::new(1, 0, 0)).unwrap();

    // Rate A → B (across the +x face) is zero.
    assert_eq!(a.rates[Direction::Xp.index()], 0);
    // Rate B → A (across its -x face) is zero.
    assert_eq!(b.rates[Direction::Xn.index()], 0);
}

#[test]
fn lower_energy_neighbor_does_not_emit_back() {
    // A is high-energy, B is low-energy. A → B is positive (gradient
    // myE > nE), but B → A is zero (gradient myE < nE).
    let mut w = SparseWorld::new(123);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1; 1000]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1; 10]));
    compute_natural_rates(&mut w, 0.15);

    let a = w.get(Coord::new(0, 0, 0)).unwrap();
    let b = w.get(Coord::new(1, 0, 0)).unwrap();

    assert!(
        a.rates[Direction::Xp.index()] > 0,
        "high-energy cell should emit toward low-energy neighbor"
    );
    assert_eq!(
        b.rates[Direction::Xn.index()],
        0,
        "low-energy cell must not emit toward high-energy neighbor"
    );
}

// ----- determinism and order independence -----

#[test]
fn same_seed_and_state_produce_same_rates() {
    let mut a = SparseWorld::big_bang(0xDEAD_BEEF, 1000);
    let mut b = SparseWorld::big_bang(0xDEAD_BEEF, 1000);
    compute_natural_rates(&mut a, 0.15);
    compute_natural_rates(&mut b, 0.15);
    let ca = a.get(Coord::ORIGIN).unwrap();
    let cb = b.get(Coord::ORIGIN).unwrap();
    assert_eq!(ca.rates, cb.rates);
}

#[test]
fn different_ticks_produce_different_rates() {
    // Per-cell-per-tick RNG depends on tick, so for a fractional
    // expected rate (energy * coeff is non-integer) the rate vector
    // between two adjacent ticks should differ with high probability
    // — but the chance two adjacent ticks happen to match all 6
    // directions is roughly 0.5^6 ≈ 1.6 %, which is a flaky test.
    //
    // We compare 5 consecutive ticks instead: the chance every pair
    // matches is 0.5^(6 * 4) ≈ 6e-8, well below any flake threshold.
    let runs: Vec<[u32; 6]> = (0..5)
        .map(|tick| {
            let mut w = SparseWorld::big_bang(99, 17); // 17 * 0.15 = 2.55
            w.tick = tick;
            compute_natural_rates(&mut w, 0.15);
            w.get(Coord::ORIGIN).unwrap().rates
        })
        .collect();

    let all_same = runs.windows(2).all(|w| w[0] == w[1]);
    assert!(!all_same, "all 5 ticks produced identical rates");
}

#[test]
fn rates_independent_of_insert_order() {
    // Insert two cells in two different orders and verify that the
    // resulting rates are identical for both. Since per-cell-per-tick
    // RNG is keyed only on (seed, tick, coord), iteration order
    // through the BTreeMap can't change the outcome.
    let make_world = |insert_b_first: bool| {
        let mut w = SparseWorld::new(7);
        let a = Coord::new(0, 0, 0);
        let b = Coord::new(1, 0, 0);
        if insert_b_first {
            w.insert(b, Cell::with_memory(vec![1; 50]));
            w.insert(a, Cell::with_memory(vec![1; 100]));
        } else {
            w.insert(a, Cell::with_memory(vec![1; 100]));
            w.insert(b, Cell::with_memory(vec![1; 50]));
        }
        compute_natural_rates(&mut w, 0.15);
        let a_rates = w.get(a).unwrap().rates;
        let b_rates = w.get(b).unwrap().rates;
        (a_rates, b_rates)
    };

    let (a1, b1) = make_world(false);
    let (a2, b2) = make_world(true);
    assert_eq!(a1, a2);
    assert_eq!(b1, b2);
}

// ----- conservation -----

#[test]
fn total_rate_never_exceeds_cell_energy() {
    // A cluster of cells with various energies; check the conservation
    // invariant for every cell after compute_natural_rates.
    let mut w = SparseWorld::new(2024);
    w.insert(Coord::new(0, 0, 0), Cell::with_memory(vec![1; 10]));
    w.insert(Coord::new(1, 0, 0), Cell::with_memory(vec![1; 5]));
    w.insert(Coord::new(0, 1, 0), Cell::with_memory(vec![1; 3]));
    w.insert(Coord::new(0, 0, 1), Cell::with_memory(vec![1; 1]));

    compute_natural_rates(&mut w, 0.30);

    for (coord, cell) in &w {
        assert!(
            cell.total_rate() <= cell.energy(),
            "cell at {coord:?} has total_rate {} > energy {}",
            cell.total_rate(),
            cell.energy()
        );
    }
}

#[test]
fn proportional_clamp_kicks_in_for_high_coeff() {
    // With coeff = 1.0, an isolated cell with energy 100 sees a rate of
    // 100 per direction (the full gradient against void) — so the sum
    // of rates is 600, far above its energy. The proportional clamp
    // should bring the sum back down to exactly 100.
    let mut w = SparseWorld::big_bang(0, 100);
    compute_natural_rates(&mut w, 1.0);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.total_rate(), 100);
}

// ----- coeff = 0 -----

#[test]
fn coeff_zero_kills_all_rates() {
    let mut w = SparseWorld::big_bang(0, 1000);
    compute_natural_rates(&mut w, 0.0);
    let cell = w.get(Coord::ORIGIN).unwrap();
    assert_eq!(cell.rates, [0; 6]);
}

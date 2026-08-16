//! Host selection for "Run Program" / Project Pilgrim possession.
//!
//! `possess()` is energy-neutral: it overwrites an EXISTING cell (see
//! `docs/pilgrim.md`), so the host must already hold at least the
//! program's length plus a reserve (scratch + compute + emission fuel).
//! This module scans a cell snapshot and picks an eligible host.
//!
//! Mirror of `src/host-select.ts` — the WASM worker selects its host in
//! JS over the snapshot it already owns, while the native server calls
//! this function; both must pick the **same** host for the same world,
//! so the arithmetic below follows the TS implementation operation for
//! operation (all accumulation in `f64`, same iteration order over the
//! lex-sorted snapshot, same `>` tie-break keeping the first seen).
//!
//! Snapshot layout ([`crate::snapshot::SNAPSHOT_STRIDE`] = 6): per cell
//! `[x, y, z, energy, origin_tag, appearance]`, coords stored as their
//! `u32` bit pattern.

use crate::snapshot::SNAPSHOT_STRIDE;
use crate::Coord;

/// Offset of the `energy` field within a snapshot record.
const ENERGY_OFFSET: usize = 3;

/// Pick a host cell for an injected program from a flat cell `snapshot`
/// (layout of [`crate::snapshot::snapshot_into`]).
///
/// Eligibility (hard constraint): a host's energy must be at least
/// `code_len + reserve` — possession is energy-neutral and cannot grow
/// the cell.
///
/// Among eligible cells the one **farthest from the energy-weighted
/// center of mass** wins: the pilgrim starts on the cool periphery with
/// the longest journey inward toward the dense core (`docs/pilgrim.md`).
/// Ties resolve to the first cell seen — the snapshot is emitted in
/// `(x, y, z)` lex order, so that is the lex-smallest coord.
///
/// Returns `None` if no cell is eligible (or the world is empty) — the
/// caller should refuse the run.
#[must_use]
// Coordinate/energy → f64 conversions mirror JS number semantics, where
// every value is f64 to begin with; magnitudes stay well inside f64's
// exact-integer range for any realistic world. The `u32 as i32` casts
// are deliberate bit reinterpretations (snapshot coords are i32 stored
// as their u32 bit pattern).
#[allow(clippy::cast_possible_wrap)]
pub fn find_host(snapshot: &[u32], code_len: usize, reserve: u32) -> Option<Coord> {
    let need = code_len as u64 + u64::from(reserve);

    // Pass 1: energy-weighted center of mass over every live cell.
    let mut sum_e = 0.0_f64;
    let mut cx = 0.0_f64;
    let mut cy = 0.0_f64;
    let mut cz = 0.0_f64;
    for cell in snapshot.chunks_exact(SNAPSHOT_STRIDE) {
        let e = f64::from(cell[ENERGY_OFFSET]);
        sum_e += e;
        cx += f64::from(cell[0] as i32) * e;
        cy += f64::from(cell[1] as i32) * e;
        cz += f64::from(cell[2] as i32) * e;
    }
    if sum_e == 0.0 {
        return None;
    }
    cx /= sum_e;
    cy /= sum_e;
    cz /= sum_e;

    // Pass 2: eligible cell farthest from the center of mass. `-1.0`
    // (not `0.0`) so a sole eligible host sitting exactly at the COM
    // (dist² == 0) still wins the `dist2 > best_dist` comparison.
    let mut best: Option<Coord> = None;
    let mut best_dist = -1.0_f64;
    for cell in snapshot.chunks_exact(SNAPSHOT_STRIDE) {
        if u64::from(cell[ENERGY_OFFSET]) < need {
            continue;
        }
        let x = cell[0] as i32;
        let y = cell[1] as i32;
        let z = cell[2] as i32;
        let dx = f64::from(x) - cx;
        let dy = f64::from(y) - cy;
        let dz = f64::from(z) - cz;
        // Plain mul+add (no `mul_add`/FMA): JS evaluates `dx*dx + dy*dy +
        // dz*dz` with an intermediate rounding per operation, and this
        // function must be bit-identical to the TS implementation.
        #[allow(clippy::suboptimal_flops)]
        let dist2 = dx * dx + dy * dy + dz * dz;
        if dist2 > best_dist {
            best_dist = dist2;
            best = Some(Coord::new(x, y, z));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::find_host;
    use crate::snapshot::SNAPSHOT_STRIDE;
    use crate::Coord;

    /// Build a flat snapshot (stride 6) from `[x, y, z, energy]` records;
    /// `origin_tag` / `appearance` are zero-filled. Coords are written as
    /// their `u32` bit pattern so negatives round-trip the decode.
    fn snapshot(cells: &[[i32; 4]]) -> Vec<u32> {
        let mut out = vec![0_u32; cells.len() * SNAPSHOT_STRIDE];
        for (i, &[x, y, z, e]) in cells.iter().enumerate() {
            out[i * SNAPSHOT_STRIDE] = x as u32;
            out[i * SNAPSHOT_STRIDE + 1] = y as u32;
            out[i * SNAPSHOT_STRIDE + 2] = z as u32;
            out[i * SNAPSHOT_STRIDE + 3] = u32::try_from(e).unwrap();
        }
        out
    }

    #[test]
    fn returns_none_for_an_empty_world() {
        assert_eq!(find_host(&[], 1, 0), None);
    }

    #[test]
    fn picks_the_only_eligible_cell() {
        let snap = snapshot(&[[1, 2, 3, 50]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(1, 2, 3)));
    }

    #[test]
    fn returns_none_when_no_cell_has_enough_energy() {
        let snap = snapshot(&[[1, 2, 3, 5], [4, 5, 6, 9]]);
        assert_eq!(find_host(&snap, 10, 0), None);
    }

    #[test]
    fn counts_the_reserve_toward_the_requirement() {
        let snap = snapshot(&[[1, 2, 3, 12]]);
        assert_eq!(find_host(&snap, 10, 3), None); // need 13 > 12
        assert_eq!(find_host(&snap, 10, 2), Some(Coord::new(1, 2, 3))); // need 12
    }

    #[test]
    fn accepts_a_cell_whose_energy_exactly_meets_the_requirement() {
        let snap = snapshot(&[[7, 7, 7, 12]]);
        assert_eq!(find_host(&snap, 12, 0), Some(Coord::new(7, 7, 7)));
    }

    #[test]
    fn picks_the_eligible_cell_farthest_from_the_center_of_mass() {
        // A heavy core near x=0 anchors the COM; among eligible cells the
        // farthest one wins regardless of its own (lower) energy.
        let snap = snapshot(&[[0, 0, 0, 1000], [10, 0, 0, 50], [100, 0, 0, 50]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(100, 0, 0)));
    }

    #[test]
    fn weights_the_center_of_mass_by_energy() {
        // Huge mass at x=100 drags the COM to ~99.8, so the small eligible
        // cell at x=0 is the farthest — not the nearest in raw coord terms.
        let snap = snapshot(&[[0, 0, 0, 20], [100, 0, 0, 10000], [110, 0, 0, 20]]);
        assert_eq!(find_host(&snap, 15, 0), Some(Coord::new(0, 0, 0)));
    }

    #[test]
    fn uses_the_y_component_of_the_energy_weighted_com() {
        // Heavy mass high in +y pulls the COM to ~95.5; the candidate at
        // y=0 is therefore the farthest.
        let snap = snapshot(&[[0, 0, 0, 50], [0, 90, 0, 50], [0, 100, 0, 1000]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(0, 0, 0)));
    }

    #[test]
    fn uses_the_z_component_of_the_energy_weighted_com() {
        let snap = snapshot(&[[0, 0, 0, 50], [0, 0, 90, 50], [0, 0, 100, 1000]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(0, 0, 0)));
    }

    #[test]
    fn divides_the_x_com_by_total_energy_not_multiplies() {
        // Correct COM.x ≈ 63.6 makes the x=200 cell the farthest; a `/=`
        // corruption (`*=`, `%=`) moves the COM and flips the winner.
        let snap = snapshot(&[[0, 0, 0, 50], [60, 0, 0, 1000], [200, 0, 0, 50]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(200, 0, 0)));
    }

    #[test]
    fn divides_the_y_com_by_total_energy_not_multiplies() {
        // Correct COM.y ≈ 63.6 makes the y=200 cell the farthest; a `/=`→`*=`
        // bug explodes COM.y and flips the winner to y=0.
        let snap = snapshot(&[[0, 0, 0, 50], [0, 60, 0, 1000], [0, 200, 0, 50]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(0, 200, 0)));
    }

    #[test]
    fn divides_the_z_com_by_total_energy_not_multiplies() {
        let snap = snapshot(&[[0, 0, 0, 50], [0, 0, 60, 1000], [0, 0, 200, 50]]);
        assert_eq!(find_host(&snap, 10, 0), Some(Coord::new(0, 0, 200)));
    }

    #[test]
    fn picks_a_sole_eligible_host_even_at_the_com() {
        // Single cell IS the COM (dist² = 0); pins the `best_dist = -1.0`
        // sentinel against a `0.0`/`+1.0` regression.
        let snap = snapshot(&[[5, 5, 5, 50]]);
        assert_eq!(find_host(&snap, 1, 0), Some(Coord::new(5, 5, 5)));
    }

    #[test]
    fn breaks_distance_ties_toward_the_first_lex_smallest_cell() {
        // Symmetric about the COM (x=0): both at distance 5; first wins.
        let snap = snapshot(&[[-5, 0, 0, 50], [5, 0, 0, 50]]);
        assert_eq!(find_host(&snap, 1, 0), Some(Coord::new(-5, 0, 0)));
    }

    #[test]
    fn decodes_negative_coordinates_from_their_u32_bit_pattern() {
        let snap = snapshot(&[[-3, -1, -100, 40]]);
        assert_eq!(find_host(&snap, 1, 0), Some(Coord::new(-3, -1, -100)));
    }
}

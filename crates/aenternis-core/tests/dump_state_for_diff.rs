//! Diagnostic dump of world state for cell-by-cell comparison against
//! JS prototype 9-B.
//!
//!   TICKS=1 cargo test --test `dump_state_for_diff` -- --ignored
//!
//! Writes to `reports/rust-tick<N>.txt` directly (no shell redirect or
//! sed filtering — same reason as the JS counterpart in
//! `prototypes/09b-sparse-world-3d/dump-state.js`). `TICKS` defaults to
//! 5. Pair with the JS dump and `diff reports/9b-tick<N>.txt
//! reports/rust-tick<N>.txt`.
//!
//! Expected: identical output line-for-line, given that the Rust core
//! always runs in 9-B parity (xorshift32, tick-1 RNG keying, f64
//! stochastic-floor, wrapping port, opcode set capped at 0x13) and
//! the JS hash is on `Math.imul`.
//!
//! When they diverge: the first differing line tells us which
//! `(coord, tick)` first sees a different `stochastic_floor` outcome,
//! `intrusion_depth`, or memory layout — narrowing the hunt drastically.
//! Bisect by running `TICKS=1`, then `TICKS=2`, etc., until you find
//! the first tick where the diff appears.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use aenternis_core::{tick, SparseWorld};

#[test]
#[ignore = "diagnostic — runs only with --ignored, writes to reports/rust-tick<N>.txt"]
fn dump_state_at_tick_n() {
    // Read TICKS from the environment, default 5 — must match the JS
    // counterpart's default for an apples-to-apples diff.
    let ticks: u32 = env::var("TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    // Self-xp-replicator program, hand-assembled (matches the asm
    // `setp xp, start; jmp start` from the UI preset).
    //
    // Layout:
    //   slot 0: 0x09 (setp opcode)
    //   slot 1: 0x00 (xp direction)
    //   slot 2: 0x00 (start address)
    //   slot 3: 0x07 (jmp opcode)
    //   slot 4: 0x00 (start address)
    let program: [u32; 5] = [0x09, 0, 0, 0x07, 0];

    let mut w = SparseWorld::big_bang_with_program(1234, 65_536, &program);
    w.move_threshold = 1.0;

    for _ in 0..ticks {
        tick::step(&mut w, 0.15, 1);
    }

    // Sort by (x, y, z) — same order the JS dump prints.
    let mut cells: Vec<_> = w.cells.iter().collect();
    cells.sort_by_key(|(c, _)| (c.x, c.y, c.z));

    // Write to `reports/rust-tick<N>.txt` at the workspace root. The
    // integration test binary lives in `target/debug/deps/...` but
    // `CARGO_MANIFEST_DIR` points at the crate root (`crates/
    // aenternis-core/`); two parents up is the workspace root.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let reports_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("reports");
    fs::create_dir_all(&reports_dir).expect("create reports dir");

    let out_path = reports_dir.join(format!("rust-tick{ticks}.txt"));
    let mut f = fs::File::create(&out_path).expect("create output file");
    for (coord, cell) in &cells {
        let m0 = cell.memory.first().copied().unwrap_or(0);
        let m1 = cell.memory.get(1).copied().unwrap_or(0);
        let m2 = cell.memory.get(2).copied().unwrap_or(0);
        writeln!(
            f,
            "({},{},{}) E={} mem[0..3]=[{},{},{}]",
            coord.x,
            coord.y,
            coord.z,
            cell.energy(),
            m0,
            m1,
            m2,
        )
        .expect("write cell line");
    }
    writeln!(f, "total energy: {}, cells: {}", w.total_energy(), w.len())
        .expect("write summary line");

    eprintln!(
        "Rust dump @ tick {ticks} → {} ({} cells)",
        out_path.display(),
        cells.len(),
    );

    // Sanity — at least make sure something happened (only when we
    // actually stepped at least once).
    if ticks > 0 {
        assert!(w.len() > 1);
    }
}

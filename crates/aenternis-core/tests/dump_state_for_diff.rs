//! Diagnostic state dump — archived historical harness.
//!
//!   TICKS=1 cargo test --test `dump_state_for_diff` -- --ignored
//!
//! Writes per-cell state to `reports/rust-tick<N>.txt` for diffing
//! against an external reference dump. `TICKS` defaults to 5.
//!
//! **Status: archival.** This harness was the cell-by-cell comparison
//! point against JS laboratory prototype 9-B during the Rust core port.
//! Bit-parity with that prototype was a working contract through
//! 2026-05-12 (see `docs/optimalizace-2026-05.md`); after the
//! Largest-Remainder shuffle landed in commit `3737536` and the
//! `apply_outflow` rope
//! merge landed in `26d7d53`, the Rust core's per-cell stream is no
//! longer expected to match the JS dump exactly. The test stays as
//! `#[ignore]` (never in CI) for forensic value: if a future
//! divergence ever surprises us, comparing against an old `reports/
//! 9b-tick<N>.txt` may still narrow the hunt to a specific
//! `(coord, tick)` first divergence.
//!
//! Frozen RNG / hash invariants — the load-bearing parts of stream
//! stability — live in `tests/rng.rs` and `tests/apply_outflow_bit_
//! parity.rs`. This file is not where to look for those.

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
    let mut cells: Vec<_> = w.iter().collect();
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
        let m0 = cell.memory(w.arena()).first().copied().unwrap_or(0);
        let m1 = cell.memory(w.arena()).get(1).copied().unwrap_or(0);
        let m2 = cell.memory(w.arena()).get(2).copied().unwrap_or(0);
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

//! Criterion benchmarks for the snapshot binary encoder.
//!
//! Measures `encode_snapshot_frame_into` over synthetic payloads at
//! four cell-count tiers. The output `Vec<u8>` is reused across
//! iterations to mirror the `WorldActor`'s per-tick reuse pattern,
//! so the numbers reflect the encoder body and not allocator churn.
//!
//! Run with `cargo bench -p aenternis-server`. HTML reports land
//! under `target/criterion/`.

// `criterion_group!` and `criterion_main!` expand to module-level
// items the workspace's `missing_docs = "warn"` lint can't see
// through. Same allow as the core-crate `tick` bench.
#![allow(missing_docs)]

use aenternis_server::protocol::{encode_snapshot_frame_into, SnapshotFrame, SNAPSHOT_STRIDE};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

/// Cell counts to sweep. Spans three orders of magnitude so the
/// bench captures both per-call fixed cost (small N) and
/// throughput-dominated steady-state (large N).
const CELL_COUNTS: &[usize] = &[1_000, 10_000, 100_000, 1_000_000];

/// Build a synthetic `snap` payload of `n_cells * SNAPSHOT_STRIDE`
/// `u32` values. Deterministic and side-effect-free so iterations
/// stay comparable across runs.
fn synth_snap(n_cells: usize) -> Vec<u32> {
    let stride = SNAPSHOT_STRIDE as usize;
    let mut v = Vec::with_capacity(n_cells * stride);
    for i in 0..n_cells {
        let i32_val = i as u32;
        v.push(i32_val);
        v.push(i32_val.wrapping_mul(7));
        v.push(i32_val.wrapping_mul(13));
        v.push(100u32.wrapping_add(i32_val));
        v.push(0xCAFE_BABE ^ i32_val);
        v.push(0xDEAD_BEEF ^ i32_val);
    }
    v
}

fn bench_encode_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_snapshot_frame_into");
    // The bench is bandwidth-dominated past ~100k cells; ten samples
    // is enough to stabilize the median while keeping the 1M case
    // under a minute total.
    group.sample_size(20);

    for &n in CELL_COUNTS {
        let snap = synth_snap(n);
        let frame = SnapshotFrame {
            tick: 42,
            cell_count: n as u32,
            total_energy: 100u32.wrapping_mul(n as u32),
            ms_per_tick: 16.0,
            bbox: [-1_000, 1_000, -1_000, 1_000, -1_000, 1_000],
            snap: &snap,
        };
        group.bench_with_input(BenchmarkId::from_parameter(n), &frame, |b, frame| {
            // Buffer lives across iterations — mirrors the
            // WorldActor's per-tick `encoded_buf` reuse.
            let mut buf = Vec::new();
            b.iter(|| {
                encode_snapshot_frame_into(&mut buf, frame);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_encode_snapshot);
criterion_main!(benches);

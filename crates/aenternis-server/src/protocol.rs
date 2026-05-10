//! Wire format for the aenternis-server WebSocket endpoint.
//!
//! Two layers, both little-endian where applicable:
//!
//! - **JSON** text frames for control messages, mirroring
//!   `src/protocol.ts` exactly (same `type` tags, same camelCase
//!   field names). Parsed via `serde_json`.
//! - **Binary** frames for snapshot and cellDetail. JSON would
//!   otherwise inflate the `Uint32Array` payload roughly 2\u{00d7} and
//!   force a string\u{2192}array round-trip on the JS side.
//!
//! ## Frame layouts
//!
//! ```text
//! snapshot   = [u8 tag=1][u32 tick][u32 cellCount][u32 totalEnergy]
//!              [f64 msPerTick][u32 stride]
//!              [i32 x_min][i32 x_max][i32 y_min][i32 y_max]
//!              [i32 z_min][i32 z_max]
//!              [u32 \u{00d7} (cellCount * stride)]
//!
//! cellDetail = [u8 tag=2][i32 x][i32 y][i32 z][u32 tick]
//!              [u32 prefix][u32 dataLen]
//!              [u32 \u{00d7} dataLen]
//! ```
//!
//! `stride` and `prefix` are constants today (6 and 28 respectively),
//! sent in-band so the parser doesn't need to recompile to keep up
//! with future layout changes.

use serde::{Deserialize, Serialize};

/// Inbound control message from the viewer. Mirrors
/// `MainToWorkerMsg` in `src/protocol.ts`: identical `type` tags and
/// camelCase field names.
///
/// `rename_all = "camelCase"` is applied **twice** — once on the enum
/// (renames variant tags: `Init` → `init`, etc.) and once on each
/// struct-shaped variant (renames its fields: `move_threshold` →
/// `moveThreshold`). The enum-level attribute does *not* descend into
/// variant fields, so without the per-variant attributes serde would
/// look for the `snake_case` field name and silently default to
/// `None`/empty for the one we actually receive.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ClientMessage {
    /// Reset the shared world with the given seed/energy and an
    /// optional program prefix written into the origin cell's memory.
    /// Affects every connected client \u{2014} this is a global reset.
    #[serde(rename_all = "camelCase")]
    Init {
        /// PRNG seed for `SparseWorld::big_bang`.
        seed: u32,
        /// Total starting energy at the origin cell.
        energy: u32,
        /// Diffusion coefficient passed to `tick::step`.
        coeff: f64,
        /// CPU compute constant `k` (`instructions_per_cell = floor(energy / k)`).
        k: u32,
        /// Optional override of the world's `move_threshold` (default 2.0).
        #[serde(default)]
        move_threshold: Option<f32>,
        /// Optional program prefix written verbatim into the origin
        /// cell's memory before the deterministic RNG fills the rest.
        #[serde(default)]
        program: Vec<u32>,
    },
    /// Update tick-time parameters in place \u{2014} the world's state
    /// is not touched.
    #[serde(rename_all = "camelCase")]
    Config {
        coeff: f64,
        k: u32,
        #[serde(default)]
        move_threshold: Option<f32>,
    },
    /// Resume (`true`) or pause (`false`) the autonomous tick loop.
    Running { running: bool },
    /// Single-step: advance the world by exactly one tick and emit
    /// one snapshot, regardless of the current `running` flag.
    Step,
    /// Request a full inspect of the cell at `(x, y, z)`. The reply
    /// is a binary cellDetail frame addressed back to the requesting
    /// client only.
    Inspect { x: i32, y: i32, z: i32 },
}

/// Outbound JSON control message to the viewer. Snapshot and
/// cellDetail are binary frames, encoded by [`encode_snapshot_frame`]
/// and [`encode_cell_detail_frame`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ServerControl {
    /// Sent immediately after the WebSocket handshake completes.
    Ready,
    /// Late-join state for a freshly connected client. Only
    /// `running` is needed; the rest of the welcome state arrives
    /// implicitly with the next snapshot frame.
    Welcome { running: bool },
}

/// Tag byte for a snapshot binary frame.
pub(crate) const SNAPSHOT_TAG: u8 = 1;

/// Tag byte for a cellDetail binary frame.
pub(crate) const CELL_DETAIL_TAG: u8 = 2;

/// Snapshot stride: number of `u32` fields per cell in the snapshot
/// payload. Matches `aenternis-wasm::World::SNAPSHOT_STRIDE` so JS
/// callers see an identical layout regardless of backend.
pub(crate) const SNAPSHOT_STRIDE: u32 = 6;

/// `CellDetail` prefix length: number of `u32` fields in the fixed
/// header before the variable-length memory dump. Matches
/// `aenternis-wasm::World::INSPECT_PREFIX`.
pub(crate) const INSPECT_PREFIX: u32 = 28;

/// All the inputs needed to encode a snapshot binary frame. Held by
/// reference so we don't copy the (potentially large) `snap` payload
/// just to hand it to the encoder.
pub(crate) struct SnapshotFrame<'a> {
    pub(crate) tick: u32,
    pub(crate) cell_count: u32,
    pub(crate) total_energy: u32,
    pub(crate) ms_per_tick: f64,
    /// `[x_min, x_max, y_min, y_max, z_min, z_max]`. Empty world
    /// senders should fill with zeros; the JS side already handles
    /// the degenerate bbox case.
    pub(crate) bbox: [i32; 6],
    /// Flat cell payload, `cell_count * SNAPSHOT_STRIDE` `u32`s long.
    pub(crate) snap: &'a [u32],
}

impl SnapshotFrame<'_> {
    /// Length in bytes of the fixed-width header preceding the cell
    /// payload (tag + tick + `cell_count` + `total_energy` +
    /// `ms_per_tick` + stride + bbox6).
    pub(crate) const HEADER_LEN: usize = 1 + 4 + 4 + 4 + 8 + 4 + 6 * 4;
}

/// Encode a snapshot binary frame.
pub(crate) fn encode_snapshot_frame(frame: &SnapshotFrame<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(SnapshotFrame::HEADER_LEN + frame.snap.len() * 4);
    out.push(SNAPSHOT_TAG);
    out.extend_from_slice(&frame.tick.to_le_bytes());
    out.extend_from_slice(&frame.cell_count.to_le_bytes());
    out.extend_from_slice(&frame.total_energy.to_le_bytes());
    out.extend_from_slice(&frame.ms_per_tick.to_le_bytes());
    out.extend_from_slice(&SNAPSHOT_STRIDE.to_le_bytes());
    for v in frame.bbox {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in frame.snap {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Inputs for a cellDetail binary frame. `data` is the
/// fixed-prefix-then-memory layout produced by
/// `world_actor::encode_cell_detail_data`.
pub(crate) struct CellDetailFrame<'a> {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) z: i32,
    pub(crate) tick: u32,
    /// Empty when the queried coordinate has no cell; the JS side
    /// renders that as "no cell" without the panel disappearing.
    pub(crate) data: &'a [u32],
}

impl CellDetailFrame<'_> {
    /// Length in bytes of the cellDetail header.
    pub(crate) const HEADER_LEN: usize = 1 + 4 * 4 + 4 + 4;
}

/// Encode a cellDetail binary frame.
pub(crate) fn encode_cell_detail_frame(frame: &CellDetailFrame<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(CellDetailFrame::HEADER_LEN + frame.data.len() * 4);
    out.push(CELL_DETAIL_TAG);
    out.extend_from_slice(&frame.x.to_le_bytes());
    out.extend_from_slice(&frame.y.to_le_bytes());
    out.extend_from_slice(&frame.z.to_le_bytes());
    out.extend_from_slice(&frame.tick.to_le_bytes());
    out.extend_from_slice(&INSPECT_PREFIX.to_le_bytes());
    let data_len = u32::try_from(frame.data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&data_len.to_le_bytes());
    for v in frame.data {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        encode_cell_detail_frame, encode_snapshot_frame, CellDetailFrame, ClientMessage,
        ServerControl, SnapshotFrame, CELL_DETAIL_TAG, INSPECT_PREFIX, SNAPSHOT_STRIDE,
        SNAPSHOT_TAG,
    };

    // -- JSON layer -----------------------------------------------------------

    #[test]
    fn parse_init_full() {
        // Pick float values whose IEEE-754 representation is exact in
        // both f32 and f64 (powers of two and their sums) so we can
        // assert by `to_bits()` without tolerance gymnastics.
        let json = r#"{
            "type": "init",
            "seed": 1234,
            "energy": 10,
            "coeff": 0.5,
            "k": 1,
            "moveThreshold": 1.5,
            "program": [1, 2, 3]
        }"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Init {
                seed,
                energy,
                coeff,
                k,
                move_threshold,
                program,
            } => {
                assert_eq!(seed, 1234);
                assert_eq!(energy, 10);
                assert_eq!(coeff.to_bits(), 0.5_f64.to_bits());
                assert_eq!(k, 1);
                let mt = move_threshold.expect("moveThreshold parses to Some");
                assert_eq!(mt.to_bits(), 1.5_f32.to_bits());
                assert_eq!(program, vec![1, 2, 3]);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_init_minimal_omits_optionals() {
        let json = r#"{"type":"init","seed":1,"energy":5,"coeff":0.2,"k":2}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Init {
                move_threshold,
                program,
                ..
            } => {
                assert_eq!(move_threshold, None);
                assert!(program.is_empty());
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_config() {
        // Exact-binary float values, see `parse_init_full`.
        let json = r#"{"type":"config","coeff":0.25,"k":2,"moveThreshold":2.5}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Config {
                coeff,
                k,
                move_threshold,
            } => {
                assert_eq!(coeff.to_bits(), 0.25_f64.to_bits());
                assert_eq!(k, 2);
                let mt = move_threshold.expect("moveThreshold parses to Some");
                assert_eq!(mt.to_bits(), 2.5_f32.to_bits());
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn parse_running() {
        let json = r#"{"type":"running","running":true}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg, ClientMessage::Running { running: true });
    }

    #[test]
    fn parse_step() {
        let msg: ClientMessage = serde_json::from_str(r#"{"type":"step"}"#).unwrap();
        assert_eq!(msg, ClientMessage::Step);
    }

    #[test]
    fn parse_inspect() {
        let json = r#"{"type":"inspect","x":-1,"y":2,"z":3}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg, ClientMessage::Inspect { x: -1, y: 2, z: 3 });
    }

    #[test]
    fn rejects_unknown_type() {
        let err = serde_json::from_str::<ClientMessage>(r#"{"type":"bogus"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_missing_required_init_fields() {
        let err = serde_json::from_str::<ClientMessage>(r#"{"type":"init","seed":1}"#);
        assert!(err.is_err());
    }

    #[test]
    fn serializes_ready() {
        let json = serde_json::to_string(&ServerControl::Ready).unwrap();
        assert_eq!(json, r#"{"type":"ready"}"#);
    }

    #[test]
    fn serializes_welcome_running_true() {
        let json = serde_json::to_string(&ServerControl::Welcome { running: true }).unwrap();
        assert_eq!(json, r#"{"type":"welcome","running":true}"#);
    }

    #[test]
    fn serializes_welcome_running_false() {
        let json = serde_json::to_string(&ServerControl::Welcome { running: false }).unwrap();
        assert_eq!(json, r#"{"type":"welcome","running":false}"#);
    }

    // -- Binary layer ---------------------------------------------------------

    /// Tiny LE byte reader for the round-trip tests; mirrors what
    /// `native-client.ts` will do with a `DataView`.
    struct Reader<'a> {
        buf: &'a [u8],
        pos: usize,
    }
    impl<'a> Reader<'a> {
        fn new(buf: &'a [u8]) -> Self {
            Self { buf, pos: 0 }
        }
        fn u8(&mut self) -> u8 {
            let v = self.buf[self.pos];
            self.pos += 1;
            v
        }
        fn u32(&mut self) -> u32 {
            let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }
        fn i32(&mut self) -> i32 {
            let v = i32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }
        fn f64(&mut self) -> f64 {
            let v = f64::from_le_bytes(self.buf[self.pos..self.pos + 8].try_into().unwrap());
            self.pos += 8;
            v
        }
    }

    #[test]
    fn snapshot_frame_round_trip_empty() {
        let frame = SnapshotFrame {
            tick: 7,
            cell_count: 0,
            total_energy: 0,
            ms_per_tick: 0.0,
            bbox: [0; 6],
            snap: &[],
        };
        let bytes = encode_snapshot_frame(&frame);
        assert_eq!(bytes.len(), SnapshotFrame::HEADER_LEN);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), SNAPSHOT_TAG);
        assert_eq!(r.u32(), 7);
        assert_eq!(r.u32(), 0);
        assert_eq!(r.u32(), 0);
        assert!((r.f64() - 0.0).abs() < f64::EPSILON);
        assert_eq!(r.u32(), SNAPSHOT_STRIDE);
        for _ in 0..6 {
            assert_eq!(r.i32(), 0);
        }
        assert_eq!(r.pos, bytes.len());
    }

    #[test]
    fn snapshot_frame_round_trip_three_cells() {
        // Three cells, each 6 u32s. Negative coords reinterpreted to u32
        // bits; JS will read them via Int32Array view to recover the sign.
        // `rustfmt::skip` keeps the per-cell row layout — the default
        // formatter would expand each element onto its own line.
        #[rustfmt::skip]
        let snap: Vec<u32> = vec![
            (-1_i32) as u32, 0, 0, 100, 0xCAFE_BABE, 0xDEAD_BEEF,
            0,               0, 0,  50,          0,           1,
            1, (-2_i32) as u32, 3,  25,       0xAA,        0xBB,
        ];
        let frame = SnapshotFrame {
            tick: 1234,
            cell_count: 3,
            total_energy: 175,
            ms_per_tick: 4.5,
            bbox: [-1, 1, -2, 0, 0, 3],
            snap: &snap,
        };
        let bytes = encode_snapshot_frame(&frame);
        assert_eq!(bytes.len(), SnapshotFrame::HEADER_LEN + snap.len() * 4);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), SNAPSHOT_TAG);
        assert_eq!(r.u32(), 1234);
        assert_eq!(r.u32(), 3);
        assert_eq!(r.u32(), 175);
        assert!((r.f64() - 4.5).abs() < f64::EPSILON);
        assert_eq!(r.u32(), SNAPSHOT_STRIDE);
        for expected in [-1, 1, -2, 0, 0, 3] {
            assert_eq!(r.i32(), expected);
        }
        for &expected in &snap {
            assert_eq!(r.u32(), expected);
        }
        assert_eq!(r.pos, bytes.len());
    }

    #[test]
    fn cell_detail_frame_round_trip_present() {
        // 28 prefix + 5 memory slots = 33 u32s.
        let mut data = Vec::with_capacity(33);
        data.extend(0_u32..28);
        data.extend([0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        let frame = CellDetailFrame {
            x: -3,
            y: 4,
            z: -5,
            tick: 99,
            data: &data,
        };
        let bytes = encode_cell_detail_frame(&frame);
        assert_eq!(bytes.len(), CellDetailFrame::HEADER_LEN + data.len() * 4);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), CELL_DETAIL_TAG);
        assert_eq!(r.i32(), -3);
        assert_eq!(r.i32(), 4);
        assert_eq!(r.i32(), -5);
        assert_eq!(r.u32(), 99);
        assert_eq!(r.u32(), INSPECT_PREFIX);
        assert_eq!(r.u32(), 33);
        for (i, &expected) in data.iter().enumerate() {
            assert_eq!(r.u32(), expected, "mismatch at index {i}");
        }
        assert_eq!(r.pos, bytes.len());
    }

    #[test]
    fn cell_detail_frame_round_trip_empty() {
        // Empty data signals "no cell at this coordinate" per
        // protocol.ts CellDetailMsg semantics.
        let frame = CellDetailFrame {
            x: 0,
            y: 0,
            z: 0,
            tick: 1,
            data: &[],
        };
        let bytes = encode_cell_detail_frame(&frame);
        assert_eq!(bytes.len(), CellDetailFrame::HEADER_LEN);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), CELL_DETAIL_TAG);
        assert_eq!(r.i32(), 0);
        assert_eq!(r.i32(), 0);
        assert_eq!(r.i32(), 0);
        assert_eq!(r.u32(), 1);
        assert_eq!(r.u32(), INSPECT_PREFIX);
        assert_eq!(r.u32(), 0);
    }
}

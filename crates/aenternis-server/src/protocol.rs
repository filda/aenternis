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
//!
//! metrics    = [u8 tag=3][u32 tick][u32 count]
//!              [f64 \u{00d7} count]
//! ```
//!
//! The metrics payload is the flat layout of
//! `aenternis_core::CodeMetrics::to_flat` (`[cells, entropy,
//! diversity, uniqueTypes, ...opcodeHist]`) — identical to what the
//! WASM `World::metrics` hands the worker, so JS unpacks one layout
//! regardless of backend.
//!
//! `stride` and `prefix` are constants today (6 and 28 respectively),
//! sent in-band so the parser doesn't need to recompile to keep up
//! with future layout changes.

use serde::{Deserialize, Serialize};

/// Full `init` payload — a field-for-field mirror of `InitMsg` in
/// `src/protocol.ts`. Every physics / genesis / sampling knob the WASM
/// worker accepts must exist here too, or the native backend silently
/// simulates a different world than the viewer asked for (that drift
/// happened once; the shared-fixture test below now pins the mirror).
///
/// `Serialize` exists for that fixture test only: round-tripping the
/// canonical JSON through this struct proves no field is silently
/// dropped by serde's ignore-unknown-fields default.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitMsg {
    /// PRNG seed for the big bang.
    pub(crate) seed: u32,
    /// Total starting energy at the origin cell.
    pub(crate) energy: u32,
    /// Diffusion coefficient passed to `tick::step`.
    pub(crate) coeff: f64,
    /// CPU compute constant `k` (`instructions_per_cell = floor(energy / k)`).
    pub(crate) k: u32,
    /// Optional override of the world's `move_threshold`.
    #[serde(default)]
    pub(crate) move_threshold: Option<f32>,
    /// Gravity coupling strength (omitted = off).
    #[serde(default)]
    pub(crate) gravity: Option<f64>,
    /// Mass coupling `alpha` in `m = alpha · E`.
    #[serde(default)]
    pub(crate) gravity_alpha: Option<f64>,
    /// Gravity cutoff radius `R`.
    #[serde(default)]
    pub(crate) gravity_radius: Option<i32>,
    /// Critical mass `m_crit` of the peaked gravitational potential (the
    /// inflation law, `docs/inflation-plan.md`); omitted/0 = off.
    #[serde(default)]
    pub(crate) gravity_crit_mass: Option<f64>,
    /// Pressure amplitude (omitted = off).
    #[serde(default)]
    pub(crate) pressure: Option<f64>,
    /// Polytropic index γ; snapped to the portable set on apply.
    #[serde(default)]
    pub(crate) pressure_gamma: Option<f64>,
    /// Reference energy `eref` for the pressure law.
    #[serde(default)]
    pub(crate) pressure_eref: Option<f64>,
    /// Density-coupled mutation ceiling (omitted = off).
    #[serde(default)]
    pub(crate) mutation_strength: Option<f64>,
    /// Half-saturation density `K` for the mutation curve.
    #[serde(default)]
    pub(crate) mutation_half_density: Option<f64>,
    /// Genesis working-window size `A` (construction-time only).
    #[serde(default)]
    pub(crate) genesis_window: Option<u32>,
    /// Genesis fertility multiplier (construction-time only).
    #[serde(default)]
    pub(crate) genesis_fertility: Option<f64>,
    /// Code-metrics sampling cadence in ticks; `0`/omitted = disabled.
    #[serde(default)]
    pub(crate) metrics_every: Option<u32>,
    /// Optional program prefix overlaid on the origin cell's
    /// macro-genesis memory.
    #[serde(default)]
    pub(crate) program: Vec<u32>,
}

/// Full `config` payload — mirror of `ConfigMsg` in `src/protocol.ts`.
/// `coeff` / `k` always apply; every optional field updates only when
/// present (an explicit `0` still applies), matching the worker's
/// `applyConfig` reducer. Genesis knobs are deliberately absent — they
/// shape only the initial program, so they exist on `init` alone.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigMsg {
    /// Diffusion coefficient passed to `tick::step`.
    pub(crate) coeff: f64,
    /// CPU compute constant `k`.
    pub(crate) k: u32,
    /// See the matching [`InitMsg`] fields for knob semantics.
    #[serde(default)]
    pub(crate) move_threshold: Option<f32>,
    #[serde(default)]
    pub(crate) gravity: Option<f64>,
    #[serde(default)]
    pub(crate) gravity_alpha: Option<f64>,
    #[serde(default)]
    pub(crate) gravity_radius: Option<i32>,
    #[serde(default)]
    pub(crate) gravity_crit_mass: Option<f64>,
    #[serde(default)]
    pub(crate) pressure: Option<f64>,
    #[serde(default)]
    pub(crate) pressure_gamma: Option<f64>,
    #[serde(default)]
    pub(crate) pressure_eref: Option<f64>,
    #[serde(default)]
    pub(crate) mutation_strength: Option<f64>,
    #[serde(default)]
    pub(crate) mutation_half_density: Option<f64>,
    #[serde(default)]
    pub(crate) metrics_every: Option<u32>,
}

/// `runProgram` payload — mirror of `RunProgramMsg` in
/// `src/protocol.ts` (Project Pilgrim "Run Program"). The server picks
/// an eligible host via `aenternis_core::find_host` (the same
/// energy-weighted-periphery rule the worker applies in
/// `src/host-select.ts`) and `possess`es it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunProgramMsg {
    /// Program written over the host's leading slots.
    pub(crate) code: Vec<u32>,
    /// Extra slots the host must have beyond the program.
    pub(crate) reserve: u32,
    /// `origin_tag` stamped on the host (lineage marker).
    pub(crate) tag: u32,
    /// `appearance` stamped on the host (war-paint / color).
    pub(crate) appearance: u32,
}

/// Inbound control message from the viewer. Mirrors
/// `MainToWorkerMsg` in `src/protocol.ts`: identical `type` tags and
/// camelCase field names. The struct-shaped variants live as named
/// types above so the actor's `Command` enum can carry them without
/// re-listing every field.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ClientMessage {
    /// Reset the shared world. Affects every connected client \u{2014}
    /// this is a global reset.
    Init(InitMsg),
    /// Update tick-time parameters in place \u{2014} the world's state
    /// is not touched.
    Config(ConfigMsg),
    /// Resume (`true`) or pause (`false`) the autonomous tick loop.
    Running { running: bool },
    /// Single-step: advance the world by exactly one tick and emit
    /// one snapshot, regardless of the current `running` flag.
    Step,
    /// Request a full inspect of the cell at `(x, y, z)`. The reply
    /// is a binary cellDetail frame addressed back to the requesting
    /// client only.
    Inspect { x: i32, y: i32, z: i32 },
    /// Inject a program into the running world. The reply
    /// (`programStarted` / `programRejected`) is a JSON text frame
    /// addressed back to the requesting client only; the world change
    /// itself reaches everyone via the next snapshot broadcast.
    RunProgram(RunProgramMsg),
}

/// Outbound JSON control message to the viewer. Snapshot, cellDetail
/// and metrics are binary frames, encoded by the `encode_*` helpers
/// below.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ServerControl {
    /// Sent immediately after the WebSocket handshake completes.
    Ready,
    /// Late-join state for a freshly connected client. Only
    /// `running` is needed; the rest of the welcome state arrives
    /// implicitly with the next snapshot frame.
    Welcome { running: bool },
    /// Reply to a successful `runProgram`: the program was injected
    /// into the cell at `(x, y, z)`, stamped with `tag`. Mirrors
    /// `ProgramStartedMsg` in `src/protocol.ts`.
    ProgramStarted { x: i32, y: i32, z: i32, tag: u32 },
    /// Reply to a `runProgram` that could not be honored — no cell
    /// was large enough to host the program. Nothing was changed.
    /// Mirrors `ProgramRejectedMsg` in `src/protocol.ts`.
    ProgramRejected { reason: String },
}

/// Tag byte for a snapshot binary frame.
pub const SNAPSHOT_TAG: u8 = 1;

/// Tag byte for a cellDetail binary frame.
pub(crate) const CELL_DETAIL_TAG: u8 = 2;

/// Tag byte for a code-metrics binary frame.
pub(crate) const METRICS_TAG: u8 = 3;

/// Snapshot stride: number of `u32` fields per cell in the snapshot
/// payload.
///
/// Re-exported from the core so the layout has a single definition
/// shared with the WASM backend — JS callers see an identical payload
/// regardless of backend.
pub const SNAPSHOT_STRIDE: u32 = aenternis_core::snapshot::SNAPSHOT_STRIDE as u32;

/// `CellDetail` prefix length: number of `u32` fields in the fixed
/// header before the variable-length memory dump. Re-exported from the
/// core alongside [`SNAPSHOT_STRIDE`].
pub(crate) const INSPECT_PREFIX: u32 = aenternis_core::snapshot::INSPECT_PREFIX as u32;

/// All the inputs needed to encode a snapshot binary frame. Held by
/// reference so we don't copy the (potentially large) `snap` payload
/// just to hand it to the encoder.
pub struct SnapshotFrame<'a> {
    /// Tick counter as encoded into the binary header.
    pub tick: u32,
    /// Cell count for the header; must equal `snap.len() / SNAPSHOT_STRIDE`.
    pub cell_count: u32,
    /// Total energy summed across cells, for the header.
    pub total_energy: u32,
    /// Rolling-average per-tick wall time in milliseconds.
    pub ms_per_tick: f64,
    /// `[x_min, x_max, y_min, y_max, z_min, z_max]`. Empty world
    /// senders should fill with zeros; the JS side already handles
    /// the degenerate bbox case.
    pub bbox: [i32; 6],
    /// Flat cell payload, `cell_count * SNAPSHOT_STRIDE` `u32`s long.
    pub snap: &'a [u32],
}

impl SnapshotFrame<'_> {
    /// Length in bytes of the fixed-width header preceding the cell
    /// payload (tag + tick + `cell_count` + `total_energy` +
    /// `ms_per_tick` + stride + bbox6).
    pub const HEADER_LEN: usize = 1 + 4 + 4 + 4 + 8 + 4 + 6 * 4;
}

/// Encode a snapshot binary frame into a fresh `Vec<u8>`. Thin
/// convenience wrapper around [`encode_snapshot_frame_into`] for
/// call-sites that don't have a buffer to recycle (notably tests).
#[must_use]
pub fn encode_snapshot_frame(frame: &SnapshotFrame<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(SnapshotFrame::HEADER_LEN + frame.snap.len() * 4);
    encode_snapshot_frame_into(&mut out, frame);
    out
}

/// Encode a snapshot binary frame into `out`, clearing the buffer
/// first. Capacity is preserved across calls — the `WorldActor`
/// relies on this to amortize allocation across ticks.
///
/// No explicit `reserve` here: in the steady-state actor path the
/// buffer already has peak capacity from the previous tick, and on
/// the cold path `write_u32_slice_le` reaches the final length in
/// one `resize` call so at most one realloc happens regardless.
pub fn encode_snapshot_frame_into(out: &mut Vec<u8>, frame: &SnapshotFrame<'_>) {
    out.clear();
    out.push(SNAPSHOT_TAG);
    out.extend_from_slice(&frame.tick.to_le_bytes());
    out.extend_from_slice(&frame.cell_count.to_le_bytes());
    out.extend_from_slice(&frame.total_energy.to_le_bytes());
    out.extend_from_slice(&frame.ms_per_tick.to_le_bytes());
    out.extend_from_slice(&SNAPSHOT_STRIDE.to_le_bytes());
    for v in frame.bbox {
        out.extend_from_slice(&v.to_le_bytes());
    }
    write_u32_slice_le(out, frame.snap);
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

/// Encode a cellDetail binary frame into a fresh `Vec<u8>`. Thin
/// convenience wrapper around [`encode_cell_detail_frame_into`].
pub(crate) fn encode_cell_detail_frame(frame: &CellDetailFrame<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(CellDetailFrame::HEADER_LEN + frame.data.len() * 4);
    encode_cell_detail_frame_into(&mut out, frame);
    out
}

/// Encode a cellDetail binary frame into `out`, clearing the buffer
/// first. Capacity is preserved across calls; see the matching note
/// on [`encode_snapshot_frame_into`] for why no `reserve` is needed.
pub(crate) fn encode_cell_detail_frame_into(out: &mut Vec<u8>, frame: &CellDetailFrame<'_>) {
    out.clear();
    out.push(CELL_DETAIL_TAG);
    out.extend_from_slice(&frame.x.to_le_bytes());
    out.extend_from_slice(&frame.y.to_le_bytes());
    out.extend_from_slice(&frame.z.to_le_bytes());
    out.extend_from_slice(&frame.tick.to_le_bytes());
    out.extend_from_slice(&INSPECT_PREFIX.to_le_bytes());
    let data_len = u32::try_from(frame.data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&data_len.to_le_bytes());
    write_u32_slice_le(out, frame.data);
}

/// Encode a code-metrics binary frame into a fresh `Vec<u8>`.
/// `values` is the flat `CodeMetrics::to_flat` layout; metrics frames
/// are small (a few dozen `f64`s) and infrequent (every
/// `metricsEvery` ticks), so no buffer-recycling variant is needed.
pub(crate) fn encode_metrics_frame(tick: u32, values: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + 4 + values.len() * 8);
    out.push(METRICS_TAG);
    out.extend_from_slice(&tick.to_le_bytes());
    let count = u32::try_from(values.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Append `slice` as little-endian bytes to `out`. `resize` reserves
/// the full output region in one step, then a tight indexed loop
/// stores four bytes per element via `copy_from_slice` — the compiler
/// vectorizes that into SIMD stores on every target we care about,
/// and the workspace's `unsafe_code = "forbid"` keeps us in safe Rust
/// throughout.
fn write_u32_slice_le(out: &mut Vec<u8>, slice: &[u32]) {
    let start = out.len();
    out.resize(start + slice.len() * 4, 0);
    for (i, &v) in slice.iter().enumerate() {
        let off = start + i * 4;
        out[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        encode_cell_detail_frame, encode_cell_detail_frame_into, encode_metrics_frame,
        encode_snapshot_frame, encode_snapshot_frame_into, CellDetailFrame, ClientMessage,
        ServerControl, SnapshotFrame, CELL_DETAIL_TAG, INSPECT_PREFIX, METRICS_TAG,
        SNAPSHOT_STRIDE, SNAPSHOT_TAG,
    };

    // -- JSON layer -----------------------------------------------------------

    /// Shared wire fixture — the canonical JSON forms of every control
    /// message, checked byte-for-byte on the TS side too
    /// (`tests/protocol-wire-parity.test.ts`). Editing `src/protocol.ts`
    /// without mirroring the change here fails one side or the other,
    /// which is the whole point: the two backends can no longer drift
    /// silently.
    const WIRE_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/wire-messages.json"
    ));

    fn fixture_section(section: &str) -> Vec<serde_json::Value> {
        let root: serde_json::Value = serde_json::from_str(WIRE_FIXTURE).expect("fixture parses");
        root.get(section)
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("fixture has array section {section:?}"))
            .clone()
    }

    #[test]
    fn every_client_fixture_message_round_trips_losslessly() {
        // Round-trip = deserialize into `ClientMessage`, serialize back,
        // compare `Value`s. Serde ignores unknown JSON fields by default,
        // so a field the viewer sends but this enum lacks would vanish in
        // the re-serialization and fail the equality — exactly the silent
        // parameter drop this test exists to catch.
        let entries = fixture_section("clientToServer");
        assert_eq!(entries.len(), 6, "one fixture entry per MainToWorkerMsg");
        for entry in entries {
            let msg: ClientMessage = serde_json::from_value(entry.clone())
                .unwrap_or_else(|e| panic!("fixture entry must parse: {e}\n{entry}"));
            let back = serde_json::to_value(&msg).expect("ClientMessage serializes");
            assert_eq!(back, entry, "lossless round-trip for {entry}");
        }
    }

    #[test]
    fn every_server_fixture_message_matches_serialization() {
        // The outbound direction: each fixture entry must be exactly what
        // `ServerControl` serializes to, so the TS decoder (which parses
        // the same fixture) stays in lock-step with the server encoder.
        let entries = fixture_section("serverToClient");
        assert_eq!(entries.len(), 4, "one fixture entry per JSON server msg");
        for entry in entries {
            let control = match entry["type"].as_str().expect("type tag") {
                "ready" => ServerControl::Ready,
                "welcome" => ServerControl::Welcome {
                    running: entry["running"].as_bool().expect("running"),
                },
                "programStarted" => ServerControl::ProgramStarted {
                    x: i32::try_from(entry["x"].as_i64().expect("x")).unwrap(),
                    y: i32::try_from(entry["y"].as_i64().expect("y")).unwrap(),
                    z: i32::try_from(entry["z"].as_i64().expect("z")).unwrap(),
                    tag: u32::try_from(entry["tag"].as_u64().expect("tag")).unwrap(),
                },
                "programRejected" => ServerControl::ProgramRejected {
                    reason: entry["reason"].as_str().expect("reason").to_owned(),
                },
                other => panic!("unknown serverToClient fixture type {other:?}"),
            };
            let json = serde_json::to_value(&control).expect("ServerControl serializes");
            assert_eq!(json, entry);
        }
    }

    #[test]
    fn parse_init_full() {
        // The full-fields init lives in the shared fixture; this test
        // pins a handful of representative values by exact bits.
        let entries = fixture_section("clientToServer");
        let init = entries
            .iter()
            .find(|e| e["type"] == "init")
            .expect("fixture has an init entry");
        let msg: ClientMessage = serde_json::from_value(init.clone()).unwrap();
        match msg {
            ClientMessage::Init(init) => {
                assert_eq!(init.seed, 1234);
                assert_eq!(init.energy, 1_000_000);
                assert_eq!(init.coeff.to_bits(), 0.5_f64.to_bits());
                assert_eq!(init.k, 2);
                let mt = init.move_threshold.expect("moveThreshold parses to Some");
                assert_eq!(mt.to_bits(), 1.5_f32.to_bits());
                let gravity = init.gravity.expect("gravity parses to Some");
                assert_eq!(gravity.to_bits(), 1.5_f64.to_bits());
                assert_eq!(init.gravity_radius, Some(4));
                assert_eq!(init.genesis_window, Some(512));
                assert_eq!(init.metrics_every, Some(25));
                assert_eq!(init.program, vec![1, 2, 3]);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_init_minimal_omits_optionals() {
        let json = r#"{"type":"init","seed":1,"energy":5,"coeff":0.2,"k":2}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Init(init) => {
                assert_eq!(init.move_threshold, None);
                assert_eq!(init.gravity, None);
                assert_eq!(init.gravity_alpha, None);
                assert_eq!(init.gravity_radius, None);
                assert_eq!(init.pressure, None);
                assert_eq!(init.pressure_gamma, None);
                assert_eq!(init.pressure_eref, None);
                assert_eq!(init.mutation_strength, None);
                assert_eq!(init.mutation_half_density, None);
                assert_eq!(init.genesis_window, None);
                assert_eq!(init.genesis_fertility, None);
                assert_eq!(init.metrics_every, None);
                assert!(init.program.is_empty());
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_config() {
        // Exact-binary float values so `to_bits` comparison works.
        let json = r#"{"type":"config","coeff":0.25,"k":2,"moveThreshold":2.5,"gravity":1.5}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Config(cfg) => {
                assert_eq!(cfg.coeff.to_bits(), 0.25_f64.to_bits());
                assert_eq!(cfg.k, 2);
                let mt = cfg.move_threshold.expect("moveThreshold parses to Some");
                assert_eq!(mt.to_bits(), 2.5_f32.to_bits());
                let gravity = cfg.gravity.expect("gravity parses to Some");
                assert_eq!(gravity.to_bits(), 1.5_f64.to_bits());
                assert_eq!(cfg.mutation_strength, None);
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_program() {
        let json =
            r#"{"type":"runProgram","code":[7,8,9],"reserve":16,"tag":4242,"appearance":99}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::RunProgram(rp) => {
                assert_eq!(rp.code, vec![7, 8, 9]);
                assert_eq!(rp.reserve, 16);
                assert_eq!(rp.tag, 4242);
                assert_eq!(rp.appearance, 99);
            }
            other => panic!("expected RunProgram, got {other:?}"),
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
    fn encode_snapshot_frame_into_buffer_reuse() {
        // First encode produces a known-good baseline; second encode
        // into the same buffer must overwrite it, not append, and the
        // resulting bytes must match a fresh encode of the second
        // frame. The retained capacity is the whole point of the
        // _into form — the WorldActor relies on it per tick.
        let snap_a: Vec<u32> = vec![10, 20, 30, 40, 50, 60];
        let frame_a = SnapshotFrame {
            tick: 1,
            cell_count: 1,
            total_energy: 40,
            ms_per_tick: 1.0,
            bbox: [-1, 1, -1, 1, -1, 1],
            snap: &snap_a,
        };
        let snap_b: Vec<u32> = vec![
            1, 2, 3, 4, 5, 6, //
            7, 8, 9, 10, 11, 12,
        ];
        let frame_b = SnapshotFrame {
            tick: 999,
            cell_count: 2,
            total_energy: 22,
            ms_per_tick: 8.0,
            bbox: [0, 10, 0, 10, 0, 10],
            snap: &snap_b,
        };

        let mut buf = Vec::new();
        encode_snapshot_frame_into(&mut buf, &frame_a);
        let baseline_a = encode_snapshot_frame(&frame_a);
        assert_eq!(buf, baseline_a, "first encode must match fresh-Vec encode");

        let cap_after_a = buf.capacity();
        encode_snapshot_frame_into(&mut buf, &frame_b);
        let baseline_b = encode_snapshot_frame(&frame_b);
        assert_eq!(
            buf, baseline_b,
            "second encode must overwrite, not append, and match fresh-Vec encode"
        );
        assert_eq!(
            buf[0], SNAPSHOT_TAG,
            "second frame must start at byte 0 (clear-then-fill semantics)"
        );
        assert!(
            buf.capacity() >= cap_after_a,
            "capacity must be retained across calls (got {}, was {})",
            buf.capacity(),
            cap_after_a
        );
    }

    #[test]
    fn encode_cell_detail_frame_into_buffer_reuse() {
        let data_a: Vec<u32> = (0..30).collect();
        let frame_a = CellDetailFrame {
            x: 1,
            y: 2,
            z: 3,
            tick: 4,
            data: &data_a,
        };
        let data_b: Vec<u32> = (100..110).collect();
        let frame_b = CellDetailFrame {
            x: -10,
            y: -20,
            z: -30,
            tick: 77,
            data: &data_b,
        };

        let mut buf = Vec::new();
        encode_cell_detail_frame_into(&mut buf, &frame_a);
        let baseline_a = encode_cell_detail_frame(&frame_a);
        assert_eq!(buf, baseline_a);

        let cap_after_a = buf.capacity();
        encode_cell_detail_frame_into(&mut buf, &frame_b);
        let baseline_b = encode_cell_detail_frame(&frame_b);
        assert_eq!(
            buf, baseline_b,
            "second encode must overwrite, not append, and match fresh-Vec encode"
        );
        assert_eq!(buf[0], CELL_DETAIL_TAG);
        assert!(buf.capacity() >= cap_after_a);
    }

    #[test]
    fn metrics_frame_round_trip() {
        // Flat layout `[cells, entropy, diversity, uniqueTypes, ...hist]`;
        // values chosen exact in f64 so bit-equality holds.
        let values = [3.0, 1.5, 0.25, 2.0, 7.0, 0.0, 42.0];
        let bytes = encode_metrics_frame(1234, &values);
        assert_eq!(bytes.len(), 1 + 4 + 4 + values.len() * 8);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), METRICS_TAG);
        assert_eq!(r.u32(), 1234);
        assert_eq!(r.u32(), u32::try_from(values.len()).unwrap());
        for &expected in &values {
            assert!((r.f64() - expected).abs() < f64::EPSILON);
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

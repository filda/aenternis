//! World actor: single owner of the shared [`SparseWorld`], driven
//! by viewer commands and emitting binary snapshot frames over a
//! broadcast channel.
//!
//! ## Architecture
//!
//! One tokio task owns the world; every WS connection task talks to
//! it through a [`Handle`]. Inbound commands arrive via an unbounded
//! mpsc channel, snapshots fan out via a `tokio::sync::broadcast` of
//! `Arc<[u8]>` (the encoded binary frame, refcounted across
//! connections — one heap allocation per snapshot, fan-out is a
//! pointer bump). Late-join welcome state lives behind a
//! `tokio::sync::watch` so a freshly connected client can read it
//! without sitting on the broadcast queue.
//!
//! Inspect requests carry a `oneshot::Sender` so the cellDetail frame
//! goes back to the requesting client only.

use std::sync::Arc;
use std::time::Instant;

use aenternis_core::{
    compute_metrics, find_host, snap_gamma, tick, Base, Coord, GenesisConfig, SparseWorld,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::protocol::{
    encode_cell_detail_frame, encode_metrics_frame, encode_snapshot_frame_into, CellDetailFrame,
    ConfigMsg, InitMsg, RunProgramMsg, ServerControl, SnapshotFrame,
};

/// Broadcast channel capacity for snapshot frames. Lagging receivers
/// just lose old snapshots, which is exactly what the viewer wants
/// (it only ever renders the latest). 64 is comfortable headroom for
/// tests that spin up the actor and then `recv` lazily; production
/// connections recv in a tight loop and never come close.
const SNAPSHOT_BROADCAST_CAP: usize = 64;

/// Smoothing factor for the `ms_per_tick` rolling average. Mirrors
/// `worker-state.ts` so JS/native paths report comparable numbers.
const TICK_MS_SMOOTHING: f64 = 0.85;

/// Cheaply-cloneable handle to the world actor. WS connection tasks
/// take a clone, send commands through it, subscribe to the snapshot
/// broadcast, and peek at welcome state for late-join.
#[derive(Clone)]
pub struct Handle {
    cmd_tx: mpsc::UnboundedSender<Command>,
    event_tx: broadcast::Sender<Arc<[u8]>>,
    welcome_rx: watch::Receiver<WelcomeState>,
}

impl Handle {
    /// Send a command to the actor. Errors only if the actor task
    /// has shut down (every other handle dropped + cmd channel
    /// closed).
    pub(crate) fn send_command(&self, cmd: Command) -> Result<(), mpsc::error::SendError<Command>> {
        self.cmd_tx.send(cmd)
    }

    /// New broadcast subscription for snapshot frames. Each
    /// connection takes its own; broadcast handles fan-out.
    pub(crate) fn subscribe_events(&self) -> broadcast::Receiver<Arc<[u8]>> {
        self.event_tx.subscribe()
    }

    /// Snapshot of the current welcome state. Cheap (no async, no
    /// channel traversal) — backed by a `watch` borrow.
    pub(crate) fn welcome_state(&self) -> WelcomeState {
        self.welcome_rx.borrow().clone()
    }
}

/// Welcome-state surface for fresh clients. Today it's just
/// `running`; the rest of the welcome state arrives with the next
/// snapshot frame the client receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WelcomeState {
    pub(crate) running: bool,
}

/// Inbound actor command. `Init`/`Config`/`Running`/`Step` are
/// shared global state changes — every connected client sees them
/// on the next snapshot. `Inspect` and `RunProgram` are per-client
/// requests whose replies travel back through the supplied oneshots.
pub(crate) enum Command {
    /// Reset the shared world with the full viewer payload. Boxed
    /// (like `Config`): the full message structs are the largest
    /// variants and would otherwise bloat every `Command` (and its
    /// `SendError`) to their size.
    Init(Box<InitMsg>),
    /// Update tick parameters in place; world state untouched.
    Config(Box<ConfigMsg>),
    /// Resume / pause the autonomous tick loop.
    Running { running: bool },
    /// Single-step + emit one snapshot regardless of `running`.
    Step,
    /// Build a cellDetail frame for `(x, y, z)` and send it back via
    /// the oneshot. The frame is empty when no cell exists at that
    /// coordinate, matching the JS contract.
    Inspect {
        x: i32,
        y: i32,
        z: i32,
        reply: oneshot::Sender<Vec<u8>>,
    },
    /// Inject a program (Project Pilgrim). The JSON reply text
    /// (`programStarted` / `programRejected`) goes back through the
    /// oneshot to the requesting client only; the possession itself
    /// reaches everyone via the next snapshot broadcast.
    RunProgram {
        msg: RunProgramMsg,
        reply: oneshot::Sender<String>,
    },
}

/// Runtime simulation parameters — a field-for-field mirror of
/// `WorkerSimState` (`src/worker-state.ts`), including its defaults,
/// so an `init`/`config` with omitted optional fields resolves to the
/// **same** effective world on both backends.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SimParams {
    pub(crate) coeff: f64,
    pub(crate) k: u32,
    pub(crate) move_threshold: f32,
    pub(crate) gravity: f64,
    pub(crate) gravity_alpha: f64,
    pub(crate) gravity_radius: i32,
    pub(crate) pressure: f64,
    pub(crate) pressure_gamma: f64,
    pub(crate) pressure_eref: f64,
    pub(crate) mutation_strength: f64,
    pub(crate) mutation_half_density: f64,
    /// Code-metrics sampling cadence in ticks; `0` = disabled.
    pub(crate) metrics_every: u32,
}

impl Default for SimParams {
    /// Mirrors `DEFAULT_STATE` in `src/worker-state.ts` (which in turn
    /// mirrors the `SparseWorld` frozen-baseline defaults, plus the
    /// legacy worker fall-backs for the tick params).
    fn default() -> Self {
        Self {
            coeff: 0.20,
            k: 1,
            move_threshold: SparseWorld::DEFAULT_MOVE_THRESHOLD,
            gravity: 0.0,
            gravity_alpha: 0.0,
            gravity_radius: 1,
            pressure: 0.0,
            pressure_gamma: SparseWorld::DEFAULT_PRESSURE_GAMMA,
            pressure_eref: SparseWorld::DEFAULT_PRESSURE_EREF,
            mutation_strength: 0.0,
            mutation_half_density: SparseWorld::DEFAULT_MUTATION_HALF_DENSITY,
            metrics_every: 0,
        }
    }
}

impl SimParams {
    /// Reducer for `init` — mirrors `stateFromInit` in
    /// `src/worker-state.ts`: every omitted optional field falls back
    /// to the default (NOT to the previous value; an init is a reset).
    fn from_init(msg: &InitMsg) -> Self {
        let d = Self::default();
        Self {
            coeff: msg.coeff,
            k: msg.k,
            move_threshold: msg.move_threshold.unwrap_or(d.move_threshold),
            gravity: msg.gravity.unwrap_or(d.gravity),
            gravity_alpha: msg.gravity_alpha.unwrap_or(d.gravity_alpha),
            gravity_radius: msg.gravity_radius.unwrap_or(d.gravity_radius),
            pressure: msg.pressure.unwrap_or(d.pressure),
            pressure_gamma: msg.pressure_gamma.unwrap_or(d.pressure_gamma),
            pressure_eref: msg.pressure_eref.unwrap_or(d.pressure_eref),
            mutation_strength: msg.mutation_strength.unwrap_or(d.mutation_strength),
            mutation_half_density: msg.mutation_half_density.unwrap_or(d.mutation_half_density),
            metrics_every: msg.metrics_every.unwrap_or(d.metrics_every),
        }
    }

    /// Reducer for `config` — mirrors `applyConfig` in
    /// `src/worker-state.ts`: `coeff` / `k` always apply; every other
    /// field updates only when present on the message.
    const fn apply_config(&mut self, msg: &ConfigMsg) {
        self.coeff = msg.coeff;
        self.k = msg.k;
        if let Some(v) = msg.move_threshold {
            self.move_threshold = v;
        }
        if let Some(v) = msg.gravity {
            self.gravity = v;
        }
        if let Some(v) = msg.gravity_alpha {
            self.gravity_alpha = v;
        }
        if let Some(v) = msg.gravity_radius {
            self.gravity_radius = v;
        }
        if let Some(v) = msg.pressure {
            self.pressure = v;
        }
        if let Some(v) = msg.pressure_gamma {
            self.pressure_gamma = v;
        }
        if let Some(v) = msg.pressure_eref {
            self.pressure_eref = v;
        }
        if let Some(v) = msg.mutation_strength {
            self.mutation_strength = v;
        }
        if let Some(v) = msg.mutation_half_density {
            self.mutation_half_density = v;
        }
        if let Some(v) = msg.metrics_every {
            self.metrics_every = v;
        }
    }

    /// Push every physics knob onto the world — the counterpart of the
    /// WASM setter sequence the worker applies (`applySimConfig` /
    /// `applyStateToWorld`). γ snaps through the same core
    /// [`snap_gamma`] the WASM `setPressureGamma` boundary uses.
    fn apply_to(&self, world: &mut SparseWorld) {
        world.move_threshold = self.move_threshold;
        world.gravity = self.gravity;
        world.gravity_alpha = self.gravity_alpha;
        world.gravity_radius = self.gravity_radius;
        world.pressure = self.pressure;
        world.pressure_gamma = snap_gamma(self.pressure_gamma);
        world.pressure_eref = self.pressure_eref;
        world.mutation_strength = self.mutation_strength;
        world.mutation_half_density = self.mutation_half_density;
    }
}

/// Spawn the world actor with default config and return a handle for
/// connection tasks. The actor task runs until the last [`Handle`]
/// (or rather its underlying `cmd_tx`) is dropped.
#[must_use]
pub fn spawn() -> Handle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, _drop_initial_rx) = broadcast::channel(SNAPSHOT_BROADCAST_CAP);
    let (welcome_tx, welcome_rx) = watch::channel(WelcomeState { running: false });

    let actor = WorldActor {
        world: SparseWorld::new(0),
        params: SimParams::default(),
        running: false,
        tick_ms_avg: 0.0,
        cmd_rx,
        event_tx: event_tx.clone(),
        welcome_tx,
        snapshot_buf: Vec::new(),
        encoded_buf: Vec::new(),
    };

    tokio::spawn(actor.run());

    Handle {
        cmd_tx,
        event_tx,
        welcome_rx,
    }
}

struct WorldActor {
    world: SparseWorld,
    params: SimParams,
    running: bool,
    tick_ms_avg: f64,
    cmd_rx: mpsc::UnboundedReceiver<Command>,
    event_tx: broadcast::Sender<Arc<[u8]>>,
    welcome_tx: watch::Sender<WelcomeState>,
    /// Sort-and-pack scratch buffer, reused across every snapshot
    /// broadcast. Capacity grows monotonically with peak cell count;
    /// `clear()` between ticks does not release capacity.
    snapshot_buf: Vec<u32>,
    /// Binary-encoded snapshot frame scratch buffer, reused across
    /// every broadcast. Same capacity-growth contract as
    /// `snapshot_buf`.
    encoded_buf: Vec<u8>,
}

impl WorldActor {
    async fn run(mut self) {
        loop {
            if self.running {
                self.step_and_broadcast();
                // Drain any commands queued during the tick
                // without blocking. If the channel is closed,
                // shut down cleanly.
                loop {
                    match self.cmd_rx.try_recv() {
                        Ok(cmd) => {
                            if !self.handle_command(cmd) {
                                return;
                            }
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => return,
                    }
                }
                // Yield so other tasks (WS handlers, signal) can run
                // between ticks. Without this the tick loop monopolizes
                // the worker thread.
                tokio::task::yield_now().await;
            } else {
                let Some(cmd) = self.cmd_rx.recv().await else {
                    return;
                };
                if !self.handle_command(cmd) {
                    return;
                }
            }
        }
    }

    /// Returns `false` to request shutdown.
    fn handle_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Init(init) => {
                let init = *init;
                // Macro-genesis base with the viewer's program overlaid —
                // the exact constructor the WASM worker uses
                // (`World::newWithProgram`), so both backends build a
                // bit-identical initial world from the same init message.
                let defaults = GenesisConfig::default();
                let genesis = GenesisConfig {
                    window: init.genesis_window.unwrap_or(defaults.window),
                    fertility: init.genesis_fertility.unwrap_or(defaults.fertility),
                };
                self.world = SparseWorld::big_bang_with_config(
                    u64::from(init.seed),
                    init.energy,
                    Base::Macros,
                    &init.program,
                    &genesis,
                );
                self.params = SimParams::from_init(&init);
                self.params.apply_to(&mut self.world);
                self.tick_ms_avg = 0.0;
                self.broadcast_snapshot();
            }
            Command::Config(cfg) => {
                self.params.apply_config(&cfg);
                self.params.apply_to(&mut self.world);
            }
            Command::Running { running } => {
                self.running = running;
                self.publish_welcome();
            }
            Command::Step => {
                self.step_and_broadcast();
            }
            Command::Inspect { x, y, z, reply } => {
                let frame = self.build_inspect_frame(x, y, z);
                let _ = reply.send(frame);
            }
            Command::RunProgram { msg, reply } => {
                let response = self.run_program(&msg);
                let _ = reply.send(response);
            }
        }
        true
    }

    /// Advance one tick and emit the snapshot (plus a metrics frame
    /// when the sampling cadence lands on this tick). Shared between
    /// the autonomous run loop and the on-demand `Step` command so
    /// single-step and run-mode behave identically — mirrors
    /// `stepOnce` in `src/worker-handler.ts`.
    fn step_and_broadcast(&mut self) {
        let start = Instant::now();
        tick::step(&mut self.world, self.params.coeff, self.params.k);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        // Equivalent to `a * old + (1 - a) * elapsed`, but `mul_add` is
        // both faster (single FMA) and slightly more accurate.
        self.tick_ms_avg =
            TICK_MS_SMOOTHING.mul_add(self.tick_ms_avg, (1.0 - TICK_MS_SMOOTHING) * elapsed_ms);
        self.broadcast_snapshot();
        self.broadcast_metrics_if_due();
    }

    /// Inject a program into the shared world (Project Pilgrim) and
    /// build the JSON reply for the requesting client. Mirrors
    /// `runProgram` in `src/worker-handler.ts`: same host-selection
    /// rule (via the shared core `find_host`), same rejection reason
    /// text, and a fresh snapshot broadcast on success so the
    /// possession is visible immediately.
    fn run_program(&mut self, msg: &RunProgramMsg) -> String {
        aenternis_core::snapshot::snapshot_into(&self.world, &mut self.snapshot_buf);
        let Some(host) = find_host(&self.snapshot_buf, msg.code.len(), msg.reserve) else {
            let need = msg.code.len() as u64 + u64::from(msg.reserve);
            let rejected = ServerControl::ProgramRejected {
                reason: format!("no host cell with energy >= {need}"),
            };
            return serde_json::to_string(&rejected).expect("ProgramRejected always serializes");
        };
        // `find_host` guarantees the host exists with energy >=
        // code.len(), so possession cannot fail (same invariant the
        // worker relies on to skip its try/catch).
        self.world
            .possess(host, &msg.code, msg.tag, msg.appearance)
            .expect("find_host returned an eligible host");
        let started = ServerControl::ProgramStarted {
            x: host.x,
            y: host.y,
            z: host.z,
            tag: msg.tag,
        };
        let reply = serde_json::to_string(&started).expect("ProgramStarted always serializes");
        self.broadcast_snapshot();
        reply
    }

    /// Broadcast a binary metrics frame when sampling is enabled and
    /// the current tick lands on the cadence — the same
    /// `metricsEvery > 0 && tick % metricsEvery == 0` rule as the
    /// worker. `compute_metrics` is an `O(total_energy)` walk, so it
    /// only runs when a frame will actually be sent.
    fn broadcast_metrics_if_due(&self) {
        if self.params.metrics_every == 0
            || self.world.tick % u64::from(self.params.metrics_every) != 0
            || self.event_tx.receiver_count() == 0
        {
            return;
        }
        let flat = compute_metrics(&self.world).to_flat();
        let tick_u32 = u32::try_from(self.world.tick).unwrap_or(u32::MAX);
        let bytes: Arc<[u8]> = Arc::from(encode_metrics_frame(tick_u32, &flat).as_slice());
        let _ = self.event_tx.send(bytes);
    }

    fn broadcast_snapshot(&mut self) {
        // No subscribers → skip the sort+pack+encode entirely. A
        // future fresh subscriber sees the next live snapshot; pause/
        // resume bookkeeping is independent of broadcast traffic.
        if self.event_tx.receiver_count() == 0 {
            return;
        }
        // Snapshot payload layout lives in the core, shared with the
        // WASM backend so both emit byte-identical cells. The frame
        // header below is server-only (the WASM path hands JS the raw
        // `Uint32Array` in-process, with no header).
        aenternis_core::snapshot::snapshot_into(&self.world, &mut self.snapshot_buf);
        let cell_count = u32::try_from(self.world.len()).unwrap_or(u32::MAX);
        let total_energy = u32::try_from(self.world.total_energy()).unwrap_or(u32::MAX);
        // Indexed access avoids `clippy::tuple_array_conversions`,
        // which doesn't have a corresponding `From` impl in std for
        // 6-tuples.
        let bbox = self
            .world
            .bounding_box()
            .map_or([0_i32; 6], |b| [b.0, b.1, b.2, b.3, b.4, b.5]);
        let tick_u32 = u32::try_from(self.world.tick).unwrap_or(u32::MAX);

        let frame = SnapshotFrame {
            tick: tick_u32,
            cell_count,
            total_energy,
            ms_per_tick: self.tick_ms_avg,
            bbox,
            snap: &self.snapshot_buf,
        };
        encode_snapshot_frame_into(&mut self.encoded_buf, &frame);
        // `Arc::from(&[u8])` copies the bytes into a single
        // refcounted allocation — one heap block per snapshot rather
        // than two (Vec + Arc header) as the previous `Arc<Vec<u8>>`
        // form required.
        let bytes: Arc<[u8]> = Arc::from(self.encoded_buf.as_slice());
        // `send` returns Err only when there are zero subscribers
        // (race: last receiver dropped between our check and here);
        // we tolerate that — the snapshot just has no audience.
        let _ = self.event_tx.send(bytes);
    }

    fn build_inspect_frame(&self, x: i32, y: i32, z: i32) -> Vec<u8> {
        let tick_u32 = u32::try_from(self.world.tick).unwrap_or(u32::MAX);
        // Detail payload layout lives in the core, shared with the
        // WASM backend. Inspect is a rare per-click path, so a fresh
        // `Vec` per request (rather than a persistent scratch buffer)
        // is fine.
        let mut data = Vec::new();
        aenternis_core::snapshot::inspect_into(&self.world, Coord::new(x, y, z), &mut data);
        let frame = CellDetailFrame {
            x,
            y,
            z,
            tick: tick_u32,
            data: &data,
        };
        encode_cell_detail_frame(&frame)
    }

    fn publish_welcome(&self) {
        let _ = self.welcome_tx.send(WelcomeState {
            running: self.running,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{spawn, Command, SimParams, WelcomeState};
    use crate::protocol::{
        ConfigMsg, InitMsg, RunProgramMsg, CELL_DETAIL_TAG, INSPECT_PREFIX, METRICS_TAG,
        SNAPSHOT_STRIDE, SNAPSHOT_TAG,
    };
    use aenternis_core::{tick, Base, GenesisConfig, SparseWorld};
    use std::time::Duration;
    use tokio::sync::{broadcast, oneshot};

    fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    /// Receive the next snapshot, tolerating any `Lagged` events
    /// (which are fine — for tests we just want the next live frame).
    async fn next_event(rx: &mut broadcast::Receiver<std::sync::Arc<[u8]>>) -> Vec<u8> {
        loop {
            let recv = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("timed out waiting for event");
            match recv {
                Ok(arc) => return arc.to_vec(),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("event channel closed"),
            }
        }
    }

    /// A minimal `InitMsg` (every optional field omitted), the wire
    /// equivalent of `{"type":"init","seed":…,"energy":…,"coeff":0.15,"k":1}`.
    fn init_msg(seed: u32, energy: u32) -> InitMsg {
        InitMsg {
            seed,
            energy,
            coeff: 0.15,
            k: 1,
            move_threshold: None,
            gravity: None,
            gravity_alpha: None,
            gravity_radius: None,
            pressure: None,
            pressure_gamma: None,
            pressure_eref: None,
            mutation_strength: None,
            mutation_half_density: None,
            genesis_window: None,
            genesis_fertility: None,
            metrics_every: None,
            program: vec![],
        }
    }

    fn init_command(seed: u32, energy: u32) -> Command {
        Command::Init(Box::new(init_msg(seed, energy)))
    }

    #[tokio::test]
    async fn init_emits_snapshot_at_tick_zero() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(1234, 100)).unwrap();
        let frame = next_event(&mut events).await;

        assert_eq!(frame[0], SNAPSHOT_TAG);
        assert_eq!(read_u32_le(&frame, 1), 0, "tick should be 0 after init");
    }

    #[tokio::test]
    async fn step_advances_one_tick() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(1, 100)).unwrap();
        let init = next_event(&mut events).await;
        assert_eq!(read_u32_le(&init, 1), 0);

        handle.send_command(Command::Step).unwrap();
        let stepped = next_event(&mut events).await;
        assert_eq!(read_u32_le(&stepped, 1), 1, "Step should advance tick by 1");
    }

    #[tokio::test]
    async fn running_drives_autonomous_ticks() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(1, 100)).unwrap();
        let _init = next_event(&mut events).await;

        handle
            .send_command(Command::Running { running: true })
            .unwrap();

        // Wait until we see at least 3 ticks elapse — the world is
        // small so ticks fly by.
        let mut last_tick: u32 = 0;
        for _ in 0..30 {
            let frame = next_event(&mut events).await;
            last_tick = read_u32_le(&frame, 1);
            if last_tick >= 3 {
                break;
            }
        }
        assert!(
            last_tick >= 3,
            "expected >= 3 autonomous ticks, got {last_tick}"
        );

        // Stop, then verify no further snapshots arrive in a small
        // window.
        handle
            .send_command(Command::Running { running: false })
            .unwrap();
        // Drain any in-flight broadcasts queued before the stop.
        while tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_ok()
        {}
        let after_stop = tokio::time::timeout(Duration::from_millis(150), events.recv()).await;
        assert!(
            after_stop.is_err(),
            "expected silence after pausing, got an event"
        );
    }

    #[tokio::test]
    async fn inspect_returns_cell_detail_frame_for_present_cell() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(1, 100)).unwrap();
        let _ = next_event(&mut events).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        handle
            .send_command(Command::Inspect {
                x: 0,
                y: 0,
                z: 0,
                reply: reply_tx,
            })
            .unwrap();

        let frame = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(frame[0], CELL_DETAIL_TAG);
        // Header layout: tag(1) + x(4) + y(4) + z(4) + tick(4) +
        // prefix(4) + dataLen(4) = 25 bytes before payload.
        assert_eq!(read_u32_le(&frame, 17), INSPECT_PREFIX);
        let data_len = read_u32_le(&frame, 21);
        // Origin cell exists with energy=100 → memory of 100 slots
        // → data length = 28 prefix + 100 memory = 128.
        assert_eq!(data_len, 128);
    }

    #[tokio::test]
    async fn inspect_returns_empty_for_absent_cell() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(1, 100)).unwrap();
        let _ = next_event(&mut events).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        handle
            .send_command(Command::Inspect {
                x: 99_999,
                y: 0,
                z: 0,
                reply: reply_tx,
            })
            .unwrap();

        let frame = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(frame[0], CELL_DETAIL_TAG);
        assert_eq!(read_u32_le(&frame, 21), 0, "absent cell → empty data");
    }

    #[tokio::test]
    async fn welcome_state_tracks_running_flag() {
        let handle = spawn();

        assert_eq!(
            handle.welcome_state(),
            WelcomeState { running: false },
            "default welcome state is paused"
        );

        let mut events = handle.subscribe_events();
        handle.send_command(init_command(1, 50)).unwrap();
        let _ = next_event(&mut events).await;

        handle
            .send_command(Command::Running { running: true })
            .unwrap();

        // Welcome state propagates through the watch channel; give
        // the actor a moment to publish.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(handle.welcome_state(), WelcomeState { running: true });

        handle
            .send_command(Command::Running { running: false })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(handle.welcome_state(), WelcomeState { running: false });
    }

    #[tokio::test]
    async fn snapshot_frame_carries_stride_and_bbox() {
        let handle = spawn();
        let mut events = handle.subscribe_events();

        handle.send_command(init_command(42, 50)).unwrap();
        let frame = next_event(&mut events).await;

        // Header layout: tag(1) + tick(4) + cellCount(4) +
        // totalEnergy(4) + msPerTick(8) + stride(4) = byte 21 starts
        // the bbox; stride sits at offset 21 - 4 = 17. Wait — tick
        // is u32, msPerTick is f64. Recount:
        //   tag       0..1   (1)
        //   tick      1..5   (4)
        //   cellCount 5..9   (4)
        //   totalEng  9..13  (4)
        //   msPerTick 13..21 (8)
        //   stride    21..25 (4)
        //   bbox      25..49 (24)
        assert_eq!(read_u32_le(&frame, 21), SNAPSHOT_STRIDE);
        let cell_count = read_u32_le(&frame, 5);
        assert!(cell_count >= 1, "big_bang must produce >= 1 cell");
    }

    // -- SimParams reducers (mirror src/worker-state.ts) ----------------------

    #[test]
    fn from_init_falls_back_to_worker_defaults() {
        let params = SimParams::from_init(&init_msg(1, 10));
        let expected = SimParams {
            coeff: 0.15,
            k: 1,
            ..SimParams::default()
        };
        assert_eq!(params, expected, "omitted optionals resolve to defaults");
    }

    #[test]
    fn from_init_takes_every_provided_knob() {
        let msg = InitMsg {
            move_threshold: Some(1.0),
            gravity: Some(1.5),
            gravity_alpha: Some(0.05),
            gravity_radius: Some(4),
            pressure: Some(0.2),
            pressure_gamma: Some(3.0),
            pressure_eref: Some(50_000.0),
            mutation_strength: Some(0.75),
            mutation_half_density: Some(30_000.0),
            metrics_every: Some(25),
            ..init_msg(1, 10)
        };
        let params = SimParams::from_init(&msg);
        let expected = SimParams {
            coeff: 0.15,
            k: 1,
            move_threshold: 1.0,
            gravity: 1.5,
            gravity_alpha: 0.05,
            gravity_radius: 4,
            pressure: 0.2,
            pressure_gamma: 3.0,
            pressure_eref: 50_000.0,
            mutation_strength: 0.75,
            mutation_half_density: 30_000.0,
            metrics_every: 25,
        };
        assert_eq!(params, expected);
    }

    #[test]
    fn apply_config_updates_present_fields_and_keeps_the_rest() {
        let mut params = SimParams {
            gravity: 1.5,
            mutation_strength: 0.75,
            ..SimParams::default()
        };
        params.apply_config(&ConfigMsg {
            coeff: 0.3,
            k: 2,
            move_threshold: None,
            gravity: Some(0.0), // explicit 0 still applies
            gravity_alpha: None,
            gravity_radius: Some(2),
            pressure: None,
            pressure_gamma: None,
            pressure_eref: None,
            mutation_strength: None,
            mutation_half_density: None,
            metrics_every: Some(10),
        });
        let expected = SimParams {
            coeff: 0.3,
            k: 2,
            gravity: 0.0,
            gravity_radius: 2,
            mutation_strength: 0.75, // absent → kept
            metrics_every: 10,
            ..SimParams::default()
        };
        assert_eq!(params, expected);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact assignments and snap targets
    fn apply_to_pushes_every_knob_and_snaps_gamma() {
        let params = SimParams {
            move_threshold: 1.0,
            gravity: 1.5,
            gravity_alpha: 0.05,
            gravity_radius: 4,
            pressure: 0.2,
            pressure_gamma: 2.3, // snaps to 2.5
            pressure_eref: 50_000.0,
            mutation_strength: 0.75,
            mutation_half_density: 30_000.0,
            ..SimParams::default()
        };
        let mut world = SparseWorld::new(0);
        params.apply_to(&mut world);
        assert_eq!(world.move_threshold, 1.0);
        assert_eq!(world.gravity, 1.5);
        assert_eq!(world.gravity_alpha, 0.05);
        assert_eq!(world.gravity_radius, 4);
        assert_eq!(world.pressure, 0.2);
        assert_eq!(world.pressure_gamma, 2.5, "γ must snap through snap_gamma");
        assert_eq!(world.pressure_eref, 50_000.0);
        assert_eq!(world.mutation_strength, 0.75);
        assert_eq!(world.mutation_half_density, 30_000.0);
    }

    // -- WASM-worker parity through the wire -----------------------------------

    #[tokio::test]
    async fn init_builds_the_macro_genesis_world() {
        // The worker constructs via `World::newWithProgram` = macro
        // genesis base + program overlay. The origin cell's *memory*
        // (not visible in a snapshot) is where a Noise-base regression
        // would show, so compare the inspect payload against a world
        // built by the exact core constructor the WASM path uses.
        let (seed, energy) = (7, 50);
        let program = vec![11, 22, 33];
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle
            .send_command(Command::Init(Box::new(InitMsg {
                program: program.clone(),
                ..init_msg(seed, energy)
            })))
            .unwrap();
        let _ = next_event(&mut events).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        handle
            .send_command(Command::Inspect {
                x: 0,
                y: 0,
                z: 0,
                reply: reply_tx,
            })
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .unwrap()
            .unwrap();

        let reference = SparseWorld::big_bang_with_config(
            u64::from(seed),
            energy,
            Base::Macros,
            &program,
            &GenesisConfig::default(),
        );
        let mut expected = Vec::new();
        aenternis_core::snapshot::inspect_into(
            &reference,
            aenternis_core::Coord::ORIGIN,
            &mut expected,
        );
        // CellDetail header is 25 bytes; payload follows.
        let data_len = read_u32_le(&frame, 21) as usize;
        assert_eq!(data_len, expected.len());
        let payload: Vec<u32> = (0..data_len)
            .map(|i| read_u32_le(&frame, 25 + i * 4))
            .collect();
        assert_eq!(
            payload, expected,
            "origin memory must match the macro-genesis constructor"
        );
    }

    #[tokio::test]
    async fn init_applies_physics_knobs_end_to_end() {
        // Drive the actor with a gravity/pressure/mutation init and
        // step twice; a reference world built + stepped locally with
        // the same parameters must produce byte-identical snapshots.
        // This is the regression test for the parameter drop that let
        // the native backend silently run the all-off baseline.
        let (seed, energy) = (1234, 5000);
        let init = InitMsg {
            coeff: 0.15,
            move_threshold: Some(1.0),
            gravity: Some(1.5),
            gravity_alpha: Some(0.05),
            gravity_radius: Some(2),
            pressure: Some(0.2),
            pressure_gamma: Some(2.0),
            pressure_eref: Some(50_000.0),
            mutation_strength: Some(1.0),
            mutation_half_density: Some(40_000.0),
            ..init_msg(seed, energy)
        };
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle
            .send_command(Command::Init(Box::new(init.clone())))
            .unwrap();
        let _ = next_event(&mut events).await;
        handle.send_command(Command::Step).unwrap();
        let _ = next_event(&mut events).await;
        handle.send_command(Command::Step).unwrap();
        let frame = next_event(&mut events).await;
        assert_eq!(read_u32_le(&frame, 1), 2, "two steps → tick 2");

        let mut reference = SparseWorld::big_bang_with_config(
            u64::from(seed),
            energy,
            Base::Macros,
            &[],
            &GenesisConfig::default(),
        );
        reference.move_threshold = 1.0;
        reference.gravity = 1.5;
        reference.gravity_alpha = 0.05;
        reference.gravity_radius = 2;
        reference.pressure = 0.2;
        reference.pressure_gamma = 2.0;
        reference.pressure_eref = 50_000.0;
        reference.mutation_strength = 1.0;
        reference.mutation_half_density = 40_000.0;
        tick::step(&mut reference, 0.15, 1);
        tick::step(&mut reference, 0.15, 1);
        let mut expected = Vec::new();
        aenternis_core::snapshot::snapshot_into(&reference, &mut expected);

        // Snapshot header is 49 bytes; cell payload follows.
        let cell_count = read_u32_le(&frame, 5) as usize;
        assert_eq!(cell_count * SNAPSHOT_STRIDE as usize, expected.len());
        let payload: Vec<u32> = (0..expected.len())
            .map(|i| read_u32_le(&frame, 49 + i * 4))
            .collect();
        assert_eq!(
            payload, expected,
            "wire-driven world must match a locally-parameterized one"
        );
    }

    // -- Metrics broadcast -----------------------------------------------------

    #[tokio::test]
    async fn metrics_frame_follows_snapshot_on_cadence() {
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle
            .send_command(Command::Init(Box::new(InitMsg {
                metrics_every: Some(1),
                ..init_msg(1, 50)
            })))
            .unwrap();
        let init_frame = next_event(&mut events).await;
        assert_eq!(
            init_frame[0], SNAPSHOT_TAG,
            "no metrics at tick 0 — the worker samples in stepOnce only"
        );

        handle.send_command(Command::Step).unwrap();
        let snap = next_event(&mut events).await;
        assert_eq!(snap[0], SNAPSHOT_TAG);
        let metrics = next_event(&mut events).await;
        assert_eq!(metrics[0], METRICS_TAG);
        assert_eq!(read_u32_le(&metrics, 1), 1, "metrics tick");
        let count = read_u32_le(&metrics, 5) as usize;
        assert_eq!(count, 4 + aenternis_core::OPCODE_BINS);
        assert_eq!(metrics.len(), 9 + count * 8);
    }

    #[tokio::test]
    async fn no_metrics_frame_off_cadence() {
        // metricsEvery=2 → tick 1 must emit a snapshot and nothing
        // else. Pins the guard's `||` chain: corrupting either `||`
        // to `&&` lets an off-cadence tick fall through to a send.
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle
            .send_command(Command::Init(Box::new(InitMsg {
                metrics_every: Some(2),
                ..init_msg(1, 50)
            })))
            .unwrap();
        let _ = next_event(&mut events).await;
        handle.send_command(Command::Step).unwrap();
        let snap = next_event(&mut events).await;
        assert_eq!(snap[0], SNAPSHOT_TAG);
        let extra = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
        assert!(extra.is_err(), "tick 1 is off the every-2 cadence");

        // Tick 2 lands on the cadence — metrics must follow.
        handle.send_command(Command::Step).unwrap();
        let snap2 = next_event(&mut events).await;
        assert_eq!(snap2[0], SNAPSHOT_TAG);
        let metrics = next_event(&mut events).await;
        assert_eq!(metrics[0], METRICS_TAG);
        assert_eq!(read_u32_le(&metrics, 1), 2);
    }

    #[tokio::test]
    async fn commands_drained_mid_run_do_not_stop_the_actor() {
        // While the autonomous loop runs, commands are drained via
        // `try_recv` between ticks. A benign command handled there
        // must NOT shut the actor down (pins the `!` in the drain
        // loop's `if !self.handle_command(cmd) { return; }`).
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle.send_command(init_command(1, 50)).unwrap();
        let _ = next_event(&mut events).await;
        handle
            .send_command(Command::Running { running: true })
            .unwrap();
        let _ = next_event(&mut events).await;

        // Handled inside the drain loop because the loop is running.
        handle
            .send_command(Command::Config(Box::new(ConfigMsg {
                coeff: 0.2,
                k: 1,
                move_threshold: None,
                gravity: None,
                gravity_alpha: None,
                gravity_radius: None,
                pressure: None,
                pressure_gamma: None,
                pressure_eref: None,
                mutation_strength: None,
                mutation_half_density: None,
                metrics_every: None,
            })))
            .unwrap();

        // Ticks must keep flowing past the drained command.
        let before = read_u32_le(&next_event(&mut events).await, 1);
        let mut after = before;
        for _ in 0..50 {
            after = read_u32_le(&next_event(&mut events).await, 1);
            if after > before + 2 {
                break;
            }
        }
        assert!(
            after > before + 2,
            "actor must keep ticking after a mid-run command (before={before}, after={after})"
        );
    }

    #[tokio::test]
    async fn no_metrics_frame_when_cadence_disabled() {
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle.send_command(init_command(1, 50)).unwrap();
        let _ = next_event(&mut events).await;
        handle.send_command(Command::Step).unwrap();
        let snap = next_event(&mut events).await;
        assert_eq!(snap[0], SNAPSHOT_TAG);
        // Nothing else should arrive in a short window.
        let extra = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
        assert!(extra.is_err(), "metricsEvery=0 must emit no metrics frame");
    }

    // -- RunProgram (Project Pilgrim) -------------------------------------------

    async fn send_run_program(
        handle: &super::Handle,
        code: Vec<u32>,
        reserve: u32,
        tag: u32,
    ) -> String {
        let (reply_tx, reply_rx) = oneshot::channel();
        handle
            .send_command(Command::RunProgram {
                msg: RunProgramMsg {
                    code,
                    reserve,
                    tag,
                    appearance: 7,
                },
                reply: reply_tx,
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn run_program_possesses_a_host_and_broadcasts() {
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle.send_command(init_command(1, 100)).unwrap();
        let _ = next_event(&mut events).await;

        let reply = send_run_program(&handle, vec![1, 2, 3], 0, 4242).await;
        // Single-cell world → the origin is the only (eligible) host.
        assert_eq!(
            reply,
            r#"{"type":"programStarted","x":0,"y":0,"z":0,"tag":4242}"#
        );

        // Possession broadcasts a fresh snapshot; the host cell now
        // carries the pilgrim's origin_tag (snapshot field +4).
        let frame = next_event(&mut events).await;
        assert_eq!(frame[0], SNAPSHOT_TAG);
        assert_eq!(read_u32_le(&frame, 49 + 4 * 4), 4242, "origin_tag stamped");
    }

    #[tokio::test]
    async fn run_program_rejects_when_no_host_is_large_enough() {
        let handle = spawn();
        let mut events = handle.subscribe_events();
        handle.send_command(init_command(1, 10)).unwrap();
        let _ = next_event(&mut events).await;

        // need = 8 + 5 = 13 > 10 → rejected, worker-identical reason.
        let reply = send_run_program(&handle, vec![0; 8], 5, 1).await;
        assert_eq!(
            reply,
            r#"{"type":"programRejected","reason":"no host cell with energy >= 13"}"#
        );

        // Rejection must not broadcast a snapshot.
        let extra = tokio::time::timeout(Duration::from_millis(100), events.recv()).await;
        assert!(extra.is_err(), "rejected runProgram must not broadcast");
    }
}

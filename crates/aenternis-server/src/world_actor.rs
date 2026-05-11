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

use aenternis_core::{tick, Cell, Coord, SparseWorld};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::protocol::{
    encode_cell_detail_frame, encode_snapshot_frame_into, CellDetailFrame, SnapshotFrame,
    SNAPSHOT_STRIDE,
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
/// on the next snapshot. `Inspect` is a per-client request whose
/// reply travels back through the supplied oneshot.
pub(crate) enum Command {
    /// Reset the shared world. See [`crate::protocol::ClientMessage::Init`]
    /// for field semantics.
    Init {
        seed: u32,
        energy: u32,
        coeff: f64,
        k: u32,
        move_threshold: Option<f32>,
        program: Vec<u32>,
    },
    /// Update tick parameters in place; world state untouched.
    Config {
        coeff: f64,
        k: u32,
        move_threshold: Option<f32>,
    },
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
        coeff: 0.15,
        k: 1,
        move_threshold: SparseWorld::DEFAULT_MOVE_THRESHOLD,
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
    coeff: f64,
    k: u32,
    move_threshold: f32,
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
                self.tick_once();
                self.broadcast_snapshot();
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
            Command::Init {
                seed,
                energy,
                coeff,
                k,
                move_threshold,
                program,
            } => {
                self.world = if program.is_empty() {
                    SparseWorld::big_bang(u64::from(seed), energy)
                } else {
                    SparseWorld::big_bang_with_program(u64::from(seed), energy, &program)
                };
                self.coeff = coeff;
                self.k = k;
                self.move_threshold = move_threshold.unwrap_or(self.move_threshold);
                self.world.move_threshold = self.move_threshold;
                self.tick_ms_avg = 0.0;
                self.broadcast_snapshot();
            }
            Command::Config {
                coeff,
                k,
                move_threshold,
            } => {
                self.coeff = coeff;
                self.k = k;
                if let Some(mt) = move_threshold {
                    self.move_threshold = mt;
                    self.world.move_threshold = mt;
                }
            }
            Command::Running { running } => {
                self.running = running;
                self.publish_welcome();
            }
            Command::Step => {
                self.tick_once();
                self.broadcast_snapshot();
            }
            Command::Inspect { x, y, z, reply } => {
                let frame = self.build_inspect_frame(x, y, z);
                let _ = reply.send(frame);
            }
        }
        true
    }

    fn tick_once(&mut self) {
        let start = Instant::now();
        tick::step(&mut self.world, self.coeff, self.k);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        // Equivalent to `a * old + (1 - a) * elapsed`, but `mul_add` is
        // both faster (single FMA) and slightly more accurate.
        self.tick_ms_avg =
            TICK_MS_SMOOTHING.mul_add(self.tick_ms_avg, (1.0 - TICK_MS_SMOOTHING) * elapsed_ms);
    }

    fn broadcast_snapshot(&mut self) {
        // No subscribers → skip the sort+pack+encode entirely. A
        // future fresh subscriber sees the next live snapshot; pause/
        // resume bookkeeping is independent of broadcast traffic.
        if self.event_tx.receiver_count() == 0 {
            return;
        }
        build_snapshot_payload_into(&mut self.snapshot_buf, &self.world);
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
        let coord = Coord::new(x, y, z);
        let tick_u32 = u32::try_from(self.world.tick).unwrap_or(u32::MAX);
        let data = self
            .world
            .get(coord)
            .map(build_inspect_data)
            .unwrap_or_default();
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

/// Build the flat `Uint32Array`-style snapshot payload — one cell
/// per `[x, y, z, energy, origin_tag, appearance]` group, in
/// `(x, y, z)` lex order. Mirrors `aenternis-wasm::World::cells_snapshot`
/// bit-for-bit so the JS viewer treats both backends identically.
///
/// `out` is cleared first and re-filled in place. Capacity is
/// retained across calls — the `WorldActor` holds a persistent
/// `snapshot_buf` and amortizes the allocation away after the first
/// peak-sized world.
fn build_snapshot_payload_into(out: &mut Vec<u32>, world: &SparseWorld) {
    out.clear();
    out.reserve(world.len() * SNAPSHOT_STRIDE as usize);
    for (coord, cell) in world.sorted_iter() {
        out.push(coord.x as u32);
        out.push(coord.y as u32);
        out.push(coord.z as u32);
        out.push(cell.energy());
        out.push(cell.origin_tag);
        out.push(cell.appearance);
    }
}

/// Build the `[pc, energy, origin_tag, appearance, pointers×6,
/// rates×6, active_outflow×6, inflow×6, memory×E]` cellDetail
/// payload. Mirrors `aenternis-wasm::World::cell_inspect` so the
/// viewer's inspector panel parses both backends with the same
/// `prefix` constant.
fn build_inspect_data(cell: &Cell) -> Vec<u32> {
    let mut out = Vec::with_capacity(28 + cell.memory.len());
    out.push(cell.pc);
    out.push(cell.energy());
    out.push(cell.origin_tag);
    out.push(cell.appearance);
    out.extend_from_slice(&cell.pointers);
    out.extend_from_slice(&cell.rates);
    out.extend_from_slice(&cell.active_outflow);
    out.extend_from_slice(&cell.inflow);
    out.extend_from_slice(&cell.memory);
    out
}

#[cfg(test)]
mod tests {
    use super::{spawn, Command, WelcomeState};
    use crate::protocol::{CELL_DETAIL_TAG, INSPECT_PREFIX, SNAPSHOT_STRIDE, SNAPSHOT_TAG};
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

    fn init_command(seed: u32, energy: u32) -> Command {
        Command::Init {
            seed,
            energy,
            coeff: 0.15,
            k: 1,
            move_threshold: None,
            program: vec![],
        }
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
}

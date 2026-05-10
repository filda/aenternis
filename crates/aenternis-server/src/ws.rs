//! `/sim` WebSocket handler.
//!
//! Per-connection task. After the upgrade handshake:
//!
//! 1. Sends `Ready` (JSON text frame).
//! 2. Sends `Welcome { running }` reflecting the actor's current
//!    autonomous-tick state — late-join clients pick up the right
//!    Pause/Resume button label without waiting a tick.
//! 3. Subscribes to the actor's snapshot broadcast and forwards
//!    every frame as a binary WebSocket message.
//! 4. Reads JSON text frames from the client, parses them as
//!    [`ClientMessage`], translates to [`Command`], and routes
//!    through the actor.
//!
//! `Inspect` is per-client: the cellDetail reply travels back
//! through a oneshot, then onto this connection's sender only —
//! never the broadcast — so a click in tab A doesn't paint tab B's
//! inspector.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::protocol::{ClientMessage, ServerControl};
use crate::world_actor::{Command, Handle};

/// Axum route handler for `GET /sim`. Upgrades the connection to
/// WebSocket and hands it off to [`handle_socket`].
pub(crate) async fn ws_handler(
    State(handle): State<Handle>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, handle))
}

/// Per-connection driver. Returns when either side hangs up or the
/// snapshot broadcast closes.
async fn handle_socket(socket: WebSocket, world: Handle) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = world.subscribe_events();
    let (inspect_tx, mut inspect_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Bootstrap: ready, then welcome with the actor's current state.
    let ready_text = serde_json::to_string(&ServerControl::Ready).expect("Ready always serializes");
    if sender.send(Message::Text(ready_text)).await.is_err() {
        return;
    }
    let welcome = ServerControl::Welcome {
        running: world.welcome_state().running,
    };
    let welcome_text = serde_json::to_string(&welcome).expect("Welcome always serializes");
    if sender.send(Message::Text(welcome_text)).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(arc) => {
                    if sender.send(Message::Binary(arc.to_vec())).await.is_err() {
                        return;
                    }
                }
                // Lagging is fine — the viewer only ever renders the
                // latest frame, so a dropped intermediate snapshot is
                // invisible.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            inspect = inspect_rx.recv() => {
                let Some(frame) = inspect else { return; };
                if sender.send(Message::Binary(frame)).await.is_err() {
                    return;
                }
            }
            client = receiver.next() => match client {
                Some(Ok(Message::Text(text))) => {
                    // Malformed JSON or unknown variant — silently
                    // drop. The viewer never produces these in
                    // normal operation; logging would just spam.
                    if let Ok(parsed) = serde_json::from_str::<ClientMessage>(&text) {
                        handle_client_message(parsed, &world, &inspect_tx);
                    }
                }
                // Close, stream end, and underlying transport error
                // all mean the connection is gone.
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                // Binary, Ping, Pong: ignore. Axum handles ping/pong
                // automatically; binary from the client isn't part of
                // the protocol.
                Some(Ok(_)) => {}
            }
        }
    }
}

/// Translate a parsed [`ClientMessage`] into a [`Command`] and route
/// it to the actor. Inspect grows an extra oneshot listener task
/// that funnels the reply onto this connection's `inspect_tx`.
fn handle_client_message(
    msg: ClientMessage,
    world: &Handle,
    inspect_tx: &mpsc::UnboundedSender<Vec<u8>>,
) {
    let cmd = match msg {
        ClientMessage::Init {
            seed,
            energy,
            coeff,
            k,
            move_threshold,
            program,
        } => Command::Init {
            seed,
            energy,
            coeff,
            k,
            move_threshold,
            program,
        },
        ClientMessage::Config {
            coeff,
            k,
            move_threshold,
        } => Command::Config {
            coeff,
            k,
            move_threshold,
        },
        ClientMessage::Running { running } => Command::Running { running },
        ClientMessage::Step => Command::Step,
        ClientMessage::Inspect { x, y, z } => {
            let (reply_tx, reply_rx) = oneshot::channel();
            let inspect_tx = inspect_tx.clone();
            tokio::spawn(async move {
                if let Ok(frame) = reply_rx.await {
                    // Receiver dropped means the connection died
                    // while the actor was busy — fine, just discard.
                    let _ = inspect_tx.send(frame);
                }
            });
            Command::Inspect {
                x,
                y,
                z,
                reply: reply_tx,
            }
        }
    };
    // Send-error means the actor task itself shut down (process
    // shutdown in flight). Connection will hang up naturally on the
    // next loop iteration.
    let _ = world.send_command(cmd);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message as TgMessage;

    use crate::protocol::{CELL_DETAIL_TAG, SNAPSHOT_TAG};
    use crate::world_actor;

    /// Spawn the full HTTP+WS stack on a random localhost port.
    /// Returns `(port, server task handle)`. The task aborts when
    /// the handle is dropped at end of test.
    async fn spawn_test_server() -> (u16, tokio::task::AbortHandle) {
        let world = world_actor::spawn();
        let app = axum::Router::new()
            .route("/sim", axum::routing::get(super::ws_handler))
            .with_state(world);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (port, handle.abort_handle())
    }

    /// Receive next text frame, with timeout. Skips Ping/Pong.
    async fn next_text(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> String {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .expect("text frame timed out")
                .expect("stream ended")
                .expect("ws error");
            match msg {
                // `String::from` is the explicit conversion; `t.to_string()`
                // would trigger `clippy::implicit_clone` on the `Utf8Bytes`
                // → `String` deref path.
                TgMessage::Text(t) => return String::from(t.as_str()),
                TgMessage::Ping(_) | TgMessage::Pong(_) => {}
                other => panic!("expected Text, got {other:?}"),
            }
        }
    }

    /// Receive next binary frame, with timeout.
    async fn next_binary(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Vec<u8> {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .expect("binary frame timed out")
                .expect("stream ended")
                .expect("ws error");
            match msg {
                // `&b[..]` derefs `Bytes` to a `&[u8]` slice
                // unambiguously; plain `b.as_ref()` has multiple
                // `From<&_>` impls so the type can't infer, and
                // `b.to_vec()` triggers `clippy::implicit_clone`.
                TgMessage::Binary(b) => return Vec::from(&b[..]),
                // Skip Ping/Pong (transport-level) and any text
                // frames that arrived before the binary one we want.
                TgMessage::Ping(_) | TgMessage::Pong(_) | TgMessage::Text(_) => {}
                other => panic!("expected Binary, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn handshake_emits_ready_then_welcome() {
        let (port, _abort) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/sim");
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        let ready = next_text(&mut ws).await;
        assert!(ready.contains("\"type\":\"ready\""), "ready frame: {ready}");

        let welcome = next_text(&mut ws).await;
        assert!(
            welcome.contains("\"type\":\"welcome\""),
            "welcome frame: {welcome}"
        );
        assert!(
            welcome.contains("\"running\":false"),
            "welcome frame: {welcome}"
        );
    }

    #[tokio::test]
    async fn init_then_step_emits_snapshot_with_tick_one() {
        let (port, _abort) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/sim");
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Skip ready + welcome
        let _ready = next_text(&mut ws).await;
        let _welcome = next_text(&mut ws).await;

        // Init, then Step
        ws.send(TgMessage::Text(
            r#"{"type":"init","seed":1,"energy":50,"coeff":0.15,"k":1}"#.into(),
        ))
        .await
        .unwrap();
        // Init triggers an immediate snapshot at tick 0 — drain it.
        let init_snap = next_binary(&mut ws).await;
        assert_eq!(init_snap[0], SNAPSHOT_TAG);
        let init_tick = u32::from_le_bytes(init_snap[1..5].try_into().unwrap());
        assert_eq!(init_tick, 0);

        ws.send(TgMessage::Text(r#"{"type":"step"}"#.into()))
            .await
            .unwrap();
        let step_snap = next_binary(&mut ws).await;
        assert_eq!(step_snap[0], SNAPSHOT_TAG);
        let step_tick = u32::from_le_bytes(step_snap[1..5].try_into().unwrap());
        assert_eq!(step_tick, 1);
    }

    #[tokio::test]
    async fn inspect_returns_cell_detail_frame() {
        let (port, _abort) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/sim");
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        let _ = next_text(&mut ws).await; // ready
        let _ = next_text(&mut ws).await; // welcome

        ws.send(TgMessage::Text(
            r#"{"type":"init","seed":1,"energy":50,"coeff":0.15,"k":1}"#.into(),
        ))
        .await
        .unwrap();
        let _init_snap = next_binary(&mut ws).await;

        ws.send(TgMessage::Text(
            r#"{"type":"inspect","x":0,"y":0,"z":0}"#.into(),
        ))
        .await
        .unwrap();

        // Inspect reply is binary; tag = CELL_DETAIL_TAG (2).
        // Snapshots could in principle interleave from a running
        // world, but `running` is false here so the only binary
        // frame we'll see is the cellDetail reply.
        let frame = next_binary(&mut ws).await;
        assert_eq!(frame[0], CELL_DETAIL_TAG, "expected cellDetail tag");
    }

    #[tokio::test]
    async fn malformed_json_does_not_drop_connection() {
        let (port, _abort) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/sim");
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        let _ = next_text(&mut ws).await;
        let _ = next_text(&mut ws).await;

        // Send garbage; handler should silently drop and keep going.
        ws.send(TgMessage::Text("{not even json".into()))
            .await
            .unwrap();

        // Then a valid step, expect normal snapshot.
        ws.send(TgMessage::Text(
            r#"{"type":"init","seed":1,"energy":10,"coeff":0.15,"k":1}"#.into(),
        ))
        .await
        .unwrap();
        let frame = next_binary(&mut ws).await;
        assert_eq!(frame[0], SNAPSHOT_TAG);
    }

    #[tokio::test]
    async fn welcome_running_true_after_running_command() {
        let (port, _abort) = spawn_test_server().await;
        let url = format!("ws://127.0.0.1:{port}/sim");

        // First connection flips running to true.
        let (mut ws_a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let _ = next_text(&mut ws_a).await; // ready
        let _ = next_text(&mut ws_a).await; // welcome (running:false)
        ws_a.send(TgMessage::Text(
            r#"{"type":"init","seed":1,"energy":10,"coeff":0.15,"k":1}"#.into(),
        ))
        .await
        .unwrap();
        let _ = next_binary(&mut ws_a).await; // init snapshot
        ws_a.send(TgMessage::Text(
            r#"{"type":"running","running":true}"#.into(),
        ))
        .await
        .unwrap();

        // Give the actor a beat to publish welcome state.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Second connection should see running:true in welcome.
        let (mut ws_b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let _ready = next_text(&mut ws_b).await;
        let welcome_b = next_text(&mut ws_b).await;
        assert!(
            welcome_b.contains("\"running\":true"),
            "second client should see running:true: {welcome_b}"
        );
    }
}

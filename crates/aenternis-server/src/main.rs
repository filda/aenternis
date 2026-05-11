//! Aenternis native dev backend.
//!
//! Hosts a shared `SparseWorld` and exposes it to the Vite-served
//! viewer over WebSocket. Multiple clients see and drive the same
//! world; control messages and snapshots are multiplexed.
//!
//! ## Status
//!
//! Step 2 skeleton: arg parsing, tokio runtime, axum HTTP listener
//! with a `/health` route, graceful Ctrl-C shutdown. The simulation
//! actor and the `/sim` WebSocket upgrade land in subsequent steps.
//!
//! See `docs/native-server.md` for the dev workflow.

// `pub(crate)` is the workspace-wide idiom for "visible in this
// crate, not exported" (see `unreachable_pub = "warn"` in the root
// Cargo.toml). The new `clippy::redundant_pub_crate` nursery lint
// wants `pub` for items inside a private module of a binary, which
// directly conflicts with that idiom. Silence it crate-wide; the
// `pub(crate)` annotations document the intent more precisely than a
// blanket `pub`.
#![allow(clippy::redundant_pub_crate)]

mod args;

use std::net::SocketAddr;
use std::process::ExitCode;

use aenternis_server::{world_actor, ws};

use crate::args::{parse, Args, ParseOutcome, USAGE};

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let parsed = match parse(std::env::args_os().skip(1), |k| std::env::var(k).ok()) {
        Ok(ParseOutcome::Args(a)) => a,
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            // Conventional exit status for "usage error" — matches `getopt`.
            return ExitCode::from(2);
        }
    };

    if !parsed.is_loopback() {
        tracing::warn!(
            host = %parsed.host,
            "bound to non-loopback address \u{2014} no auth enforced. LAN-only, dev use."
        );
    }

    match run(parsed).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("server error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Drive the HTTP listener until Ctrl-C. Split out from `main` so the
/// happy path uses `?` for `io::Error`s without colliding with the
/// `ExitCode` return type the binary contract expects.
async fn run(args: Args) -> std::io::Result<()> {
    // Single shared world for every client on this server. Every
    // WebSocket connection takes a Handle clone via `with_state`.
    let world = world_actor::spawn();
    let app = axum::Router::new()
        .route("/health", axum::routing::get(health))
        .route("/sim", axum::routing::get(ws::ws_handler))
        .with_state(world);

    let addr = SocketAddr::new(args.host, args.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("aenternis-server listening on http://{addr} (ws at /sim)");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// `/health` route. Plain `text/plain` "ok" so a curl probe or a
/// connectivity test from the viewer can confirm the server is up
/// without parsing JSON.
async fn health() -> &'static str {
    "ok"
}

/// Resolves once Ctrl-C is pressed. Used as `with_graceful_shutdown`
/// hook so in-flight responses get a chance to drain.
async fn shutdown_signal() {
    // `expect` is intentional — if the process can't even install the
    // Ctrl-C handler, there's no recovery path; the alternative is to
    // ignore the failure and never shut down cleanly.
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
    tracing::info!("shutdown signal received");
}

/// Configure `tracing-subscriber` from `RUST_LOG`, defaulting to `info`
/// when the env var is missing or malformed.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

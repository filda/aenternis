//! Aenternis native dev backend — library surface.
//!
//! The crate is laid out as `[lib]+[[bin]]`. The library exists so
//! benches can call the binary-format encoder directly; the binary
//! is the actual server entry-point.
//!
//! Modules are exposed to the binary at `pub(crate)` granularity
//! whenever possible. Only `protocol` (consumed by the snapshot
//! bench) and the small surface the binary needs to construct the
//! axum router (`spawn`, `Handle`, `ws_handler`) sit at `pub`.

// See the rationale in `main.rs`: workspace style is `pub(crate)`,
// but `clippy::redundant_pub_crate` (nursery) wants plain `pub`
// inside private modules. The annotations document intent more
// precisely than a crate-wide `pub` would.
#![allow(clippy::redundant_pub_crate)]

pub mod protocol;
pub mod world_actor;
pub mod ws;

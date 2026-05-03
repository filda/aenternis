//! # Aenternis core
//!
//! Simulation core for Aenternis: cells, world topology, virtual machine,
//! and per-tick physics. The crate has no WASM dependencies — browser
//! integration lives in the separate `aenternis-wasm` crate, layered on top.
//!
//! ## Vocabulary
//!
//! See `docs/aenternis.md` and `docs/mechanics.md` in the repository root
//! for the full design corpus. Briefly:
//!
//! - **cell** — a physical location with energy and memory
//! - **slot** — one 32-bit unit of memory (= one unit of energy)
//! - **direction** — one of six orthogonal faces (`xp`, `xn`, `yp`, `yn`, `zp`, `zn`)
//!
//! ## Status
//!
//! Skeleton crate. Today only the world coordinate primitive is wired up.
//! Cell, world, RNG, VM, and tick logic are scheduled to land in subsequent
//! commits — see `docs/plan.md` for the roadmap.

pub mod cell;
pub mod coord;
pub mod rng;
pub mod tick;
pub mod vm;
pub mod world;

pub use crate::cell::Cell;
pub use crate::coord::{Coord, Direction};
pub use crate::rng::Rng;
pub use crate::vm::Opcode;
pub use crate::world::SparseWorld;

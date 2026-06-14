//! World topology — different ways to arrange and address cells.
//!
//! Today only the sparse world (`SparseWorld`) exists. A toroidal reference
//! model will be added later as a fixed-N test harness for bit-identity
//! comparison against the JS prototypes.

pub mod arena;
pub mod cells;
pub mod sparse;

pub use arena::Arena;
pub(crate) use cells::Cells;

pub use crate::world::sparse::{Base, MemoryReport, PossessError, SparseWorld};

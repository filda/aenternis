//! World topology — different ways to arrange and address cells.
//!
//! Today only the sparse world (`SparseWorld`) exists. A toroidal reference
//! model will be added later as a fixed-N test harness for bit-identity
//! comparison against the JS prototypes.

pub mod cells;
pub mod sparse;

pub(crate) use cells::Cells;

pub use crate::world::sparse::SparseWorld;

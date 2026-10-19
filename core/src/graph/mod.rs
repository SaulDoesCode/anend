//! The entry graph (work in progress).

pub mod assets;
pub mod backup;
pub mod blocks;
pub mod codec;
pub mod gematria;
pub mod interp;
pub mod place;
pub mod postings;
pub mod query;
pub mod record;
pub mod store;

pub use store::{GResult, Graph, Reader, Tx, View};

#[cfg(test)]
mod tests;

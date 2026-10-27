//! The entry graph (work in progress).

pub mod anims;
pub mod assets;
pub mod backup;
pub mod blocks;
pub mod chains;
pub mod checkpoints;
pub mod codec;
pub mod gematria;
pub mod interp;
pub mod lights;
pub mod place;
pub mod postings;
pub mod prefabs;
pub mod query;
pub mod record;
pub mod scenes;
pub mod settle;
pub mod store;
pub mod textures;
pub mod wiring;

pub use store::{GResult, Graph, Reader, Tx, View};

#[cfg(test)]
mod tests;

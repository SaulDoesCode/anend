//! anend's saves, shared by the desktop app and the hub: notes, memories and their
//! gematria search tables, lists, the forge, and weum's helpers. No user interface here.

pub mod bundle;
pub mod classifiers;
pub mod classify;
pub mod clsmodule;
pub mod filters;
pub mod fmath;
pub mod forge;
#[cfg(not(target_arch = "wasm32"))]
pub mod grammar;
pub mod graph;
pub mod kinds;
pub mod lists;
pub mod markdown;
pub mod measure;
pub mod memo;
pub mod memory;
pub mod note;
pub mod ops;
pub mod pieces;
pub mod simd;
pub mod specs;
pub mod spread;
#[cfg(not(target_arch = "wasm32"))]
pub mod starters;
pub mod store;
pub mod texts;
pub mod vfs;
pub mod weum;

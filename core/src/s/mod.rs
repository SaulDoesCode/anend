//! s, the user's scripting language (docs/s.md): a postfix language whose acting word
//! stands at the right. Programs are entries of the save (mime `text/x-s`), stored in the
//! 5-bit format like any text (the format's s mode makes their symbols cheap, see
//! `graph::codec`), and run here: `lex` makes tokens, `machine` runs them (tail calls in
//! constant space), `lib` is the library (`std/*.s`).

pub mod lex;
pub mod lib;
pub mod machine;
pub mod val;

use std::collections::BTreeMap;

pub use machine::{Outcome, Rt};

/// runs a program: its source, what stdin holds, its files, its arguments, and at most how
/// many steps it may take
pub fn run(src: &str, stdin: &str, files: BTreeMap<String, String>, args: &[String], max_steps: u64) -> Outcome {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(7, |d| d.as_nanos() as u64);
    let mut rt = Rt::new(stdin, files, seed);
    rt.max_steps = max_steps;
    rt.run_program(src, args)
}

#[cfg(test)]
mod tests;

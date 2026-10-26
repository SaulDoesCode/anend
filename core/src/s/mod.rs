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

/// runs a program: its source, what stdin holds, its files, its arguments, at most how
/// many steps it may take, and the seed of its random numbers (the caller's clock: the
/// browser has no system time to read here)
pub fn run(
    src: &str,
    stdin: &str,
    files: BTreeMap<String, String>,
    args: &[String],
    max_steps: u64,
    seed: u64,
) -> Outcome {
    let mut rt = Rt::new(stdin, files, seed);
    rt.max_steps = max_steps;
    rt.run_program(src, args)
}

#[cfg(test)]
mod tests;

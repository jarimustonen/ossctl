#![deny(clippy::print_stdout)]

//! `shipshape` command implementation.
//!
//! The crates.io package is `shipshape-cli`; both its own binary and the
//! cargo-dist-only `shipshape` wrapper call this library entry point. Keeping one
//! implementation prevents registry and distribution package coordinates from
//! producing different commands.

mod audit;
mod cli;
mod config;
mod contract;
mod dist;
mod doctor;
mod error;
mod facts;
mod help;
mod output;
mod release;
mod skill;
mod sys;

use std::process::ExitCode;

/// Parse arguments and execute the Shipshape command.
///
/// This entry point exists only for the non-published cargo-dist naming wrapper;
/// the supported public interface is the `shipshape` executable.
#[doc(hidden)]
pub fn run() -> ExitCode {
    cli::run()
}

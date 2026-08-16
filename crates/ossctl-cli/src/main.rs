//! `ossctl` binary entry point.
//!
//! A thin clap + I/O shell over `ossctl-core`. All deterministic logic lives in
//! the library; this crate owns the command surface, output rendering, and the
//! self-diagnostic (ADR-0001 §2).

mod audit;
mod cli;
mod config;
mod contract;
mod dist;
mod doctor;
mod error;
mod facts;
mod output;
mod release;
mod skill;
mod sys;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}

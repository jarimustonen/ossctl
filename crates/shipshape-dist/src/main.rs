//! cargo-dist entry point for Shipshape-named release artifacts.

use std::process::ExitCode;

fn main() -> ExitCode {
    shipshape_cli::run()
}

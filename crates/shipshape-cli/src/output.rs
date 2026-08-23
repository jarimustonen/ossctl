//! Stdout success-payload helpers and the output-format model.
//!
//! Every machine-readable payload is shaped as the canonical envelope:
//!
//! ```json
//! {"schema_version": 1, "data": {...subcommand body...}, "warnings": []}
//! ```
//!
//! The body lives under a dedicated `data` key so the envelope can grow
//! reserved fields (`warnings`, `dry_run`, `trace_id`, …) over time without
//! colliding with payload field names (mirrors octl-core's envelope).
//!
//! Format is selected only by the explicit global `--json` flag, never by
//! `isatty()` (`AGENTS-AI-FIRST-CLI.md` §9). Streaming `--output=jsonl` (§12) is
//! a release-engine concern and is added when the long-running `release`
//! commands land — the scaffold has no long-running command to stream.

use std::fmt;
use std::io::{self, Write};

use serde::Serialize;

use shipshape_core::SCHEMA_VERSION;

use crate::error::CliError;

/// Resolved output format for one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable rendering (per-command). The default.
    #[default]
    Text,
    /// Single JSON envelope document (opted into with `--json`).
    Json,
}

impl OutputFormat {
    /// Resolve the format from the global `--json` boolean.
    pub fn from_json_flag(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Text
        }
    }
}

#[derive(Serialize)]
struct SuccessEnvelope<'a, T: Serialize> {
    schema_version: u32,
    data: &'a T,
    /// Always emitted (even when empty) per §10: a missing-vs-empty branch is a
    /// consumer tax. `warnings: []` is the steady state.
    warnings: &'a [String],
}

/// Write formatted success output to stdout without panicking.
///
/// A closed downstream pipe is conventional pipeline completion, so it is
/// treated as success and emitted nowhere. Other write failures are system I/O
/// errors and flow through the CLI's canonical error envelope.
pub fn write_stdout(args: fmt::Arguments<'_>) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    write_to(&mut lock, args)
}

fn write_to(writer: &mut dyn Write, args: fmt::Arguments<'_>) -> Result<(), CliError> {
    match writer.write_fmt(args).and_then(|()| writer.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(CliError::system(
            "io_stdout",
            format!("failed to write stdout: {error}"),
        )),
    }
}

macro_rules! stdout {
    ($($arg:tt)*) => {{
        $crate::output::write_stdout(format_args!($($arg)*))
    }};
}

macro_rules! stdoutln {
    () => {{
        $crate::output::write_stdout(format_args!("\n"))
    }};
    ($($arg:tt)*) => {{
        $crate::output::write_stdout(format_args!("{}\n", format_args!($($arg)*)))
    }};
}

pub(crate) use stdout;
pub(crate) use stdoutln;

/// Serialize `body` inside the canonical success envelope and write it to
/// stdout as a single pretty JSON document followed by a newline.
///
/// Only the JSON branch routes here; text rendering is each subcommand's own
/// responsibility (formats differ per command).
pub fn emit_json<T: Serialize>(body: &T, warnings: &[String]) -> Result<(), CliError> {
    let envelope = SuccessEnvelope {
        schema_version: SCHEMA_VERSION,
        data: body,
        warnings,
    };
    let mut s = serde_json::to_string_pretty(&envelope)
        .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    s.push('\n');
    write_stdout(format_args!("{s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_success() {
        let mut writer = FailingWriter(io::ErrorKind::BrokenPipe);
        assert!(write_to(&mut writer, format_args!("payload")).is_ok());
    }

    #[test]
    fn other_stdout_failure_is_structured_system_error() {
        let mut writer = FailingWriter(io::ErrorKind::PermissionDenied);
        let error = write_to(&mut writer, format_args!("payload")).unwrap_err();

        assert!(matches!(error.kind, crate::error::ExitKind::System));
        assert_eq!(error.code, "io_stdout");
        assert!(error.message.starts_with("failed to write stdout: "));
    }

    struct FlushFailingWriter(io::ErrorKind);

    impl Write for FlushFailingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(self.0))
        }
    }

    #[test]
    fn deferred_broken_pipe_is_success() {
        let mut writer = FlushFailingWriter(io::ErrorKind::BrokenPipe);
        assert!(write_to(&mut writer, format_args!("payload")).is_ok());
    }

    #[test]
    fn deferred_non_broken_pipe_is_structured_system_error() {
        let mut writer = FlushFailingWriter(io::ErrorKind::PermissionDenied);
        let error = write_to(&mut writer, format_args!("payload")).unwrap_err();

        assert!(matches!(error.kind, crate::error::ExitKind::System));
        assert_eq!(error.code, "io_stdout");
    }
}

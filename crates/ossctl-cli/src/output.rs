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

use serde::Serialize;

use ossctl_core::SCHEMA_VERSION;

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

/// Serialize `body` inside the canonical success envelope and print it to
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
    print!("{s}");
    Ok(())
}

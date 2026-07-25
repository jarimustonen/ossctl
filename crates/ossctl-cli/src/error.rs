//! The structured error envelope shared by every subcommand
//! (`AGENTS-AI-FIRST-CLI.md` §10).
//!
//! Failures emit a JSON object to **stderr** regardless of the stdout format:
//! the AI caller must parse failures the same way every time. Exit codes follow
//! §2: `0` success, `1` user/validation error, `2` system error (which the
//! scaffold also uses for `not_implemented` — the feature is absent from the
//! tool, not a caller mistake).

use ossctl_core::SCHEMA_VERSION;
use serde::Serialize;

/// Process exit classes (`AGENTS-AI-FIRST-CLI.md` §2).
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum ExitKind {
    /// Invalid input the caller can fix (`1`).
    User = 1,
    /// Tool/system-level failure or an unbuilt feature (`2`).
    System = 2,
}

#[derive(Debug, Serialize)]
struct ErrorPayload<'a> {
    schema_version: u32,
    error: ErrorBody<'a>,
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_value: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<&'a serde_json::Value>,
}

/// A structured, emit-once CLI error.
#[derive(Debug)]
pub struct CliError {
    /// Exit class → process exit code.
    pub kind: ExitKind,
    /// Stable machine-readable error code (e.g. `not_implemented`).
    pub code: String,
    /// Human- and agent-readable message; names the actual offending value.
    pub message: String,
    /// The offending value, when there is a single one to echo back (§4).
    pub invalid_value: Option<String>,
    /// The accepted alternatives, when a closed set applies (§4).
    pub expected: Option<serde_json::Value>,
}

impl CliError {
    /// A caller-fixable input error (exit `1`).
    pub fn user(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ExitKind::User,
            code: code.into(),
            message: message.into(),
            invalid_value: None,
            expected: None,
        }
    }

    /// A tool/system-level error (exit `2`).
    pub fn system(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ExitKind::System,
            code: code.into(),
            message: message.into(),
            invalid_value: None,
            expected: None,
        }
    }

    /// A "feature not built yet" error for a stub handler (exit `2`). The
    /// scaffold wires the whole taxonomy but only `version` and `doctor` do
    /// real work; every other subcommand returns this clean envelope rather
    /// than panicking.
    pub fn not_implemented(command: &str) -> Self {
        Self::system(
            "not_implemented",
            format!("`ossctl {command}` is not yet implemented (workspace scaffold)"),
        )
    }

    /// Attach the offending value (§4).
    pub fn with_invalid_value(mut self, value: impl Into<String>) -> Self {
        self.invalid_value = Some(value.into());
        self
    }

    /// Attach the accepted-alternatives set (§4).
    pub fn with_expected(mut self, expected: serde_json::Value) -> Self {
        self.expected = Some(expected);
        self
    }

    /// Print the error envelope to stderr as a single JSON line.
    pub fn emit(&self) {
        let payload = ErrorPayload {
            schema_version: SCHEMA_VERSION,
            error: ErrorBody {
                code: &self.code,
                message: &self.message,
                invalid_value: self.invalid_value.as_deref(),
                expected: self.expected.as_ref(),
            },
        };
        match serde_json::to_string(&payload) {
            Ok(s) => eprintln!("{s}"),
            Err(_) => eprintln!(
                "{{\"schema_version\":{SCHEMA_VERSION},\"error\":{{\"code\":\"internal_serialize\",\"message\":\"failed to serialize error envelope\"}}}}"
            ),
        }
    }
}

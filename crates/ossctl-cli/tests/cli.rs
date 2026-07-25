//! Integration tests for the `ossctl` binary surface (the scaffold's live
//! contract: `version`, `doctor`, and the `not_implemented` stub envelope).

use assert_cmd::Command;
use predicates::prelude::*;

fn ossctl() -> Command {
    Command::cargo_bin("ossctl").expect("ossctl binary builds")
}

/// `version --json` emits the §10/§17 fields inside the canonical envelope.
#[test]
fn version_json_emits_schema_fields() {
    let out = ossctl().args(["version", "--json"]).output().unwrap();
    assert!(out.status.success(), "version --json must exit 0");

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(v["schema_version"], 1, "envelope schema_version");
    assert_eq!(
        v["warnings"],
        serde_json::json!([]),
        "warnings always present"
    );

    let data = &v["data"];
    assert!(data["version"].is_string(), "version present: {data}");
    assert!(data["commit"].is_string(), "commit present: {data}");
    assert_eq!(data["schema_version"], 1, "data schema_version");
    assert_eq!(
        data["supported_schemas"],
        serde_json::json!([1]),
        "supported_schemas present"
    );
    assert!(
        data["skills"].is_array(),
        "skills is an array (empty at founding): {data}"
    );
}

/// Text mode is the default and prints a human-readable version line.
#[test]
fn version_text_is_default() {
    ossctl()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("ossctl "));
}

/// `doctor` runs the self-check and exits 0 (no failing checks).
#[test]
fn doctor_runs_and_exits_zero() {
    ossctl()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("summary:"));
}

/// `doctor --json` emits the §18 shape inside the envelope.
#[test]
fn doctor_json_shape() {
    let out = ossctl().args(["doctor", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["data"]["checks"].is_array(), "checks array: {v}");
    assert!(v["data"]["summary"]["ok"].is_u64(), "summary.ok: {v}");
}

/// `doctor --json --fix` keeps stderr clean: a JSON caller's stderr is the
/// fatal-only channel, so the `--fix` narration must not leak there.
#[test]
fn doctor_json_fix_leaves_stderr_clean() {
    let out = ossctl()
        .args(["doctor", "--json", "--fix"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "JSON-mode --fix must not narrate on stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    // stdout is still valid envelope JSON.
    let _: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
}

/// A stub subcommand returns a clean `not_implemented` error envelope on
/// stderr and exits 2 — not a panic.
#[test]
fn stub_subcommand_returns_not_implemented() {
    let out = ossctl().arg("facts").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "not_implemented → exit 2");

    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["error"]["code"], "not_implemented");
    assert!(out.stdout.is_empty(), "no data on stdout for an error");
}

/// An unknown subcommand is a clap error rendered through the §10 envelope.
#[test]
fn unknown_subcommand_is_structured_error() {
    let out = ossctl().arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "user error → exit 1");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "unknown_subcommand");
}

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
    let out = ossctl().arg("audit").output().unwrap();
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

// ── contract show / validate ─────────────────────────────────────────────────

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Every positive fixture normalizes cleanly and the `data` payload carries the
/// canonical SCHEMA.md §4 fields under the CLI envelope.
#[test]
fn contract_show_positive_fixtures() {
    for name in ["solo-rust-cli", "node-monorepo", "python-lib", "go-cli"] {
        let out = ossctl()
            .args(["contract", "show", "--json", "--repo-root"])
            .arg(fixture(name))
            .output()
            .unwrap();
        assert!(out.status.success(), "{name} must show cleanly");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
        assert_eq!(v["schema_version"], 1, "{name} envelope");
        let data = &v["data"];
        assert_eq!(data["schema_version"], 1, "{name} contract schema_version");
        assert!(data["maturity"].is_string(), "{name} maturity: {data}");
        assert!(data["targets"].is_array(), "{name} targets: {data}");
        assert!(data["extra_fields"].is_object(), "{name} extra_fields");
        assert!(data["warnings"].is_array(), "{name} warnings");
        assert!(data["versioning_pattern"].is_null() || data["versioning_pattern"].is_string());
    }
}

/// `python-lib` omits `targets`; the reader expands one concrete `PyPI` target.
#[test]
fn contract_show_expands_omitted_targets() {
    let out = ossctl()
        .args(["contract", "show", "--json", "--repo-root"])
        .arg(fixture("python-lib"))
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let targets = v["data"]["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["ecosystem"], "python");
    assert_eq!(targets[0]["registry"], "pypi");
    assert_eq!(targets[0]["adapter"], "gh-action-pypi-publish");
    assert!(targets[0]["package"].is_null());
}

/// `go-cli` carries an unknown field — preserved under `extra_fields`, reported
/// once in `warnings`, never dropped (forward-compat).
#[test]
fn contract_show_preserves_unknown_fields() {
    let out = ossctl()
        .args(["contract", "show", "--json", "--repo-root"])
        .arg(fixture("go-cli"))
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["data"]["extra_fields"]["roadmap_url"],
        "https://example.com/roadmap"
    );
    let warnings = v["data"]["warnings"].as_array().unwrap();
    assert!(warnings
        .iter()
        .any(|w| w.as_str().is_some_and(|s| s.contains("roadmap_url"))));
}

/// `contract show` is idempotent: re-showing yields byte-identical output.
#[test]
fn contract_show_is_idempotent() {
    let run = || {
        ossctl()
            .args(["contract", "show", "--json", "--repo-root"])
            .arg(fixture("solo-rust-cli"))
            .output()
            .unwrap()
            .stdout
    };
    assert_eq!(run(), run(), "contract show must be deterministic");
}

/// Every negative fixture is refused: exit 1, `invalid_contract` envelope on
/// stderr carrying the full problem list, no document on stdout.
#[test]
fn contract_validate_rejects_floor_violations() {
    for name in [
        "neg-auto-on-spike",
        "neg-registry-without-license",
        "neg-badge-without-producer",
        "neg-schema-too-new",
        "neg-fragment-dir-escape",
    ] {
        let out = ossctl()
            .args(["contract", "validate", "--repo-root"])
            .arg(fixture(name))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{name} → exit 1");
        assert!(out.stdout.is_empty(), "{name} emits no body on failure");
        let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
        assert_eq!(v["error"]["code"], "invalid_contract", "{name}");
        assert!(
            v["error"]["problems"]
                .as_array()
                .is_some_and(|p| !p.is_empty()),
            "{name} carries the problem list: {v}"
        );
    }
}

/// `show` on a valid config also refuses to emit when it is invalid (same gate).
#[test]
fn contract_show_rejects_invalid() {
    let out = ossctl()
        .args(["contract", "show", "--json", "--repo-root"])
        .arg(fixture("neg-auto-on-spike"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "no canonical body on invalid");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "invalid_contract");
}

/// A positive fixture passes `validate` (exit 0).
#[test]
fn contract_validate_accepts_valid() {
    ossctl()
        .args(["contract", "validate", "--repo-root"])
        .arg(fixture("python-lib"))
        .assert()
        .success();
}

/// `--require-approved` refuses a `status: draft` config (the mutating-member
/// gate) with a `not_approved` envelope, exit 1.
#[test]
fn contract_require_approved_refuses_draft() {
    let out = ossctl()
        .args(["contract", "show", "--require-approved", "--repo-root"])
        .arg(fixture("solo-rust-cli")) // status: draft
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "not_approved");
    assert_eq!(v["error"]["invalid_value"], "draft");
}

/// A missing `OSS-RELEASE.md` is a system-level (exit 2) error, distinct from an
/// invalid one (exit 1).
#[test]
fn contract_show_missing_file_is_system_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["contract", "show", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "missing file → exit 2");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "contract_not_found");
}

// ── facts ────────────────────────────────────────────────────────────────────

/// `facts --json` over a manifest-bearing (non-git) temp dir emits the canonical
/// envelope with the SCHEMA.md §4 field names oss-init relies on. A fresh temp
/// dir is outside any git work tree, so the git-derived fields are deterministic
/// (`is_git: false`, no committers, no tags).
#[test]
fn facts_json_shape_and_field_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"toolx\"\nversion = \"0.3.0\"\ndescription = \"a tool\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), "on: push\n").unwrap();

    let out = ossctl()
        .args(["facts", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "facts --json must exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(v["schema_version"], 1, "envelope schema_version");
    assert_eq!(v["warnings"], serde_json::json!([]));

    let d = &v["data"];
    // The exact field names oss-init reads (SCHEMA.md §4).
    for key in [
        "ecosystems",
        "packages",
        "has_ci",
        "tags",
        "committers_total",
        "committers_recent_year",
        "inferred_maturity",
        "maturity_signals",
        "has_semver_tag",
        "has_ge_1_0_release",
    ] {
        assert!(d.get(key).is_some(), "missing facts key {key}: {d}");
    }
    assert_eq!(d["ecosystems"], serde_json::json!(["rust"]));
    assert_eq!(d["packages"][0]["package"], "toolx");
    assert_eq!(d["packages"][0]["version"], "0.3.0");
    assert_eq!(d["has_ci"], true);
    assert_eq!(d["is_git"], false, "a temp dir is not a git work tree");
    assert_eq!(d["committers_total"], 0);
    assert_eq!(d["tags"], serde_json::json!([]));
    assert_eq!(d["description"], "a tool");
    // Has CI (so not spike) but no >=1.0 and no recent committers → mvp.
    assert_eq!(d["inferred_maturity"], "mvp");
}

/// A repo with no package manifest reports the `binary` ecosystem and — with no
/// CI, tags, or committers — a `spike` maturity.
#[test]
fn facts_binary_fallback_is_spike() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Makefile"), "all:\n\techo hi\n").unwrap();

    let out = ossctl()
        .args(["facts", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["ecosystems"], serde_json::json!(["binary"]));
    assert_eq!(v["data"]["inferred_maturity"], "spike");
}

/// `facts` is deterministic: two runs over the same tree are byte-identical.
#[test]
fn facts_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"name": "widget", "version": "1.0.0"}"#,
    )
    .unwrap();
    let run = || {
        ossctl()
            .args(["facts", "--json", "--repo-root"])
            .arg(dir.path())
            .output()
            .unwrap()
            .stdout
    };
    assert_eq!(run(), run(), "facts must be deterministic");
}

/// A non-existent `--repo-root` is a caller-fixable (exit 1) error.
#[test]
fn facts_missing_repo_root_is_user_error() {
    let out = ossctl()
        .args(["facts", "--repo-root", "/no/such/dir/anywhere"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "invalid_repo_root");
}

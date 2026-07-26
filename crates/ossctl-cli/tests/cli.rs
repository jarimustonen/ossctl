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
    let out = ossctl().args(["release", "list"]).output().unwrap();
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

/// The `/oss-init` skill's whole "stage → validate → install" lifecycle rests on
/// one invariant: `contract validate`'s hard-floor **verdict** is a pure function
/// of the `OSS-RELEASE.md` document plus lexical `--repo-root`-relative path
/// checks — it does NOT depend on the repo's filesystem contents (manifests, git,
/// CI). That is what lets the skill validate a *staged* proposal under a bare
/// scratchpad `--repo-root` and trust the result for the real repo. This test
/// pins the invariant: validate the same document under (a) an empty staging dir
/// and (b) a populated "real repo" dir, and assert identical exit + error codes.
/// If a future floor becomes filesystem-dependent, this fails — a caught
/// regression rather than a skill that silently installs a bad config.
#[test]
fn validate_verdict_is_independent_of_repo_filesystem() {
    // A clean fixture (must pass at both roots) and every hard-floor negative
    // (must fail identically at both roots).
    for name in [
        "solo-rust-cli",
        "python-lib",
        "neg-auto-on-spike",
        "neg-registry-without-license",
        "neg-badge-without-producer",
        "neg-schema-too-new",
        "neg-fragment-dir-escape",
    ] {
        let doc = std::fs::read(fixture(name).join("OSS-RELEASE.md")).unwrap();

        // (a) bare staging root — nothing but the document (the skill's scratchpad).
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("OSS-RELEASE.md"), &doc).unwrap();

        // (b) a populated "real repo" — the filesystem signals the normalizer
        // must be proven to ignore: a git dir, manifests, CI, a fragments dir.
        let real = tempfile::tempdir().unwrap();
        std::fs::write(real.path().join("OSS-RELEASE.md"), &doc).unwrap();
        std::fs::create_dir_all(real.path().join(".git")).unwrap();
        std::fs::create_dir_all(real.path().join(".github/workflows")).unwrap();
        std::fs::create_dir_all(real.path().join("changelog/fragments")).unwrap();
        std::fs::write(real.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(real.path().join("package.json"), "{}\n").unwrap();
        std::fs::write(real.path().join("README.md"), "# x\n").unwrap();

        let run = |root: &std::path::Path| {
            let out = ossctl()
                .args(["contract", "validate", "--json", "--repo-root"])
                .arg(root)
                .output()
                .unwrap();
            let stream = if out.status.success() {
                &out.stdout
            } else {
                &out.stderr
            };
            let v: serde_json::Value = serde_json::from_slice(stream).unwrap();
            // (exit code, error code or "" when valid) — the verdict, not the warnings.
            (
                out.status.code(),
                v["error"]["code"].as_str().unwrap_or("").to_string(),
            )
        };

        assert_eq!(
            run(staging.path()),
            run(real.path()),
            "{name}: validate verdict must be identical at a bare staging root and a populated \
             real repo root (the staging-root invariant the oss-init skill depends on)"
        );
    }
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

// ── audit ────────────────────────────────────────────────────────────────────

/// List the immediate entry names of a directory, sorted — for the read-only
/// assertion (the audit must not add, remove, or change any repo file).
fn dir_snapshot(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `audit --json` over a repo that has a normalizing contract but none of the
/// gated-core files emits the canonical gap-report envelope, reports the core
/// incomplete, and — in a non-git temp dir — degrades the GitHub community
/// lookup to `unknown` (never `false`). It writes nothing to the repo.
#[test]
fn audit_json_reports_core_gaps_and_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    // A known-clean positive contract; the temp dir has no README/LICENSE/CI.
    std::fs::copy(
        fixture("python-lib").join("OSS-RELEASE.md"),
        dir.path().join("OSS-RELEASE.md"),
    )
    .unwrap();

    let before = dir_snapshot(dir.path());
    let out = ossctl()
        .args(["audit", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    // Gaps are data, not an error: a repo with gaps still exits 0.
    assert!(out.status.success(), "audit --json must exit 0");

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["warnings"], serde_json::json!([]));

    let d = &v["data"];
    assert!(d["maturity"].is_string(), "maturity present: {d}");
    assert_eq!(d["core_complete"], "incomplete", "no README/LICENSE/CI");
    let gaps = d["gaps"].as_array().expect("gaps is an array");
    let gap_ids: Vec<&str> = gaps.iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert!(gap_ids.contains(&"readme"), "readme gap: {gap_ids:?}");
    assert!(gap_ids.contains(&"license"), "license gap: {gap_ids:?}");

    // A fresh temp dir is outside any GitHub remote, so the community lookup
    // could not run — every field is `unknown`, never `false`.
    let cp = &d["community_profile"];
    assert_eq!(cp["checked"], false, "no GitHub remote → unchecked");
    assert_eq!(cp["readme"], "unknown", "outage ⇒ unknown, never absent");

    // Read-only: the audit added/removed nothing.
    assert_eq!(
        dir_snapshot(dir.path()),
        before,
        "audit must not write the repo"
    );
}

/// `audit` over a repo without an `OSS-RELEASE.md` is a system-level (exit 2)
/// error — the audit cannot score without the contract it reads.
#[test]
fn audit_missing_contract_is_system_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["audit", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "missing contract → exit 2");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "contract_not_found");
}

/// A non-existent `--repo-root` is a caller-fixable (exit 1) error.
#[test]
fn audit_missing_repo_root_is_user_error() {
    let out = ossctl()
        .args(["audit", "--repo-root", "/no/such/dir/anywhere"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "invalid_repo_root");
}

// ── release verify: read-only reconcile of a journaled run ───────────────────

/// Write a minimal journal for `run_id` under a `--journal-dir` root and return
/// that root. The events use the flat on-disk line shape the journal reads back.
fn seed_journal(run_id: &str, lines: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("journal.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();
    dir
}

/// `release verify` reconciles a journaled run and emits the report envelope.
/// Uses `rust` + `binary` targets, which classify as `unknown` without any
/// network access (rust has no wired registry query yet; binary is structurally
/// unobservable), so the test is deterministic and offline.
#[test]
fn release_verify_reconciles_a_journaled_run() {
    let dir = seed_journal(
        "RUN01",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN01","plan_id":"plan-abc","targets":["cargo","gh"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
            r#"{"schema_version":1,"seq":3,"ts":1002,"idempotency_key":"published:gh","kind":"target_published","target":"gh","receipt":{"ecosystem":"binary","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "verify", "RUN01", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "verify must exit 0: {out:?}");

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];
    assert_eq!(data["run_id"], "RUN01");
    assert_eq!(data["plan_id"], "plan-abc");
    // Both targets classify as unknown offline; never a false "missing".
    assert_eq!(data["summary"]["reconciled"], 2);
    assert_eq!(data["summary"]["unknown"], 2);
    assert_eq!(data["summary"]["missing"], 0);
    let outcomes: Vec<&str> = data["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes, vec!["unknown", "unknown"]);

    // Read-only: verify must not materialize a manifest cache next to the journal.
    assert!(
        !dir.path().join("RUN01").join("manifest.json").exists(),
        "verify wrote a manifest — it must be read-only"
    );
}

/// A verify against a run with no journal is a caller-fixable (exit 1) error.
#[test]
fn release_verify_unknown_run_is_user_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["release", "verify", "NOPE", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "run_not_found");
}

/// A `run_id` that is not a single path segment is rejected (no path traversal).
#[test]
fn release_verify_rejects_bad_run_id() {
    let dir = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["release", "verify", "../escape", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "invalid_run_id");
}

/// An interrupted run (a declared target with no receipt) reconciles what landed
/// and surfaces the un-published target as a warning — never a false "missing".
#[test]
fn release_verify_warns_about_unpublished_targets() {
    let dir = seed_journal(
        "RUN02",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN02","plan_id":"plan-xyz","targets":["cargo","npm"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"2.0.0","registry_url":null,"digest":null}}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "verify", "RUN02", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["data"]["summary"]["reconciled"], 1,
        "only the published target reconciles"
    );
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("npm")),
        "the unpublished target must be surfaced as a warning: {warnings:?}"
    );
}

/// A concurrently-appended journal (a torn final line with no trailing newline)
/// must NOT crash verify: the incomplete tail is dropped and the complete events
/// reconcile. This is the "safe against a live run" guarantee.
#[test]
fn release_verify_tolerates_a_torn_final_line() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("RUN03");
    std::fs::create_dir_all(&run_dir).unwrap();
    // Two complete newline-terminated events, then a partial third with NO newline
    // (as if `release cut` were mid-append).
    let good = concat!(
        r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN03","plan_id":"plan-torn","targets":["cargo"]}"#,
        "\n",
        r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
        "\n",
        r#"{"schema_version":1,"seq":3,"ts":1002,"idempotency_key":"published:npm","kind":"target_publ"#, // truncated, no newline
    );
    std::fs::write(run_dir.join("journal.jsonl"), good).unwrap();

    let out = ossctl()
        .args(["release", "verify", "RUN03", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "a torn tail must not fail verify: {out:?}"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // Only the complete cargo receipt is reconciled; the torn npm line is dropped.
    assert_eq!(v["data"]["summary"]["reconciled"], 1);
    assert_eq!(v["data"]["journal_seq"], 2);
}

/// A cancelled target is reported with its cancellation reason, not the generic
/// "not yet published" warning.
#[test]
fn release_verify_reports_cancelled_targets_distinctly() {
    let dir = seed_journal(
        "RUN04",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN04","plan_id":"plan-c","targets":["cargo","npm"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
            r#"{"schema_version":1,"seq":3,"ts":1002,"idempotency_key":"cancelled:npm","kind":"target_cancelled","target":"npm","reason":"OTP timeout"}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "verify", "RUN04", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("npm' was cancelled: OTP timeout")),
        "cancelled target must report its reason: {warnings:?}"
    );
// ── release cut: drift-refusal + approval gate (no external publish) ──────────

/// Prepare a temp repo from a positive fixture with `status: approved`, inside a
/// real git repo with one commit — the minimum a `release cut` needs to reach its
/// drift check. Returns the temp dir (kept alive by the caller).
fn approved_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let contract = std::fs::read_to_string(fixture("solo-rust-cli").join("OSS-RELEASE.md"))
        .unwrap()
        .replace("status: draft", "status: approved");
    std::fs::write(dir.path().join("OSS-RELEASE.md"), contract).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"tool\"\n",
    )
    .unwrap();

    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .expect("git runs")
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
    dir
}

/// `release cut` on a `draft` contract is refused (a cut mutates external state,
/// so it requires human approval) — before any git or journal work.
#[test]
fn release_cut_refuses_a_draft_contract() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture("solo-rust-cli").join("OSS-RELEASE.md"),
        dir.path().join("OSS-RELEASE.md"),
    )
    .unwrap();

    let out = ossctl()
        .args([
            "release",
            "cut",
            "--plan",
            "deadbeef",
            "--version",
            "1.0.0",
            "--repo-root",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "draft cut → user error");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "not_approved");
    assert!(out.stdout.is_empty());
}

/// `release cut` with a `plan_id` the current repo does not hash to is refused
/// with `plan_stale` — the drift guard (ADR-0002 §3). Nothing is published.
#[test]
fn release_cut_refuses_a_stale_plan_id() {
    let dir = approved_git_repo();
    let journal = tempfile::tempdir().unwrap();

    let out = ossctl()
        .args([
            "release",
            "cut",
            "--plan",
            "0000000000000000",
            "--version",
            "1.0.0",
            "--repo-root",
        ])
        .arg(dir.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "stale plan → user error");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "plan_stale");
    // The refusal echoes the offending (approved) id back to the caller.
    assert_eq!(v["error"]["invalid_value"], "0000000000000000");
    assert!(out.stdout.is_empty(), "a refused cut published nothing");
}

/// A correct-shape plan cut with the WRONG version is still drift: the version is
/// part of the content address, so a different `--version` hashes to a different
/// `plan_id` and the cut refuses rather than publishing an unapproved version.
#[test]
fn release_cut_refuses_when_version_differs_from_the_sealed_plan() {
    let dir = approved_git_repo();
    let journal = tempfile::tempdir().unwrap();

    // Seal a real plan at version 1.0.0 and read its plan_id from the envelope.
    let planned = ossctl()
        .args([
            "release",
            "plan",
            "--json",
            "--version",
            "1.0.0",
            "--repo-root",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        planned.status.code(),
        Some(0),
        "plan should succeed on an approved repo"
    );
    let pv: serde_json::Value =
        serde_json::from_slice(&planned.stdout).expect("plan stdout is JSON");
    let plan_id = pv["data"]["plan_id"]
        .as_str()
        .expect("plan_id present")
        .to_string();

    // Execute that exact plan_id but with a different version → drift refusal.
    let out = ossctl()
        .args([
            "release",
            "cut",
            "--plan",
            &plan_id,
            "--version",
            "2.0.0",
            "--repo-root",
        ])
        .arg(dir.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "wrong version → plan_stale");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "plan_stale");
    assert!(out.stdout.is_empty());
}

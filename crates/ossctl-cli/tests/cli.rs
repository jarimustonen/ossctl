//! Integration tests for the `ossctl` binary surface (`version`, `doctor`,
//! `contract`, `facts`, `audit`, `dist`, `skill`, and the `release` verbs).

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
        // The CONTRACT schema_version is the current one (2): a fixture declaring
        // schema_version: 1 is READ as v1 but EMITTED in the v2 canonical shape,
        // re-labeled 2 (never a v2 body stamped v1). Distinct from the envelope
        // version above, which versions the CLI JSON envelope, not the payload.
        assert_eq!(data["schema_version"], 2, "{name} contract schema_version");
        assert!(data["maturity"].is_string(), "{name} maturity: {data}");
        assert!(data["targets"].is_array(), "{name} targets: {data}");
        // Option A (omit-when-empty): `extra_fields` is ABSENT when empty and an
        // object only when a fixture carries unknown keys (e.g. `go-cli`).
        assert!(
            data.get("extra_fields")
                .is_none_or(serde_json::Value::is_object),
            "{name} extra_fields must be absent or an object"
        );
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
/// Uses `python` + `binary` targets, which classify as `unknown` without any
/// network access (python has no wired registry query, so its lookup errors out
/// immediately; binary is structurally unobservable), so the test is
/// deterministic and offline. (`rust`/`node` are wired and would hit the network,
/// so they are deliberately avoided here.)
#[test]
fn release_verify_reconciles_a_journaled_run() {
    let dir = seed_journal(
        "RUN01",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN01","plan_id":"plan-abc","version":"1.0.0","targets":["pypi","gh"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:pypi","kind":"target_published","target":"pypi","receipt":{"ecosystem":"python","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
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
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN02","plan_id":"plan-xyz","version":"1.0.0","targets":["pypi","npm"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:pypi","kind":"target_published","target":"pypi","receipt":{"ecosystem":"python","package":"tool","version":"2.0.0","registry_url":null,"digest":null}}"#,
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
        r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN03","plan_id":"plan-torn","version":"1.0.0","targets":["pypi"]}"#,
        "\n",
        r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:pypi","kind":"target_published","target":"pypi","receipt":{"ecosystem":"python","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
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
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN04","plan_id":"plan-c","version":"1.0.0","targets":["pypi","npm"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:pypi","kind":"target_published","target":"pypi","receipt":{"ecosystem":"python","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
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
}

// ── release show: progress query (§12 snapshot envelope) ─────────────────────

/// A terminal run (`completed`) folds to its final state and emits the §12
/// progress-query snapshot under the canonical envelope: `data.state` (the folded
/// run), `data.last_seq`, and `data.recent_events` (the event window).
#[test]
fn release_show_post_mortem_summary_from_a_journal() {
    let dir = seed_journal(
        "SHOW01",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"SHOW01","plan_id":"plan-done","version":"1.2.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"phase_entered:publish","kind":"phase_entered","phase":"publish"}"#,
            r#"{"schema_version":1,"seq":3,"ts":1002,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"1.2.0","registry_url":null,"digest":null}}"#,
            r#"{"schema_version":1,"seq":4,"ts":1003,"idempotency_key":"phase_completed:publish","kind":"phase_completed","phase":"publish","outcome":"ok"}"#,
            r#"{"schema_version":1,"seq":5,"ts":1004,"idempotency_key":"phase_entered:tag","kind":"phase_entered","phase":"tag"}"#,
            r#"{"schema_version":1,"seq":6,"ts":1005,"idempotency_key":"tag_created_local:v1.2.0","kind":"tag_created_local","tag":"v1.2.0"}"#,
            r#"{"schema_version":1,"seq":7,"ts":1006,"idempotency_key":"phase_completed:tag","kind":"phase_completed","phase":"tag","outcome":"ok"}"#,
            // The post-tag dist barrier is what completes a run (v2): a cut with no
            // post-tag target still runs it as a no-op.
            r#"{"schema_version":2,"seq":8,"ts":1007,"idempotency_key":"phase_entered:dist","kind":"phase_entered","phase":"dist"}"#,
            r#"{"schema_version":2,"seq":9,"ts":1008,"idempotency_key":"phase_completed:dist","kind":"phase_completed","phase":"dist","outcome":"ok"}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "show", "SHOW01", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "show must exit 0: {out:?}");

    // The progress query is one canonical envelope, NOT a JSONL stream.
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("summary is one JSON doc");
    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];
    // Stable public poll cursor + folded state under `data.state`.
    assert_eq!(data["last_seq"], 9);
    let state = &data["state"];
    assert_eq!(state["run_id"], "SHOW01");
    assert_eq!(state["plan_id"], "plan-done");
    assert_eq!(state["version"], "1.2.0");
    assert_eq!(state["status"], "completed");
    assert_eq!(state["published"]["cargo"]["version"], "1.2.0");
    // The recent-event window is present and carries the dist-completion event.
    let events = data["recent_events"].as_array().unwrap();
    assert_eq!(events.len(), 9, "the whole (small) log fits the window");
    assert_eq!(events.last().unwrap()["kind"], "phase_completed");

    // Read-only: show must not materialize a manifest next to the journal.
    assert!(
        !dir.path().join("SHOW01").join("manifest.json").exists(),
        "show wrote a manifest — it must be read-only"
    );
}

/// An abandoned run surfaces its abandon reason as an envelope warning.
#[test]
fn release_show_post_mortem_surfaces_abandon_reason() {
    let dir = seed_journal(
        "SHOW02",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"SHOW02","plan_id":"plan-x","version":"1.0.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"run_abandoned","kind":"run_abandoned","reason":"OTP timeout"}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "show", "SHOW02", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["state"]["status"], "abandoned");
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("abandoned: OTP timeout")),
        "abandon reason must be a warning: {warnings:?}"
    );
}

/// A live (in-progress) run returns the SAME canonical snapshot envelope as a
/// terminal one — §12 forbids switching wire shape across polls. The live event
/// window rides in `data.recent_events`; `data.state.status` is `in_progress`.
#[test]
fn release_show_live_returns_the_same_snapshot_envelope() {
    let dir = seed_journal(
        "SHOW03",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"SHOW03","plan_id":"plan-live","version":"2.0.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"phase_entered:dry_run","kind":"phase_entered","phase":"dry_run"}"#,
            r#"{"schema_version":1,"seq":3,"ts":1002,"idempotency_key":"dry_run:cargo","kind":"target_dry_run","target":"cargo"}"#,
        ],
    );

    let out = ossctl()
        .args(["release", "show", "SHOW03", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "live show must exit 0: {out:?}");

    // One envelope document — identical top-level shape to the terminal case.
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("a live poll is one envelope, not JSONL");
    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];
    assert_eq!(data["state"]["status"], "in_progress");
    assert_eq!(data["last_seq"], 3);
    // A live run in dry_run has no unpublished-target warnings (not gaps yet).
    assert_eq!(v["warnings"], serde_json::json!([]));
    // The recent-event window carries the live tail in ascending seq.
    let events = data["recent_events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["kind"], "run_created");
    assert_eq!(events[2]["kind"], "target_dry_run");
    assert_eq!(events[2]["seq"], 3);
}

/// With an explicit `--journal-dir`, `show` is a pure journal read: it needs no
/// repository, so even a bogus `--repo-root` is ignored (a post-mortem query
/// against an archived journal must work from anywhere).
#[test]
fn release_show_with_journal_dir_ignores_repo_root() {
    let dir = seed_journal(
        "SHOW04",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"SHOW04","plan_id":"p","version":"1.0.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"run_abandoned","kind":"run_abandoned","reason":"stopped"}"#,
        ],
    );
    let out = ossctl()
        .args([
            "release",
            "show",
            "SHOW04",
            "--json",
            "--repo-root",
            "/nonexistent/not/a/repo",
            "--journal-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "an explicit --journal-dir must not require a repo root: {out:?}"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["state"]["run_id"], "SHOW04");
}

/// A `show` against a run with no journal is a caller-fixable (exit 1) error, and
/// a traversal `run_id` is rejected — same guards as `verify`.
#[test]
fn release_show_unknown_and_bad_run_ids_are_user_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = ossctl()
        .args(["release", "show", "NOPE", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&missing.stderr).unwrap();
    assert_eq!(v["error"]["code"], "run_not_found");

    let bad = ossctl()
        .args(["release", "show", "../escape", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&bad.stderr).unwrap();
    assert_eq!(v["error"]["code"], "invalid_run_id");
}

// ── release list: enumerate runs for the in-flight gate ──────────────────────

/// Seed several runs (each `(run_id, journal-lines)`) into one journal root, so a
/// `release list` sees more than one run.
fn seed_journal_multi(runs: &[(&str, &[&str])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (run_id, lines) in runs {
        let run_dir = dir.path().join(run_id);
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(
            run_dir.join("journal.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();
    }
    dir
}

const RUN_CREATED_A: &str = r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNA","plan_id":"plan-aaa","version":"1.0.0","targets":["cargo"]}"#;

/// An empty journal root lists zero runs and is a normal success, not an error.
#[test]
fn release_list_empty_is_a_normal_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["release", "list", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "empty list must exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["data"]["runs"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["in_flight_count"], 0);
    assert_eq!(v["data"]["unreadable"].as_array().unwrap().len(), 0);
}

/// Two runs sharing a `started_ts` fall back to the `run_id` tiebreak, so the order
/// is deterministic (not directory/hash order).
#[test]
fn release_list_tiebreaks_equal_start_times_by_run_id() {
    let mk = |id: &str| {
        format!(
            r#"{{"schema_version":1,"seq":1,"ts":777,"idempotency_key":"run_created","kind":"run_created","run_id":"{id}","plan_id":"plan","version":"1.0.0","targets":["cargo"]}}"#
        )
    };
    // Seed in reverse id order to prove the sort, not insertion order, decides.
    let z = mk("RUNZ");
    let a = mk("RUNA");
    let dir = seed_journal_multi(&[("RUNZ", &[z.as_str()]), ("RUNA", &[a.as_str()])]);
    let out = ossctl()
        .args(["release", "list", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = v["data"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["RUNA", "RUNZ"], "equal ts → sorted by run_id");
}

/// A run whose journal cannot be read (a too-new schema) is NOT silently dropped:
/// it surfaces under `unreadable` with a warning, so the in-flight gate cannot read
/// a false clear coast while a readable run is still reported.
#[test]
fn release_list_surfaces_unreadable_runs() {
    let good = &[RUN_CREATED_A][..];
    // schema_version far in the future → read_events refuses it → list marks it
    // unreadable rather than dropping it.
    let bad = &[
        r#"{"schema_version":9999,"seq":1,"ts":2000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNBAD","plan_id":"p","version":"9.9.9","targets":["cargo"]}"#,
    ][..];
    let dir = seed_journal_multi(&[("RUNA", good), ("RUNBAD", bad)]);
    let out = ossctl()
        .args(["release", "list", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "list stays exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    // The readable run is still reported…
    let ids: Vec<&str> = v["data"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["RUNA"]);
    // …and the unreadable one is explicit (not dropped), with a warning.
    assert_eq!(
        v["data"]["unreadable"],
        serde_json::json!(["RUNBAD"]),
        "an unreadable run must be surfaced, not silently dropped"
    );
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("RUNBAD")),
        "unreadable run needs a warning: {}",
        v["warnings"]
    );
}

/// A single in-flight run is reported with its status, tag, and the `in_flight`
/// gate field the `/oss-release` skill keys on.
#[test]
fn release_list_reports_a_single_in_flight_run() {
    let dir = seed_journal_multi(&[("RUNA", &[RUN_CREATED_A])]);
    let out = ossctl()
        .args(["release", "list", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "list must exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    let runs = v["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "RUNA");
    assert_eq!(runs[0]["status"], "in_progress");
    assert_eq!(runs[0]["version"], "1.0.0");
    assert_eq!(runs[0]["tag"], "v1.0.0");
    assert_eq!(runs[0]["plan_id"], "plan-aaa");
    assert_eq!(runs[0]["in_flight"], true);
    assert_eq!(runs[0]["started_ts"], 1000);
    assert_eq!(v["data"]["in_flight_count"], 1);
}

/// N runs of every status enumerate together, sorted deterministically by start
/// time, with `in_flight_count` counting only the live one.
#[test]
fn release_list_reports_all_statuses_sorted_by_start_time() {
    // Deliberately seed out of start-time order (B starts before A) to prove the
    // output is sorted by `started_ts`, not by directory/run-id order.
    let in_progress = &[RUN_CREATED_A][..];
    let completed = &[
        r#"{"schema_version":1,"seq":1,"ts":500,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNB","plan_id":"plan-bbb","version":"2.0.0","targets":["cargo"]}"#,
        // A v1 `tag ok` completes the run (backward-compat completion signal).
        r#"{"schema_version":1,"seq":2,"ts":501,"idempotency_key":"phase_completed:tag","kind":"phase_completed","phase":"tag","outcome":"ok"}"#,
    ][..];
    let abandoned = &[
        r#"{"schema_version":1,"seq":1,"ts":1500,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNC","plan_id":"plan-ccc","version":"3.0.0","targets":["cargo"]}"#,
        r#"{"schema_version":1,"seq":2,"ts":1501,"idempotency_key":"run_abandoned","kind":"run_abandoned","reason":"gave up"}"#,
    ][..];
    let dir = seed_journal_multi(&[
        ("RUNA", in_progress),
        ("RUNB", completed),
        ("RUNC", abandoned),
    ]);

    let out = ossctl()
        .args(["release", "list", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "list must exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    let runs = v["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    // Sorted by started_ts: RUNB (500) < RUNA (1000) < RUNC (1500).
    let ids: Vec<&str> = runs.iter().map(|r| r["run_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["RUNB", "RUNA", "RUNC"]);
    let statuses: Vec<&str> = runs.iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, vec!["completed", "in_progress", "abandoned"]);
    assert_eq!(v["data"]["in_flight_count"], 1);
    // The abandoned run carries its reason for triage.
    assert_eq!(runs[2]["abandon_reason"], "gave up");
}

// ── release abandon: terminal, event-sourced, no rollback ─────────────────────

/// Abandoning a non-terminal run appends the abandonment fact (history is not
/// rewritten) and marks it terminal; already-published targets are reported as
/// still-live because abandon does not roll back.
#[test]
fn release_abandon_marks_a_non_terminal_run_and_keeps_publishes() {
    let dir = seed_journal(
        "RUN01",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUN01","plan_id":"plan-abc","version":"1.0.0","targets":["cargo","gh"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:cargo","kind":"target_published","target":"cargo","receipt":{"ecosystem":"rust","package":"tool","version":"1.0.0","registry_url":null,"digest":null}}"#,
        ],
    );

    let out = ossctl()
        .args([
            "release",
            "abandon",
            "RUN01",
            "--reason",
            "registry outage",
            "--json",
            "--journal-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "abandon must exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["data"]["run_id"], "RUN01");
    assert_eq!(v["data"]["status"], "abandoned");
    assert_eq!(v["data"]["reason"], "registry outage");
    // The already-published target is named as still-live, and a warning says so.
    assert_eq!(v["data"]["published_targets"], serde_json::json!(["cargo"]));
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("cargo")),
        "a published target must be surfaced as still-live: {warnings:?}"
    );

    // The abandonment is a durable, appended fact: `show` now reads terminal.
    let show = ossctl()
        .args(["release", "show", "RUN01", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(show.status.success());
    let sv: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(sv["data"]["state"]["status"], "abandoned");
    assert_eq!(sv["data"]["state"]["abandon_reason"], "registry outage");
    // History was appended to, not rewritten: the RunCreated + publish + the new
    // RunAbandoned are all present (last_seq advanced to 3).
    assert_eq!(sv["data"]["last_seq"], 3);
}

/// `release abandon` with no `--reason` records a generic default reason.
#[test]
fn release_abandon_without_reason_uses_a_default() {
    let dir = seed_journal("RUN01", &[RUN_CREATED_A.replace("RUNA", "RUN01").as_str()]);
    let out = ossctl()
        .args(["release", "abandon", "RUN01", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "abandon must exit 0: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["data"]["status"], "abandoned");
    assert!(
        v["data"]["reason"].as_str().unwrap().contains("no reason"),
        "default reason expected: {}",
        v["data"]["reason"]
    );
    assert_eq!(v["data"]["published_targets"].as_array().unwrap().len(), 0);
}

/// A blank `--reason` is a caller-fixable input error (a journaled reason must be
/// meaningful).
#[test]
fn release_abandon_rejects_a_blank_reason() {
    let dir = seed_journal("RUN01", &[RUN_CREATED_A.replace("RUNA", "RUN01").as_str()]);
    let out = ossctl()
        .args([
            "release",
            "abandon",
            "RUN01",
            "--reason",
            "   ",
            "--json",
            "--journal-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "invalid_reason");
}

/// A `--reason` with embedded control characters (a newline) is rejected — it is
/// journaled durably and rendered on one line, so it must stay single-line text.
#[test]
fn release_abandon_rejects_a_control_character_reason() {
    let dir = seed_journal("RUN01", &[RUN_CREATED_A.replace("RUNA", "RUN01").as_str()]);
    let out = ossctl()
        .args(["release", "abandon", "RUN01", "--reason"])
        .arg("line one\nrun OTHER completed")
        .args(["--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "invalid_reason");
    // The journal must be untouched: the run is still in-flight, not abandoned.
    let show = ossctl()
        .args(["release", "show", "RUN01", "--json", "--journal-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    let sv: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(sv["data"]["state"]["status"], "in_progress");
}

/// A terminal run cannot be abandoned: a completed run and an already-abandoned
/// run each refuse with their own caller-fixable code, and no second terminal fact
/// is appended.
#[test]
fn release_abandon_refuses_terminal_runs() {
    // A completed run (v1 `tag ok`).
    let completed = seed_journal(
        "DONE1",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"DONE1","plan_id":"plan-abc","version":"1.0.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"phase_completed:tag","kind":"phase_completed","phase":"tag","outcome":"ok"}"#,
        ],
    );
    let out = ossctl()
        .args([
            "release",
            "abandon",
            "DONE1",
            "--reason",
            "too late",
            "--json",
            "--journal-dir",
        ])
        .arg(completed.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "completed → user error: {out:?}"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "run_completed");

    // An already-abandoned run.
    let abandoned = seed_journal(
        "GONE1",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"GONE1","plan_id":"plan-abc","version":"1.0.0","targets":["cargo"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"run_abandoned","kind":"run_abandoned","reason":"first"}"#,
        ],
    );
    let out = ossctl()
        .args([
            "release",
            "abandon",
            "GONE1",
            "--reason",
            "again",
            "--json",
            "--journal-dir",
        ])
        .arg(abandoned.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "run_already_abandoned");
}

/// Abandoning a run with no journal is a caller-fixable (exit 1) error, and a
/// path-traversal run id is rejected before any journal work.
#[test]
fn release_abandon_unknown_and_bad_run_ids_are_user_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = ossctl()
        .args([
            "release",
            "abandon",
            "NOPE",
            "--reason",
            "x",
            "--json",
            "--journal-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&missing.stderr).unwrap();
    assert_eq!(v["error"]["code"], "run_not_found");

    let bad = ossctl()
        .args([
            "release",
            "abandon",
            "../escape",
            "--reason",
            "x",
            "--json",
            "--journal-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&bad.stderr).unwrap();
    assert_eq!(v["error"]["code"], "invalid_run_id");
}

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
    // The manifest carries the release version — the single source of truth now that
    // `--version` is gone (`release-drop-version-flag`). Without a version here the
    // fail-closed guard (`version-source-fail-closed-nonrust`) would refuse to plan.
    std::fs::write(
        dir.path().join("Cargo.toml"),
        // Package name matches the fixture's explicit target (`package: rg`) so facts
        // resolves the manifest version for it.
        "[package]\nname = \"rg\"\nversion = \"1.0.0\"\n",
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
        .args(["release", "cut", "--plan", "deadbeef", "--repo-root"])
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

/// Editing the manifest version AFTER sealing a plan is drift: the version is part
/// of the content address (derived from the manifest — the single source of truth,
/// `release-drop-version-flag`), so a bumped manifest hashes to a different `plan_id`
/// and the cut refuses rather than publishing an unapproved version. This replaces
/// the old `--version`-differs-from-sealed check: there is no longer a version flag
/// to differ, so drift now enters only through the manifest.
#[test]
fn release_cut_refuses_when_the_manifest_version_changed_since_sealing() {
    let dir = approved_git_repo(); // manifest at 1.0.0
    let journal = tempfile::tempdir().unwrap();

    // Seal a real plan at the manifest version (1.0.0) and read its plan_id.
    let planned = ossctl()
        .args(["release", "plan", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        planned.status.code(),
        Some(0),
        "plan should succeed on an approved repo: {planned:?}"
    );
    let pv: serde_json::Value =
        serde_json::from_slice(&planned.stdout).expect("plan stdout is JSON");
    let plan_id = pv["data"]["plan_id"]
        .as_str()
        .expect("plan_id present")
        .to_string();

    // Bump the manifest version in the working tree (a release bump after sealing).
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"rg\"\nversion = \"2.0.0\"\n",
    )
    .unwrap();

    // Execute the sealed plan_id; the cut re-derives 2.0.0 from the manifest, hashes a
    // different plan_id → drift refusal.
    let out = ossctl()
        .args(["release", "cut", "--plan", &plan_id, "--repo-root"])
        .arg(dir.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "changed version → plan_stale");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "plan_stale");
    assert!(out.stdout.is_empty());
}

/// `release plan --bump minor` (opt-in, `release-rust-workspace-multicrate` f2):
/// the engine COMPUTES the new version from the manifest (1.0.0) + the level and
/// seals a bump phase — proving the plan side end-to-end through the real binary.
#[test]
fn release_plan_bump_computes_the_version_and_seals_a_bump_phase() {
    let dir = approved_git_repo(); // manifest at 1.0.0
    let out = ossctl()
        .args([
            "release",
            "plan",
            "--json",
            "--bump",
            "minor",
            "--repo-root",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "bump plan should succeed: {out:?}"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("plan stdout JSON");
    let data = &v["data"];
    // The COMPUTED version (1.0.0 + minor = 1.1.0) is the sealed release version.
    assert_eq!(data["version"], "1.1.0");
    assert_eq!(data["bump"]["level"], "minor");
    assert_eq!(data["bump"]["from_version"], "1.0.0");
    assert_eq!(data["bump"]["to_version"], "1.1.0");
    assert_eq!(data["bump"]["changelog_finalize"], true);
    // The bump phase leads the pipeline.
    assert_eq!(data["phases"][0], "bump");
}

/// A `--bump`-less plan carries NO `bump` key (the additive-superset guarantee holds
/// through the wire envelope, not only in-memory).
#[test]
fn release_plan_without_bump_omits_the_bump_field() {
    let dir = approved_git_repo();
    let out = ossctl()
        .args(["release", "plan", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("plan stdout JSON");
    assert!(v["data"].get("bump").is_none(), "no --bump ⇒ no bump key");
    assert_eq!(v["data"]["phases"][0], "dry-run-all");
}

/// `release plan --bump bogus` is a strict, informative rejection (AI-first CLI:
/// closed-enum validation), never a silent fallback.
#[test]
fn release_plan_bump_rejects_a_bad_level() {
    let dir = approved_git_repo();
    let out = ossctl()
        .args([
            "release",
            "plan",
            "--json",
            "--bump",
            "bugfix",
            "--repo-root",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "invalid_bump");
    assert_eq!(v["error"]["invalid_value"], "bugfix");
}

/// `release cut` on a sealed bump plan fails CLOSED (`bump_execution_unimplemented`):
/// the plan side has landed but cut-time execution of the bump phase is a follow-up
/// validated by a real cut. Refusing here prevents building/publishing the un-bumped
/// manifest version — the exact partial-publish footgun the feature exists to prevent.
#[test]
fn release_cut_of_a_bump_plan_fails_closed() {
    let dir = approved_git_repo(); // manifest at 1.0.0
    let journal = tempfile::tempdir().unwrap();

    // Seal a --bump plan and read its computed plan_id.
    let planned = ossctl()
        .args([
            "release",
            "plan",
            "--json",
            "--bump",
            "patch",
            "--repo-root",
        ])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        planned.status.code(),
        Some(0),
        "bump plan should seal: {planned:?}"
    );
    let pv: serde_json::Value = serde_json::from_slice(&planned.stdout).expect("plan JSON");
    let plan_id = pv["data"]["plan_id"].as_str().expect("plan_id").to_string();

    // Cut it with the SAME --bump (so the drift check passes) → fail closed on execution.
    let out = ossctl()
        .args([
            "release",
            "cut",
            "--plan",
            &plan_id,
            "--bump",
            "patch",
            "--repo-root",
        ])
        .arg(dir.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "bump cut must refuse: {out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "bump_execution_unimplemented");
    assert!(
        out.stdout.is_empty(),
        "no journal events emitted on a closed refusal"
    );
}

// ── release resume: reconcile decisions that refuse/short-circuit offline ─────

/// A resume against a run with no journal is a caller-fixable (exit 1) error.
#[test]
fn release_resume_unknown_run_is_user_error() {
    let repo = tempfile::tempdir().unwrap();
    let journal = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["release", "resume", "NOPE", "--json", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "run_not_found");
    assert!(out.stdout.is_empty());
}

/// A `run_id` that is not a single path segment is rejected (no path traversal).
#[test]
fn release_resume_rejects_bad_run_id() {
    let repo = tempfile::tempdir().unwrap();
    let journal = tempfile::tempdir().unwrap();
    let out = ossctl()
        .args(["release", "resume", "../escape", "--json", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "invalid_run_id");
}

/// Resuming an already-`completed` run is idempotent success (exit 0) — it
/// short-circuits before touching the repo or any registry. This is a **v1**
/// journal (ended at `Tag Ok`, no Dist phase, `schema_version` 1): the reducer still
/// reads it as completed (back-compat), so an upgraded binary short-circuits rather
/// than trying to re-plan under the new seal version.
#[test]
fn release_resume_completed_run_is_idempotent_success() {
    let repo = tempfile::tempdir().unwrap();
    let journal = seed_journal(
        "RUNC",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNC","plan_id":"plan-done","version":"1.0.0","targets":["rust"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"phase_completed:tag","kind":"phase_completed","phase":"tag","outcome":"ok"}"#,
        ],
    );
    let out = ossctl()
        .args(["release", "resume", "RUNC", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "completed run resume must exit 0: {out:?}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("already complete"));
}

/// Resuming an `abandoned` run is refused (exit 1) — it was deliberately marked
/// un-resumable.
#[test]
fn release_resume_abandoned_run_is_refused() {
    let repo = tempfile::tempdir().unwrap();
    let journal = seed_journal(
        "RUNA",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNA","plan_id":"plan-x","version":"1.0.0","targets":["rust"]}"#,
            r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"run_abandoned","kind":"run_abandoned","reason":"OTP timeout"}"#,
        ],
    );
    let out = ossctl()
        .args(["release", "resume", "RUNA", "--json", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "run_abandoned");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("OTP timeout"));
}

/// A resume whose run was sealed against a `plan_id` the current repo no longer
/// hashes to is refused with `resume_drift` — before any reconcile or publish.
#[test]
fn release_resume_refuses_a_drifted_repo() {
    let repo = approved_git_repo();
    let journal = seed_journal(
        "RUND",
        &[
            r#"{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUND","plan_id":"plan-mismatch","version":"1.0.0","targets":["rust"]}"#,
        ],
    );
    let out = ossctl()
        .args(["release", "resume", "RUND", "--json", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "drifted repo → resume_drift");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "resume_drift");
    assert_eq!(v["error"]["expected"]["sealed_plan_id"], "plan-mismatch");
    assert!(out.stdout.is_empty(), "a refused resume published nothing");
}

/// A single-python-target approved repo whose target's package is explicit (so the
/// plan resolves without facts and validates), inside a one-commit git repo. Python
/// has no wired registry query, so a resume reconciles its publish to `unknown`
/// offline — the deterministic path to a `resume_conflict`.
fn approved_python_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let contract = std::fs::read_to_string(fixture("python-lib").join("OSS-RELEASE.md"))
        .unwrap()
        .replace("status: draft", "status: approved")
        .replace(
            "ecosystems: [python]",
            "ecosystems: [python]\ntargets:\n  - {ecosystem: python, package: mytool, registry: pypi, adapter: twine}",
        );
    std::fs::write(dir.path().join("OSS-RELEASE.md"), contract).unwrap();
    // A pyproject carrying the package name + version — the manifest is the single
    // source of truth for the release version (`release-drop-version-flag`), and
    // pypi is a manifest-versioned registry, so a missing version would now fail the
    // cut closed (`version-source-fail-closed-nonrust`) rather than be supplied by a
    // flag.
    std::fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = \"mytool\"\nversion = \"1.0.0\"\n",
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

/// A resume that reconciles a recorded publish to `unknown` (python has no wired
/// registry query yet — offline) refuses with `resume_conflict` rather than
/// proceed on an unverifiable target; the blocking target is surfaced.
#[test]
fn release_resume_refuses_an_unverifiable_target() {
    let repo = approved_python_repo();

    // Seal a real plan so the resume's drift check passes; reuse its id + version.
    let planned = ossctl()
        .args(["release", "plan", "--json", "--repo-root"])
        .arg(repo.path())
        .output()
        .unwrap();
    assert_eq!(
        planned.status.code(),
        Some(0),
        "plan should succeed: {planned:?}"
    );
    let pv: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    let plan_id = pv["data"]["plan_id"].as_str().unwrap().to_string();

    let run_created = format!(
        r#"{{"schema_version":1,"seq":1,"ts":1000,"idempotency_key":"run_created","kind":"run_created","run_id":"RUNU","plan_id":"{plan_id}","version":"1.0.0","targets":["python"]}}"#
    );
    let published = r#"{"schema_version":1,"seq":2,"ts":1001,"idempotency_key":"published:python","kind":"target_published","target":"python","receipt":{"ecosystem":"python","package":"mytool","version":"1.0.0","registry_url":null,"digest":null}}"#;
    let journal = seed_journal("RUNU", &[&run_created, published]);

    let out = ossctl()
        .args(["release", "resume", "RUNU", "--json", "--repo-root"])
        .arg(repo.path())
        .arg("--journal-dir")
        .arg(journal.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "unverifiable target → resume_conflict"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(v["error"]["code"], "resume_conflict");
    let problems = v["error"]["problems"].as_array().expect("problems listed");
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().unwrap().contains("rust")),
        "the blocking target must be surfaced: {problems:?}"
    );
    assert!(out.stdout.is_empty());
}

/// A version that would produce an invalid git tag (`v{version}`) is rejected up
/// front — before any repo work — so it can never publish and then fail at tag
/// time (post-publish, unrecoverable-late).
#[test]
fn release_cut_rejects_a_git_ref_unsafe_version() {
    for bad in ["1.0..0", "1.0.0~1", "../evil", "1.0.0.lock"] {
        // The version is derived from the manifest now (`release-drop-version-flag`),
        // so a tag-unsafe version must be planted there, not passed as a flag. The cut
        // re-derives it and validates its shape before any repo/journal work.
        let dir = approved_git_repo();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!("[package]\nname = \"rg\"\nversion = \"{bad}\"\n"),
        )
        .unwrap();
        let out = ossctl()
            .args(["release", "cut", "--plan", "x", "--repo-root"])
            .arg(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "version {bad:?} should be rejected"
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
        assert_eq!(v["error"]["code"], "invalid_version", "version {bad:?}");
    }
}

// ── dist generate ────────────────────────────────────────────────────────────

/// A minimal approved contract with a cargo-dist distribution block (platforms
/// omitted → the cross-platform default), written into `dir`.
fn write_dist_contract(dir: &std::path::Path, installers: &str) {
    let doc = format!(
        "---\n\
         schema_version: 1\n\
         status: approved\n\
         maturity: mvp\n\
         ecosystems: [rust]\n\
         versioning: semver\n\
         changelog: {{mode: fragment, source: issuectl-trailers}}\n\
         conventional_commits: false\n\
         release: {{model: gated, layout: single}}\n\
         contribution_provenance: none\n\
         provenance_level: keyless\n\
         dependency_bot: dependabot\n\
         health_badges: [ci]\n\
         license: MIT\n\
         docs_site: none\n\
         distribution:\n\
         \x20 adapter: cargo-dist\n\
         \x20 installers: [{installers}]\n\
         ---\n\n# Test\n"
    );
    std::fs::write(dir.join("OSS-RELEASE.md"), doc).unwrap();
}

/// `dist generate --no-workflow --json` writes the reference-shape config and
/// reports the resolved targets/installers inside the canonical envelope.
#[test]
fn dist_generate_writes_config_and_reports_json() {
    let dir = tempfile::tempdir().unwrap();
    write_dist_contract(dir.path(), "shell, powershell");

    let out = ossctl()
        .args(["dist", "generate", "--no-workflow", "--json", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "exit 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];
    assert_eq!(data["dist_config"], "dist-workspace.toml");
    assert_eq!(
        data["workflow"],
        serde_json::Value::Null,
        "--no-workflow → null"
    );
    assert_eq!(data["cargo_dist_version"], "0.28.2");
    assert_eq!(
        data["installers"],
        serde_json::json!(["shell", "powershell"])
    );
    // The cross-platform default covers macOS AND Linux (never macOS-only).
    let targets = data["targets"].as_array().unwrap();
    assert!(
        targets
            .iter()
            .any(|t| t.as_str().unwrap().contains("linux")),
        "Linux target: {data}"
    );
    assert!(
        targets
            .iter()
            .any(|t| t.as_str().unwrap().contains("darwin")),
        "macOS target: {data}"
    );

    // The config landed on disk with the pinned reference shape.
    let toml = std::fs::read_to_string(dir.path().join("dist-workspace.toml")).unwrap();
    assert!(toml.contains("pr-run-mode = \"skip\""), "{toml}");
    assert!(toml.contains("github-attestations = true"), "{toml}");
    assert!(
        !toml.contains("github-custom-runners"),
        "no personal runner override: {toml}"
    );
}

/// A contract without a `distribution` block is a user error (exit 1) and writes
/// nothing.
#[test]
fn dist_generate_without_distribution_is_user_error() {
    let dir = tempfile::tempdir().unwrap();
    // A registry-only contract (no distribution block).
    std::fs::copy(
        fixture("solo-rust-cli").join("OSS-RELEASE.md"),
        dir.path().join("OSS-RELEASE.md"),
    )
    .unwrap();

    let out = ossctl()
        .args(["dist", "generate", "--no-workflow", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "user error → exit 1");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "no_distribution");
    assert!(
        !dir.path().join("dist-workspace.toml").exists(),
        "nothing written"
    );
}

/// A monorepo contract with multiple `distributions:` is a user error (exit 1):
/// `dist generate` scaffolds one `dist-workspace.toml` and cannot yet target a
/// single package of a monorepo. Nothing is written.
#[test]
fn dist_generate_with_multiple_distributions_is_user_error() {
    let dir = tempfile::tempdir().unwrap();
    let doc = "---\n\
        schema_version: 2\n\
        status: approved\n\
        maturity: mvp\n\
        ecosystems: [rust]\n\
        versioning: semver\n\
        changelog: {mode: fragment, source: issuectl-trailers}\n\
        release: {model: gated, layout: monorepo}\n\
        health_badges: [ci]\n\
        license: MIT\n\
        distributions:\n\
        \x20 - {package: alpha, adapter: cargo-dist}\n\
        \x20 - {package: beta, adapter: cargo-dist}\n\
        ---\n\n# Test\n";
    std::fs::write(dir.path().join("OSS-RELEASE.md"), doc).unwrap();

    let out = ossctl()
        .args(["dist", "generate", "--no-workflow", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "user error → exit 1");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "multiple_distributions");
    assert!(
        !dir.path().join("dist-workspace.toml").exists(),
        "nothing written"
    );
}

/// An existing config is not clobbered without `--force`.
#[test]
fn dist_generate_refuses_to_clobber_without_force() {
    let dir = tempfile::tempdir().unwrap();
    write_dist_contract(dir.path(), "shell");
    std::fs::write(dir.path().join("dist-workspace.toml"), "# mine\n").unwrap();

    let out = ossctl()
        .args(["dist", "generate", "--no-workflow", "--repo-root"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON");
    assert_eq!(v["error"]["code"], "dist_config_exists");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("dist-workspace.toml")).unwrap(),
        "# mine\n",
        "the hand-tuned config is preserved"
    );
}

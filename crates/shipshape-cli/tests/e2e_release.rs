mod e2e;

use std::fs;
use std::process::Command;

use e2e::{error_code, json, only_run_id, plan_id, Shims, TempRepo};

#[test]
fn plan_is_deterministic_and_has_the_release_shape() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();

    let first = repo.run(&shims, &["--json", "release", "plan"]);
    let second = repo.run(&shims, &["--json", "release", "plan"]);

    assert!(first.status.success(), "first plan failed: {first:?}");
    assert!(second.status.success(), "second plan failed: {second:?}");
    let first = json(&first);
    let second = json(&second);
    assert_eq!(first["data"]["plan_id"], second["data"]["plan_id"]);
    assert_eq!(first["data"]["version"], "0.1.0");
    assert_eq!(
        first["data"]["phases"],
        serde_json::json!([
            "dry-run-all",
            "build-all",
            "publish-all",
            "tag",
            "dist",
            "verify",
            "advance-branch"
        ])
    );
    assert!(
        shims.log().is_empty(),
        "planning must not run external shims"
    );
}

#[test]
fn cut_refuses_a_draft_contract() {
    let repo = TempRepo::new("draft");
    let shims = Shims::new();

    let output = repo.run(
        &shims,
        &["--json", "release", "cut", "--plan", "irrelevant"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_code(&output), "not_approved");
    assert!(
        shims.log().is_empty(),
        "approval refusal must happen before effects"
    );
}

#[test]
fn cut_with_the_wrong_plan_id_reports_plan_stale() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();

    let plan = plan_id(&repo, &shims);
    repo.append_commit("drift.txt", "the sealed HEAD has moved\n");
    let output = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_code(&output), "plan_stale");
    assert!(
        shims.log().is_empty(),
        "plan drift refusal must happen before effects"
    );
}

#[test]
fn dry_run_failure_is_journaled_and_never_tags() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    shims.fail_after_version_probe("cargo", "simulated cargo failure");

    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);

    assert_eq!(cut.status.code(), Some(2));
    assert_eq!(error_code(&cut), "release_failed");
    shims.assert_called("cargo");
    let run_id = only_run_id(&repo);
    let journal = fs::read_to_string(repo.journal_dir().join(&run_id).join("journal.jsonl"))
        .expect("read journal");
    assert!(journal.contains("phase_completed"));
    assert!(journal.contains("dry_run"));
    assert!(journal.contains("failed"));

    let list = repo.run(&shims, &["--json", "release", "list"]);
    assert!(list.status.success(), "list failed: {list:?}");
    assert_eq!(json(&list)["data"]["in_flight_count"], 1);
    assert_eq!(json(&list)["data"]["runs"][0]["run_id"], run_id);

    let show = repo.run(&shims, &["--json", "release", "show", &run_id]);
    assert!(show.status.success(), "show failed: {show:?}");
    assert_eq!(
        json(&show)["data"]["state"]["phases"],
        serde_json::json!([{"phase": "dry_run", "outcome": "failed"}])
    );
    assert!(!Command::new("git")
        .args(["rev-parse", "--verify", "refs/tags/v0.1.0"])
        .current_dir(repo.path())
        .output()
        .expect("query git tag")
        .status
        .success());
}

#[test]
fn a_publish_none_repo_plans_and_cuts_a_tag_only_release() {
    // End to end for the private, never-published repo: the contract's explicit
    // `targets: []` survives into the plan (no phantom crates.io target), the plan
    // says so, and the cut tags without publishing anything or creating a GitHub
    // Release — no `cargo` and no `gh` process is ever spawned.
    let repo = TempRepo::new("approved");
    repo.use_publish_none_contract();
    let shims = Shims::new();

    let plan = repo.run(&shims, &["--json", "release", "plan"]);
    assert!(plan.status.success(), "plan failed: {plan:?}");
    let plan_json = json(&plan);
    assert_eq!(plan_json["data"]["targets"], serde_json::json!([]));
    let warnings = plan_json["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or_default().contains("git tag only")),
        "the plan must say it is tag-only: {warnings:?}"
    );

    let plan_id = plan_json["data"]["plan_id"].as_str().expect("plan id");
    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", plan_id]);
    assert!(cut.status.success(), "tag-only cut failed: {cut:?}");

    // The tag landed …
    assert!(Command::new("git")
        .args(["rev-parse", "--verify", "refs/tags/v0.1.0"])
        .current_dir(repo.path())
        .output()
        .expect("query git tag")
        .status
        .success());
    // … and nothing else did: no publish, no Release, no external publisher invoked.
    assert!(
        shims.log().is_empty(),
        "a tag-only cut ran external publishers: {}",
        shims.log()
    );
    let run_id = only_run_id(&repo);
    let journal = fs::read_to_string(repo.journal_dir().join(&run_id).join("journal.jsonl"))
        .expect("read journal");
    for forbidden in [
        "target_published",
        "target_delegated",
        "github_release_created",
        "github_release_delegated",
    ] {
        assert!(
            !journal.contains(forbidden),
            "tag-only cut journalled {forbidden}:\n{journal}"
        );
    }
    assert!(journal.contains("tag_pushed_remote"));
    assert!(journal.contains("default_branch_advanced"));
    let verify_ok = journal.lines().any(|line| {
        let event: serde_json::Value = serde_json::from_str(line).expect("journal event JSON");
        event["kind"] == "phase_completed" && event["phase"] == "verify" && event["outcome"] == "ok"
    });
    assert!(
        verify_ok,
        "verify must complete (nothing to observe is not Unknown):\n{journal}"
    );
}

#[test]
fn cargo_dist_version_mismatch_refuses_before_journal_or_repository_mutation() {
    let repo = TempRepo::new("approved");
    repo.use_cargo_dist_target();
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    let manifest_before = fs::read_to_string(repo.path().join("Cargo.toml")).unwrap();
    let contract_before = fs::read_to_string(repo.path().join("OSS-RELEASE.md")).unwrap();
    shims.set("dist", 0, "cargo-dist 0.31.0\n");

    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);

    assert_eq!(cut.status.code(), Some(2));
    assert_eq!(error_code(&cut), "release_dependency_missing");
    let stderr = String::from_utf8_lossy(&cut.stderr);
    assert!(stderr.contains("cargo-dist 0.32.0"), "{stderr}");
    assert!(
        stderr.contains("cargo install cargo-dist --version 0.32.0 --locked"),
        "{stderr}"
    );
    let release_run_count = fs::read_dir(repo.journal_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert_eq!(
        release_run_count, 0,
        "dependency refusal created a release journal"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("Cargo.toml")).unwrap(),
        manifest_before
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("OSS-RELEASE.md")).unwrap(),
        contract_before
    );
    assert!(!Command::new("git")
        .args(["rev-parse", "--verify", "refs/tags/v0.1.0"])
        .current_dir(repo.path())
        .output()
        .expect("query git tag")
        .status
        .success());
    assert!(
        shims
            .log()
            .lines()
            .all(|line| line.contains(" <--version>")),
        "preflight ran a mutating tool command: {}",
        shims.log()
    );
}

#[test]
fn resume_rechecks_cargo_dist_before_retrying_an_incomplete_build() {
    let repo = TempRepo::new("approved");
    repo.use_cargo_dist_target();
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    shims.set_script(
        "dist",
        r#"#!/bin/sh
printf 'dist' >> "$SHIM_DIR/log"
for arg in "$@"; do printf ' <%s>' "$arg" >> "$SHIM_DIR/log"; done
printf '\n' >> "$SHIM_DIR/log"
if [ "$1" = "--version" ]; then
  printf 'cargo-dist 0.32.0\n'
  exit 0
fi
printf 'simulated dist build failure\n' >&2
exit 1
"#,
    );
    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);
    assert_eq!(error_code(&cut), "release_failed");
    let run_id = only_run_id(&repo);
    let journal_path = repo.journal_dir().join(&run_id).join("journal.jsonl");
    let journal_before = fs::read_to_string(&journal_path).unwrap();
    assert!(journal_before.contains("\"phase\":\"build\""));
    shims.set_script("dist", "#!/bin/sh\nprintf 'cargo-dist 0.31.0\\n'\nexit 0\n");

    let resume = repo.run(&shims, &["--json", "release", "resume", &run_id]);

    assert_eq!(resume.status.code(), Some(2));
    assert_eq!(error_code(&resume), "release_dependency_missing");
    assert!(String::from_utf8_lossy(&resume.stderr).contains("cargo-dist 0.32.0"));
    assert_eq!(
        fs::read_to_string(journal_path).unwrap(),
        journal_before,
        "dependency refusal appended release events before the build retry"
    );
}

#[test]
fn delegated_release_with_zero_assets_fails_verify_and_is_posthoc_observable() {
    let repo = TempRepo::new("approved");
    repo.use_cargo_dist_target();
    let shims = Shims::new();
    shims.set_script(
        "gh",
        r#"#!/bin/sh
printf 'gh' >> "$SHIM_DIR/log"
for arg in "$@"; do printf ' <%s>' "$arg" >> "$SHIM_DIR/log"; done
printf '\n' >> "$SHIM_DIR/log"
if [ "$1 $2" = "run list" ]; then
  sha=$(git rev-parse HEAD)
  printf '[{"databaseId":42,"status":"completed","conclusion":"success","headBranch":"v0.1.0","headSha":"%s","url":"https://example/actions/runs/42"}]' "$sha"
elif [ "$1 $2" = "release view" ]; then
  printf '{"tagName":"v0.1.0","isDraft":false,"assets":[]}'
else
  printf '{}'
fi
"#,
    );
    let plan = plan_id(&repo, &shims);

    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);

    assert_eq!(cut.status.code(), Some(2));
    assert_eq!(error_code(&cut), "release_failed");
    let run_id = only_run_id(&repo);
    let journal = fs::read_to_string(repo.journal_dir().join(&run_id).join("journal.jsonl"))
        .expect("read journal");
    let verify_failed = journal.lines().any(|line| {
        let event: serde_json::Value = serde_json::from_str(line).expect("journal event JSON");
        event["kind"] == "phase_completed"
            && event["phase"] == "verify"
            && event["outcome"] == "failed"
    });
    assert!(
        verify_failed,
        "zero release assets must fail the verify barrier:\n{journal}"
    );

    let verify = repo.run(&shims, &["--json", "release", "verify", &run_id]);
    assert!(
        verify.status.success(),
        "post-hoc verify failed: {verify:?}"
    );
    let report = json(&verify);
    assert_eq!(report["data"]["summary"]["reconciled"], 1);
    assert_eq!(report["data"]["summary"]["missing"], 1);
    assert_eq!(report["data"]["targets"][0]["outcome"], "missing");
    shims.assert_called("gh");
}

#[test]
fn abandon_marks_a_failed_run_terminally() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    shims.fail_after_version_probe("cargo", "simulated cargo failure");
    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);
    assert!(!cut.status.success());
    shims.assert_called("cargo");
    let run_id = only_run_id(&repo);

    let abandon = repo.run(&shims, &["--json", "release", "abandon", &run_id]);

    assert!(abandon.status.success(), "abandon failed: {abandon:?}");
    assert_eq!(json(&abandon)["data"]["status"], "abandoned");
}

#[test]
fn cut_refuses_when_the_single_active_lock_exists() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    fs::create_dir_all(repo.journal_dir()).expect("create journal directory");
    fs::write(repo.journal_dir().join(".lock"), "test lock\n").expect("write active lock");

    let cut = repo.run(&shims, &["--json", "release", "cut", "--plan", &plan]);

    assert_eq!(cut.status.code(), Some(1));
    assert_eq!(error_code(&cut), "cut_in_progress");
    assert!(
        shims.log().is_empty(),
        "lock refusal must happen before effects"
    );
}

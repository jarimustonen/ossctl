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
        serde_json::json!(["dry-run-all", "build-all", "publish-all", "tag", "dist"])
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
    shims.set("cargo", 1, "simulated cargo failure\n");

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
fn abandon_marks_a_failed_run_terminally() {
    let repo = TempRepo::new("approved");
    let shims = Shims::new();
    let plan = plan_id(&repo, &shims);
    shims.set("cargo", 1, "simulated cargo failure\n");
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

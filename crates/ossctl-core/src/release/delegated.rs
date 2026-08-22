//! Read-only observation of GitHub Actions runs that own delegated publishes.
//!
//! The destination remains the final authority for delivery, but a GitHub-backed
//! delegated target first resolves the exact tag-triggered workflow run. This keeps
//! "still building" and "workflow failed" from being misreported as a missing
//! destination. Non-GitHub delegated adapters do not use this observer.

use std::fmt::Write as _;

use serde::Deserialize;

use crate::contract::schema::Adapter;
use crate::protocol::reconcile::{DelegatedJobFailure, DelegatedRun, DelegatedRunStatus};

use super::adapters::EffectCtx;

/// Observe the workflow run associated with `v{version}` for a GitHub-backed
/// delegated adapter. Returns `None` for adapters whose delegation is not owned by
/// GitHub Actions.
#[must_use]
pub fn observe_github_run(
    ctx: &EffectCtx<'_>,
    adapter: Adapter,
    version: &str,
) -> Option<DelegatedRun> {
    let workflow = match adapter {
        Adapter::CargoDist => ".github/workflows/release.yml".to_string(),
        Adapter::CargoPublishCi => match cargo_publish_workflow(ctx) {
            Ok(path) => path,
            Err(detail) => return Some(DelegatedRun::unknown(None, None, detail)),
        },
        _ => return None,
    };
    Some(observe_workflow(ctx, &workflow, version))
}

fn cargo_publish_workflow(ctx: &EffectCtx<'_>) -> Result<String, String> {
    let output = ctx
        .runner
        .run(
            "git",
            &[
                "grep",
                "-l",
                "-e",
                "cargo publish",
                "--",
                ".github/workflows/*.yml",
                ".github/workflows/*.yaml",
            ],
            ctx.repo_root,
        )
        .map_err(|error| format!("could not inspect tag-triggered workflows: {error}"))?;
    if output.status != Some(0) && output.status != Some(1) {
        return Err(format!(
            "could not inspect tag-triggered workflows: {}",
            output.stderr.trim()
        ));
    }
    let mut candidates: Vec<String> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(
            "no tracked GitHub Actions workflow containing `cargo publish` could be resolved for the cargo-publish-ci target"
                .to_string(),
        ),
        many => Err(format!(
            "more than one GitHub Actions workflow contains `cargo publish`; cannot identify the delegated owner: {}",
            many.join(", ")
        )),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRow {
    database_id: u64,
    status: String,
    conclusion: String,
    head_branch: String,
    head_sha: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunView {
    conclusion: String,
    url: String,
    jobs: Vec<JobRow>,
}

#[derive(Deserialize)]
struct JobRow {
    name: String,
    status: String,
    conclusion: String,
}

#[allow(clippy::too_many_lines)] // one linear fail-closed query/classification pipeline
fn observe_workflow(ctx: &EffectCtx<'_>, workflow: &str, version: &str) -> DelegatedRun {
    let tag = format!("v{version}");
    let expected_sha = match ctx
        .runner
        .run("git", &["rev-list", "-n", "1", &tag], ctx.repo_root)
    {
        Ok(output) if output.status == Some(0) && !output.stdout.trim().is_empty() => {
            output.stdout.trim().to_string()
        }
        Ok(output) => {
            return DelegatedRun::unknown(
                Some(workflow.to_string()),
                None,
                format!(
                    "could not resolve commit for tag `{tag}`: {}",
                    output.stderr.trim()
                ),
            )
        }
        Err(error) => {
            return DelegatedRun::unknown(
                Some(workflow.to_string()),
                None,
                format!("could not resolve commit for tag `{tag}`: {error}"),
            )
        }
    };
    let workflow_id = workflow.rsplit('/').next().unwrap_or(workflow);
    let output = match ctx.runner.run(
        "gh",
        &[
            "run",
            "list",
            "--workflow",
            workflow_id,
            "--branch",
            &tag,
            "--event",
            "push",
            "--json",
            "databaseId,status,conclusion,headBranch,headSha,url",
            "--limit",
            "20",
        ],
        ctx.repo_root,
    ) {
        Ok(output) => output,
        Err(error) => {
            return DelegatedRun::unknown(
                Some(workflow.to_string()),
                None,
                format!("could not query GitHub Actions runs for `{workflow}`: {error}"),
            )
        }
    };
    if output.status != Some(0) {
        return DelegatedRun::unknown(
            Some(workflow.to_string()),
            None,
            format!(
                "could not query GitHub Actions runs for `{workflow}`: {}",
                output.stderr.trim()
            ),
        );
    }
    let rows: Vec<RunRow> = match serde_json::from_str(&output.stdout) {
        Ok(rows) => rows,
        Err(error) => {
            return DelegatedRun::unknown(
                Some(workflow.to_string()),
                None,
                format!("GitHub Actions returned an unreadable run list for `{workflow}`: {error}"),
            )
        }
    };
    let mut matching = rows
        .into_iter()
        .filter(|run| run.head_branch == tag && run.head_sha == expected_sha);
    let Some(run) = matching.next() else {
        return DelegatedRun {
            provider: "github-actions".to_string(),
            workflow: Some(workflow.to_string()),
            run_id: None,
            url: None,
            status: DelegatedRunStatus::Pending,
            conclusion: None,
            failed_jobs: Vec::new(),
            detail: Some(format!(
                "the `{workflow}` run for tag `{tag}` at commit `{expected_sha}` is not visible yet"
            )),
        };
    };
    if matching.next().is_some() {
        return DelegatedRun::unknown(
            Some(workflow.to_string()),
            None,
            format!(
                "multiple `{workflow}` runs match tag `{tag}` at commit `{expected_sha}`; refusing to guess which run owns this release"
            ),
        );
    }
    classify_run(ctx, workflow, run)
}

#[allow(clippy::too_many_lines)] // keeps terminal run + job evidence assembled in one place
fn classify_run(ctx: &EffectCtx<'_>, workflow: &str, run: RunRow) -> DelegatedRun {
    if run.status != "completed" {
        return DelegatedRun {
            provider: "github-actions".to_string(),
            workflow: Some(workflow.to_string()),
            run_id: Some(run.database_id),
            url: Some(run.url),
            status: DelegatedRunStatus::Pending,
            conclusion: None,
            failed_jobs: Vec::new(),
            detail: Some(format!("the delegated workflow run is {}", run.status)),
        };
    }
    if run.conclusion == "success" {
        return DelegatedRun {
            provider: "github-actions".to_string(),
            workflow: Some(workflow.to_string()),
            run_id: Some(run.database_id),
            url: Some(run.url),
            status: DelegatedRunStatus::Success,
            conclusion: Some(run.conclusion),
            failed_jobs: Vec::new(),
            detail: None,
        };
    }
    let id = run.database_id.to_string();
    let view = ctx.runner.run(
        "gh",
        &["run", "view", &id, "--json", "status,conclusion,url,jobs"],
        ctx.repo_root,
    );
    let (conclusion, url, failed_jobs, extra) = match view {
        Ok(output) if output.status == Some(0) => {
            match serde_json::from_str::<RunView>(&output.stdout) {
                Ok(view) => {
                    let jobs = view
                        .jobs
                        .into_iter()
                        .filter(|job| {
                            job.status == "completed"
                                && !matches!(
                                    job.conclusion.as_str(),
                                    "success" | "skipped" | "neutral"
                                )
                        })
                        .map(|job| DelegatedJobFailure {
                            name: job.name,
                            conclusion: job.conclusion,
                        })
                        .collect();
                    (view.conclusion, Some(view.url), jobs, None)
                }
                Err(error) => (
                    run.conclusion.clone(),
                    Some(run.url.clone()),
                    Vec::new(),
                    Some(format!("could not parse the failed run's jobs: {error}")),
                ),
            }
        }
        Ok(output) => (
            run.conclusion.clone(),
            Some(run.url.clone()),
            Vec::new(),
            Some(format!(
                "could not inspect the failed run's jobs: {}",
                output.stderr.trim()
            )),
        ),
        Err(error) => (
            run.conclusion.clone(),
            Some(run.url.clone()),
            Vec::new(),
            Some(format!("could not inspect the failed run's jobs: {error}")),
        ),
    };
    let jobs = if failed_jobs.is_empty() {
        "no failed job detail was available".to_string()
    } else {
        failed_jobs
            .iter()
            .map(|job| format!("`{}` ({})", job.name, job.conclusion))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut detail = format!(
        "delegated workflow run {} ended `{conclusion}`; failed/cancelled job(s): {jobs}",
        run.database_id
    );
    if let Some(extra) = extra {
        let _ = write!(detail, "; {extra}");
    }
    DelegatedRun {
        provider: "github-actions".to_string(),
        workflow: Some(workflow.to_string()),
        run_id: Some(run.database_id),
        url,
        status: DelegatedRunStatus::Failed,
        conclusion: Some(conclusion),
        failed_jobs,
        detail: Some(detail),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::ports::{Clock, CommandOutput, CommandRunner, RegistryQuery};
    use crate::release::adapters::EMPTY_ARTIFACTS;

    struct ClockFake;
    impl Clock for ClockFake {
        fn now_unix(&self) -> u64 {
            0
        }
    }

    struct RegistryFake;
    impl RegistryQuery for RegistryFake {
        fn published_versions(&self, _ecosystem: &str, _package: &str) -> io::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    struct RunnerFake {
        run_list: String,
        run_view: Option<String>,
        calls: RefCell<Vec<String>>,
    }
    impl CommandRunner for RunnerFake {
        fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            let stdout = if program == "git" && args.starts_with(&["rev-list"]) {
                "abc123\n".to_string()
            } else if program == "git" && args.starts_with(&["grep"]) {
                ".github/workflows/publish-crates.yml\n".to_string()
            } else if program == "gh" && args.starts_with(&["run", "list"]) {
                self.run_list.clone()
            } else if program == "gh" && args.starts_with(&["run", "view"]) {
                self.run_view.clone().unwrap_or_default()
            } else {
                String::new()
            };
            Ok(CommandOutput {
                status: Some(0),
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn ctx<'a>(
        runner: &'a RunnerFake,
        clock: &'a ClockFake,
        registry: &'a RegistryFake,
    ) -> EffectCtx<'a> {
        EffectCtx {
            runner,
            clock,
            registry,
            repo_root: Path::new("/repo"),
            artifacts: &EMPTY_ARTIFACTS,
        }
    }

    #[test]
    fn in_progress_run_is_pending_not_missing() {
        let runner = RunnerFake {
            run_list: r#"[{"databaseId":77,"status":"in_progress","conclusion":"","headBranch":"v1.0.0","headSha":"abc123","url":"https://example/run/77"}]"#.to_string(),
            run_view: None,
            calls: RefCell::new(Vec::new()),
        };
        let (clock, registry) = (ClockFake, RegistryFake);
        let run = observe_github_run(
            &ctx(&runner, &clock, &registry),
            Adapter::CargoDist,
            "1.0.0",
        )
        .unwrap();
        assert_eq!(run.status, DelegatedRunStatus::Pending);
        assert_eq!(run.run_id, Some(77));
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.starts_with("gh release")));
    }

    #[test]
    fn cancelled_run_reports_the_terminal_job_cause() {
        let runner = RunnerFake {
            run_list: r#"[{"databaseId":88,"status":"completed","conclusion":"cancelled","headBranch":"v1.0.0","headSha":"abc123","url":"https://example/run/88"}]"#.to_string(),
            run_view: Some(r#"{"status":"completed","conclusion":"cancelled","url":"https://example/run/88","jobs":[{"name":"build (aarch64-unknown-linux-musl)","status":"completed","conclusion":"cancelled"},{"name":"host","status":"completed","conclusion":"skipped"}]}"#.to_string()),
            calls: RefCell::new(Vec::new()),
        };
        let (clock, registry) = (ClockFake, RegistryFake);
        let run = observe_github_run(
            &ctx(&runner, &clock, &registry),
            Adapter::CargoDist,
            "1.0.0",
        )
        .unwrap();
        assert_eq!(run.status, DelegatedRunStatus::Failed);
        assert_eq!(run.conclusion.as_deref(), Some("cancelled"));
        assert_eq!(
            run.failed_jobs[0].name,
            "build (aarch64-unknown-linux-musl)"
        );
        assert!(run.detail.unwrap().contains("cancelled"));
    }

    #[test]
    fn successful_run_is_distinct_and_allows_destination_observation_to_follow() {
        let runner = RunnerFake {
            run_list: r#"[{"databaseId":99,"status":"completed","conclusion":"success","headBranch":"v1.0.0","headSha":"abc123","url":"https://example/run/99"}]"#.to_string(),
            run_view: None,
            calls: RefCell::new(Vec::new()),
        };
        let (clock, registry) = (ClockFake, RegistryFake);
        let run = observe_github_run(
            &ctx(&runner, &clock, &registry),
            Adapter::CargoDist,
            "1.0.0",
        )
        .unwrap();
        assert_eq!(run.status, DelegatedRunStatus::Success);
        assert_eq!(run.run_id, Some(99));
    }

    #[test]
    fn command_failure_is_unknown_not_missing_or_pending() {
        struct Broken;
        impl CommandRunner for Broken {
            fn run(&self, program: &str, args: &[&str], _cwd: &Path) -> io::Result<CommandOutput> {
                if program == "git" && args.starts_with(&["rev-list"]) {
                    return Ok(CommandOutput {
                        status: Some(0),
                        stdout: "abc123\n".into(),
                        stderr: String::new(),
                    });
                }
                Err(io::Error::new(io::ErrorKind::TimedOut, "offline"))
            }
        }
        let (broken, clock, registry) = (Broken, ClockFake, RegistryFake);
        let context = EffectCtx {
            runner: &broken,
            clock: &clock,
            registry: &registry,
            repo_root: Path::new("/repo"),
            artifacts: &EMPTY_ARTIFACTS,
        };
        let run = observe_github_run(&context, Adapter::CargoDist, "1.0.0").unwrap();
        assert_eq!(run.status, DelegatedRunStatus::Unknown);
    }
}

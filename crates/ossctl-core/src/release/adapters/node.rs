//! Node ecosystem adapter: `release-please`, `changesets`, and `npm-publish`.
//!
//! Publishes packages to the npm registry. `npm-publish` publishes directly and
//! `changesets` runs `changeset publish` (both REAL host publishes).
//! `release-please` publishes on merge via a CI job keyed off the GitHub release,
//! never from this host, so its `publish` honestly returns
//! [`AdapterError::Unsupported`] rather than fabricating an npm receipt (matching
//! `cargo-dist` and `gh-action-pypi-publish`). `verify` reconciles against npm via
//! the default [`RegistryQuery`](crate::ports::RegistryQuery) path.

use std::time::Duration;

use serde::Deserialize;

use crate::contract::schema::Adapter;
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The node release adapter, operating as `release-please`, `changesets`, or
/// `npm-publish`.
pub struct NodeAdapter {
    adapter: Adapter,
}

impl NodeAdapter {
    /// Construct for a resolved node adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::ReleasePlease | Adapter::Changesets | Adapter::NpmPublish
        ));
        Self { adapter }
    }
}

impl ReleaseAdapter for NodeAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        let (planned_commands, notes) = match self.adapter {
            Adapter::Changesets => (
                vec![PlannedCommand::new("changeset", &["status", "--verbose"])],
                vec![],
            ),
            Adapter::ReleasePlease => (
                vec![PlannedCommand::new(
                    "release-please",
                    &["release-pr", "--dry-run"],
                )],
                vec!["release-please publishes on merge via CI, not from this host".to_string()],
            ),
            _ => (
                vec![PlannedCommand::new("npm", &["publish", "--dry-run"])],
                vec![],
            ),
        };
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands,
            notes,
        })
    }

    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        // `npm pack --json` reports the packed tarball's exact filename, which is
        // not always `{package}-{version}.tgz`: a scoped package `@scope/pkg` packs
        // to `scope-pkg-{version}.tgz`. Read the name npm actually produced so the
        // asset the coordinator threads into the binary / GitHub-Release upload set
        // is correct. (`--json` machine output requires npm ≥ 7.20.3.)
        let cmd = PlannedCommand::new("npm", &["pack", "--json"]);
        let outputs = run_all(ctx, std::slice::from_ref(&cmd))?;
        let stdout = outputs[0].stdout.trim();
        // A build that cannot identify its own artifact must fail hard, never guess:
        // reconstructing `{package}-{version}.tgz` is wrong for scoped packages (the
        // exact plausible-but-wrong artifact this engine must not fabricate), and a
        // guessed path only fails later, opaquely, at upload time. This mirrors how
        // the cargo adapter treats an unparseable `cargo metadata`.
        let Some(artifacts) = parse_pack_filenames(stdout).filter(|names| !names.is_empty()) else {
            return Err(AdapterError::Command {
                command: cmd.rendered(),
                code: None,
                stderr: format!(
                    "`npm pack --json` produced no parseable tarball filename \
                     (npm ≥ 7.20.3 is required for --json); output was: {stdout}"
                ),
            });
        };
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts,
            notes: vec![],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // release-please publishes on merge via a CI job keyed off the GitHub
        // release; there is no faithful host publish. Report that honestly rather
        // than running a representative command and returning a receipt for a
        // publish that did not happen (matching cargo-dist / gh-action-pypi-publish).
        if matches!(self.adapter, Adapter::ReleasePlease) {
            return Err(AdapterError::Unsupported {
                adapter: self.adapter,
                operation: "publish",
            });
        }
        // PER-TARGET IRREVERSIBLE.
        let cmds = match self.adapter {
            // `changeset publish` is workspace-wide — it publishes every unpublished
            // package in one shot — so the coordinator must drive a Changesets target
            // at most once per workspace; a truthful per-target receipt for a batch
            // publish is a broader change tracked as a spin-off (see the terminal
            // report). The command itself is the genuine publish, not a placeholder.
            Adapter::Changesets => vec![PlannedCommand::new("changeset", &["publish"])],
            _ => vec![PlannedCommand::new("npm", &["publish"])],
        };
        run_all(ctx, &cmds)?;
        let remote_url = Some(format!(
            "https://www.npmjs.com/package/{}/v/{}",
            t.package, t.version
        ));
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

/// One entry of `npm pack --json` output — only the packed tarball `filename` is
/// consumed (the exact name npm produced, which the coordinator threads into the
/// binary upload set).
#[derive(Deserialize)]
struct NpmPackEntry {
    filename: String,
}

/// Parse the packed tarball filename(s) from `npm pack --json` output. `None` when
/// the payload is empty or not the expected array shape; the caller then fails the
/// build rather than guessing a name (see [`NodeAdapter::build`]).
fn parse_pack_filenames(stdout: &str) -> Option<Vec<String>> {
    if stdout.is_empty() {
        return None;
    }
    let entries: Vec<NpmPackEntry> = serde_json::from_str(stdout).ok()?;
    Some(entries.into_iter().map(|e| e.filename).collect())
}

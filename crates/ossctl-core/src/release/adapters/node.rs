//! Node ecosystem adapter: `release-please`, `changesets`, and `npm-publish`.
//!
//! Publishes packages to the npm registry. `npm-publish` publishes directly;
//! `release-please` and `changesets` are release-automation tools whose publish
//! step is normally CI-driven, so their publish bodies here are clearly-marked
//! skeletons. `verify` reconciles against npm via the default [`RegistryQuery`]
//! path.

use std::time::Duration;

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
        t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        run_all(ctx, &[PlannedCommand::new("npm", &["pack"])])?;
        // SKELETON: a production build reads the tarball name npm reports.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![format!("{}-{}.tgz", t.package, t.version)],
            notes: vec![],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE.
        let cmds = match self.adapter {
            Adapter::Changesets => vec![PlannedCommand::new("changeset", &["publish"])],
            // SKELETON: release-please's release + npm publish is a CI job; the
            // representative host command creates the GitHub release it keys off.
            Adapter::ReleasePlease => {
                vec![PlannedCommand::new("release-please", &["github-release"])]
            }
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

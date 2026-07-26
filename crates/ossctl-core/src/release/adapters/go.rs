//! Go ecosystem adapter: `goreleaser`.
//!
//! Go modules are consumed straight from a pushed git tag (there is no upload to
//! a mutable registry), so a "publish" here is `GoReleaser` building and attaching
//! artifacts to the GitHub Release; module availability is fronted by the
//! immutable module proxy. `verify` uses the default [`RegistryQuery`] path
//! against `proxy.golang.org`.

use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The go release adapter, operating as `goreleaser`.
pub struct GoAdapter {
    adapter: Adapter,
}

impl GoAdapter {
    /// Construct for the resolved `goreleaser` adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(adapter, Adapter::Goreleaser));
        Self { adapter }
    }
}

impl ReleaseAdapter for GoAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands: vec![PlannedCommand::new(
                "goreleaser",
                &["release", "--snapshot", "--clean", "--skip=publish"],
            )],
            notes: vec![],
        })
    }

    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        run_all(
            ctx,
            &[PlannedCommand::new(
                "goreleaser",
                &["build", "--snapshot", "--clean"],
            )],
        )?;
        // SKELETON: a production build reads dist/artifacts.json for the exact
        // per-platform binary set.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec!["dist/".to_string()],
            notes: vec![],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE (attaches artifacts to the GitHub Release).
        run_all(
            ctx,
            &[PlannedCommand::new("goreleaser", &["release", "--clean"])],
        )?;
        Ok(make_receipt(ctx, t, None, None))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}

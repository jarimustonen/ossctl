//! Binary distribution adapter: `manual` / GitHub Releases.
//!
//! Attaches prebuilt binaries to a GitHub Release (`gh release`). GitHub
//! Releases are not observable through the
//! [`RegistryQuery`](crate::ports::RegistryQuery) port, so `verify` returns
//! [`VerifyOutcome::Unknown`] **explicitly** (ADR-0002 §1) — an honest "cannot
//! check", never a false `Missing`.

use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{
    BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt, VerifyOutcome,
};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The binary distribution adapter, operating as `manual` (GitHub Releases).
pub struct BinaryAdapter {
    adapter: Adapter,
}

impl BinaryAdapter {
    /// Construct for the resolved `manual` adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(adapter, Adapter::Manual));
        Self { adapter }
    }

    fn tag(t: &AdapterTarget) -> String {
        format!("v{}", t.version)
    }
}

impl ReleaseAdapter for BinaryAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        _ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands: vec![PlannedCommand::new(
                "gh",
                &["release", "view", &Self::tag(t)],
            )],
            notes: vec!["artifacts are built by the ecosystem's own build step and \
                 uploaded to the coordinator-owned GitHub Release"
                .to_string()],
        })
    }

    fn build(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        // The binary target uploads artifacts produced elsewhere; it has no
        // build phase of its own.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![],
            notes: vec![
                "binary target has no build phase (uploads prebuilt artifacts)".to_string(),
            ],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE (uploads assets to the release).
        // SKELETON: the concrete asset paths are threaded in via
        // `ctx.artifacts.assets` (gathered from every target's build); the real
        // upload is finished in `adapter-skeletons-finish`.
        //
        // Flags precede the `--` option terminator, and every asset path follows
        // it, so a path that happens to start with `-` is never mis-read as a flag.
        let tag = Self::tag(t);
        let mut args = vec!["release", "upload", tag.as_str(), "--clobber", "--"];
        args.extend(ctx.artifacts.assets.iter().map(String::as_str));
        run_all(ctx, &[PlannedCommand::new("gh", &args)])?;
        Ok(make_receipt(ctx, t, None, None))
    }

    fn verify(
        &self,
        _ctx: &EffectCtx<'_>,
        _receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        // GitHub Releases are not observable through RegistryQuery; report the
        // honest "cannot check" rather than a false Missing (ADR-0002 §1).
        Ok(VerifyOutcome::Unknown)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

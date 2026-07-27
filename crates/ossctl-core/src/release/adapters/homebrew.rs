//! Homebrew distribution adapter: `homebrew-tap` and `homebrew-core`.
//!
//! Updates a Homebrew formula (a custom tap, or a `homebrew-core` bump PR) so
//! `brew install` resolves the new version. A tap/core is not observable through
//! the [`RegistryQuery`](crate::ports::RegistryQuery) port, so `verify` returns
//! [`VerifyOutcome::Unknown`] **explicitly** rather than being excused from the
//! contract (ADR-0002 §1) — an honest "cannot check", never a false `Missing`.

use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{
    BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt, VerifyOutcome,
};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The homebrew distribution adapter, operating as `homebrew-tap` or
/// `homebrew-core`.
pub struct HomebrewAdapter {
    adapter: Adapter,
}

impl HomebrewAdapter {
    /// Construct for a resolved homebrew adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::HomebrewTap | Adapter::HomebrewCore
        ));
        Self { adapter }
    }
}

impl ReleaseAdapter for HomebrewAdapter {
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
                "brew",
                &["audit", "--strict", &t.package],
            )],
            notes: vec![
                "a formula is a downstream bump of an already-published artifact; \
                 there is no separate build step"
                    .to_string(),
            ],
        })
    }

    fn build(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        // Homebrew has no build phase of its own — it repackages an existing
        // release artifact. Return an empty manifest rather than shelling out.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![],
            notes: vec!["homebrew has no build phase (formula update only)".to_string()],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE (opens/merges a formula bump).
        // SKELETON: `bump-formula-pr` needs the release tarball URL + sha256 the
        // coordinator will thread in; the representative command is shown here.
        let cmds = match self.adapter {
            Adapter::HomebrewCore => vec![PlannedCommand::new(
                "brew",
                &["bump-formula-pr", "--no-fork", &t.package],
            )],
            _ => vec![PlannedCommand::new(
                "brew",
                &["bump-formula-pr", &t.package],
            )],
        };
        run_all(ctx, &cmds)?;
        Ok(make_receipt(ctx, t, None, None))
    }

    fn verify(
        &self,
        _ctx: &EffectCtx<'_>,
        _receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        // A tap/core formula is not observable through RegistryQuery; report the
        // honest "cannot check" rather than a false Missing (ADR-0002 §1).
        Ok(VerifyOutcome::Unknown)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

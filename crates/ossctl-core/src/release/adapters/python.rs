//! Python ecosystem adapter: `gh-action-pypi-publish` and `twine`.
//!
//! Publishes wheels/sdists to `PyPI`. `twine` uploads directly from this host;
//! `gh-action-pypi-publish` is the CI trusted-publisher flow, so its publish
//! body is a clearly-marked skeleton (the real upload happens in the workflow).
//! `verify` reconciles against `PyPI` via the default [`RegistryQuery`](crate::ports::RegistryQuery) path.

use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The python release adapter, operating as `gh-action-pypi-publish` or `twine`.
pub struct PythonAdapter {
    adapter: Adapter,
}

impl PythonAdapter {
    /// Construct for a resolved python adapter identity.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::GhActionPypiPublish | Adapter::Twine
        ));
        Self { adapter }
    }
}

impl ReleaseAdapter for PythonAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        _ctx: &EffectCtx<'_>,
        _t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        let notes = match self.adapter {
            Adapter::GhActionPypiPublish => {
                vec![
                    "upload runs in CI via the PyPI trusted publisher, not from this host"
                        .to_string(),
                ]
            }
            _ => vec![],
        };
        // `twine check` validates the built distributions; it is side-effect-free.
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands: vec![PlannedCommand::new("twine", &["check", "dist/*"])],
            notes,
        })
    }

    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        run_all(ctx, &[PlannedCommand::new("python", &["-m", "build"])])?;
        // SKELETON: a production build enumerates the wheels/sdists under dist/.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![
                format!("{}-{}-py3-none-any.whl", t.package, t.version),
                format!("{}-{}.tar.gz", t.package, t.version),
            ],
            notes: vec![],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE.
        if matches!(self.adapter, Adapter::GhActionPypiPublish) {
            // SKELETON: the real upload is the CI trusted-publisher job; there is
            // no honest host publish to perform, so we say so rather than
            // fabricate a receipt.
            return Err(AdapterError::Unsupported {
                adapter: self.adapter,
                operation: "publish",
            });
        }
        run_all(ctx, &[PlannedCommand::new("twine", &["upload", "dist/*"])])?;
        let remote_url = Some(format!(
            "https://pypi.org/project/{}/{}/",
            t.package, t.version
        ));
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

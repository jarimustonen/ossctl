//! Rust ecosystem adapter: `cargo-publish` (crates.io) and `cargo-dist`.
//!
//! Publishes crates to crates.io via `cargo publish`, or builds/uploads
//! distributable binaries via `cargo-dist` (`dist`). `verify` reconciles against
//! crates.io through [`RegistryQuery`](crate::ports::RegistryQuery) using the
//! adapter's default path.

use std::time::Duration;

use crate::contract::schema::Adapter;
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// The rust release adapter, operating as either `cargo-publish` or `cargo-dist`.
pub struct CargoAdapter {
    adapter: Adapter,
}

impl CargoAdapter {
    /// Construct for a resolved rust adapter identity (`cargo-publish` /
    /// `cargo-dist`).
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        debug_assert!(matches!(
            adapter,
            Adapter::CargoPublish | Adapter::CargoDist
        ));
        Self { adapter }
    }

    fn publish_commands(&self, t: &AdapterTarget) -> Vec<PlannedCommand> {
        match self.adapter {
            Adapter::CargoDist => vec![PlannedCommand::new("dist", &["build", "--artifacts=all"])],
            // `--no-verify`: the build phase already verified; re-verifying here
            // would double the work and is not what makes this irreversible.
            _ => vec![PlannedCommand::new(
                "cargo",
                &["publish", "-p", &t.package, "--no-verify"],
            )],
        }
    }
}

impl ReleaseAdapter for CargoAdapter {
    fn adapter(&self) -> Adapter {
        self.adapter
    }

    fn dry_run(
        &self,
        _ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        let planned_commands = match self.adapter {
            Adapter::CargoDist => vec![PlannedCommand::new(
                "dist",
                &["plan", "--output-format=json"],
            )],
            _ => vec![PlannedCommand::new(
                "cargo",
                &["publish", "-p", &t.package, "--dry-run"],
            )],
        };
        Ok(DryRunReport {
            adapter: self.adapter,
            planned_commands,
            notes: vec![],
        })
    }

    fn build(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<BuildArtifacts, AdapterError> {
        let cmds = match self.adapter {
            Adapter::CargoDist => vec![PlannedCommand::new("dist", &["build"])],
            _ => vec![PlannedCommand::new("cargo", &["package", "-p", &t.package])],
        };
        run_all(ctx, &cmds)?;
        // SKELETON: a production build parses the packaged `.crate` / dist
        // manifest paths out of the command output; here we name the expected
        // artifact deterministically.
        Ok(BuildArtifacts {
            adapter: self.adapter,
            artifacts: vec![format!("{}-{}.crate", t.package, t.version)],
            notes: vec![],
        })
    }

    fn publish(
        &self,
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<PublishReceipt, AdapterError> {
        // PER-TARGET IRREVERSIBLE — drives the real registry CLI through the
        // injected runner (the port is the safety seam under test).
        run_all(ctx, &self.publish_commands(t))?;
        // SKELETON: a production publish parses the crates.io checksum from the
        // `cargo publish` output for `digest`; the canonical URL is well-known.
        let remote_url = matches!(self.adapter, Adapter::CargoPublish)
            .then(|| format!("https://crates.io/crates/{}/{}", t.package, t.version));
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

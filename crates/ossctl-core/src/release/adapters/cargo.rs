//! Rust ecosystem adapter: `cargo-publish` (crates.io) and `cargo-dist`.
//!
//! `cargo-publish` publishes a crate to crates.io via `cargo publish`.
//! `cargo-dist` plans and builds distributable binaries locally (`dist`), but
//! its *upload* is the CI release workflow — so its publish body is
//! [`AdapterError::Unsupported`] from this host rather than a fabricated receipt
//! for a build-only command. `verify` (for `cargo-publish`) reconciles against
//! crates.io through [`RegistryQuery`](crate::ports::RegistryQuery) via the
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
        // `dist build` emits per-platform tarballs/installers, not a `.crate`;
        // name the artifact set to match what each identity actually produces.
        let (cmds, artifacts) = match self.adapter {
            Adapter::CargoDist => (
                vec![PlannedCommand::new("dist", &["build"])],
                vec!["dist/".to_string()],
            ),
            _ => (
                vec![PlannedCommand::new("cargo", &["package", "-p", &t.package])],
                vec![format!("{}-{}.crate", t.package, t.version)],
            ),
        };
        run_all(ctx, &cmds)?;
        // SKELETON: a production build parses the exact packaged `.crate` /
        // `dist-manifest.json` paths out of the command output; here we name the
        // expected artifact set deterministically.
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
        // cargo-dist uploads via the CI release workflow, not from this host —
        // `dist build` only builds. Report that honestly rather than returning a
        // receipt for a publish that did not happen.
        if matches!(self.adapter, Adapter::CargoDist) {
            return Err(AdapterError::Unsupported {
                adapter: self.adapter,
                operation: "publish",
            });
        }
        // PER-TARGET IRREVERSIBLE — drives the real `cargo publish` through the
        // injected runner (the port is the safety seam under test). No
        // `--no-verify`: a resume that enters publish without re-running build
        // must still let cargo verify the package before it lands.
        run_all(
            ctx,
            &[PlannedCommand::new("cargo", &["publish", "-p", &t.package])],
        )?;
        // SKELETON: a production publish parses the crates.io checksum from the
        // `cargo publish` output for `digest`; the canonical URL is well-known.
        let remote_url = Some(format!(
            "https://crates.io/crates/{}/{}",
            t.package, t.version
        ));
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

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
        // The concrete asset paths are threaded in via `ctx.artifacts.assets`
        // (gathered from every target's build). Flags precede the `--` option
        // terminator, and every asset path follows it, so a path that happens to
        // start with `-` is never mis-read as a flag.
        //
        // Pin the upload to the coordinator-resolved slug with `--repo` (when
        // known) rather than letting `gh` resolve the repository ambiently from the
        // cwd/remotes/`GH_REPO` — otherwise the upload target could differ from the
        // `remote_url` the receipt records below.
        let tag = Self::tag(t);
        let slug = ctx.artifacts.repo_slug.as_deref();
        let mut args = vec!["release", "upload", tag.as_str()];
        if let Some(slug) = slug {
            args.push("--repo");
            args.push(slug);
        }
        args.push("--clobber");
        args.push("--");
        args.extend(ctx.artifacts.assets.iter().map(String::as_str));
        run_all(ctx, &[PlannedCommand::new("gh", &args)])?;
        // Record where the assets landed: the GitHub-Release page for this tag,
        // built from the same slug the upload targeted. GitHub Releases expose no
        // single publish digest, so `digest` stays `None` (honest — the receipt
        // type documents `None` for ecosystems without one); `remote_url` is `None`
        // when the cut has no resolvable GitHub remote.
        let remote_url = slug.map(|slug| format!("https://github.com/{slug}/releases/tag/{tag}"));
        Ok(make_receipt(ctx, t, None, remote_url))
    }

    fn verify(
        &self,
        ctx: &EffectCtx<'_>,
        receipt: &PublishReceipt,
    ) -> Result<VerifyOutcome, AdapterError> {
        let tag = format!("v{}", receipt.version);
        let out = match ctx.runner.run(
            "gh",
            &["release", "view", &tag, "--json", "assets"],
            ctx.repo_root,
        ) {
            Ok(out) if out.status == Some(0) => out,
            Ok(_) => return Ok(VerifyOutcome::Missing),
            Err(_) => return Ok(VerifyOutcome::Unknown),
        };
        // A release object without its uploaded assets is not an observed binary
        // publish. The live coordinator additionally checks its known asset set.
        let has_assets = serde_json::from_str::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|v| {
                v.get("assets")
                    .and_then(|a| a.as_array())
                    .map(|a| !a.is_empty())
            })
            .unwrap_or(false);
        Ok(if has_assets {
            VerifyOutcome::Matches
        } else {
            VerifyOutcome::Missing
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

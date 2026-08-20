//! Binary distribution adapter: `manual` / GitHub Releases.
//!
//! Attaches prebuilt binaries to a GitHub Release (`gh release`). `verify` uses
//! a read-only `gh release view` observation and requires uploaded assets. A
//! command transport failure is `Unknown`; an absent Release or empty asset set is
//! `Missing`.

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubRelease {
    assets: Vec<GithubAsset>,
    is_draft: bool,
    tag_name: String,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
}

#[derive(Deserialize)]
struct DistManifest {
    announcement_tag: String,
    releases: Vec<DistRelease>,
}

#[derive(Deserialize)]
struct DistRelease {
    app_name: String,
    app_version: String,
    artifacts: Vec<String>,
}

/// Read the published GitHub Release at the stable `v<version>` tag. A malformed
/// response or command failure is an unobservable destination (`Unknown`), never
/// evidence that a Release is absent.
fn release_asset_names(
    ctx: &EffectCtx<'_>,
    version: &str,
) -> Result<BTreeSet<String>, VerifyOutcome> {
    let tag = format!("v{version}");
    let out = ctx
        .runner
        .run(
            "gh",
            &["release", "view", &tag, "--json", "assets,isDraft,tagName"],
            ctx.repo_root,
        )
        .map_err(|_| VerifyOutcome::Unknown)?;
    if out.status != Some(0) {
        // `gh` unfortunately uses exit 1 for both a definite absence and failures
        // to observe (auth, rate limits, transport). Its stable not-found diagnostic
        // is the only case that proves `Missing`; every other failure stays Unknown.
        return Err(if out.stderr.trim() == "release not found" {
            VerifyOutcome::Missing
        } else {
            VerifyOutcome::Unknown
        });
    }
    let release: GithubRelease =
        serde_json::from_str(&out.stdout).map_err(|_| VerifyOutcome::Unknown)?;
    if release.tag_name != tag || release.is_draft {
        return Err(VerifyOutcome::Missing);
    }
    Ok(release.assets.into_iter().map(|asset| asset.name).collect())
}

/// Observe a GitHub Release by tag and require its uploaded asset set. The
/// release title is deliberately irrelevant: cargo-dist commonly formats it as
/// `<version> - <date>`, while the stable lookup coordinate is the `v<version>` tag.
/// An empty `expected_assets` slice still requires at least one asset, which is
/// the strongest check available when reconciling a pre-plan-store journal.
pub(super) fn observe_release_assets(
    ctx: &EffectCtx<'_>,
    version: &str,
    expected_assets: &[String],
) -> VerifyOutcome {
    let observed = match release_asset_names(ctx, version) {
        Ok(observed) => observed,
        Err(outcome) => return outcome,
    };
    if if expected_assets.is_empty() {
        !observed.is_empty()
    } else {
        expected_assets
            .iter()
            .all(|wanted| observed.contains(wanted))
    } {
        VerifyOutcome::Matches
    } else {
        VerifyOutcome::Missing
    }
}

/// Observe cargo-dist's authoritative inventory on the published tagged Release.
/// The sealed contract's platform list is install policy and can deliberately be
/// broader than cargo-dist's configured targets, so archive names must come from
/// cargo-dist's manifest instead. Verification downloads that manifest, finds the
/// declared package and version, and requires every artifact cargo-dist records for
/// it to be present on the same Release. This avoids both the historical false red
/// and a false green while uploads are incomplete.
pub(super) fn observe_cargo_dist_release(
    ctx: &EffectCtx<'_>,
    version: &str,
    package: &str,
) -> VerifyOutcome {
    const MANIFEST: &str = "dist-manifest.json";
    let observed = match release_asset_names(ctx, version) {
        Ok(observed) => observed,
        Err(outcome) => return outcome,
    };
    if !observed.contains(MANIFEST) {
        return VerifyOutcome::Missing;
    }

    let tag = format!("v{version}");
    let out = match ctx.runner.run(
        "gh",
        &[
            "release",
            "download",
            &tag,
            "--pattern",
            MANIFEST,
            "--output",
            "-",
        ],
        ctx.repo_root,
    ) {
        Ok(out) if out.status == Some(0) => out,
        _ => return VerifyOutcome::Unknown,
    };
    let manifest: DistManifest = match serde_json::from_str(&out.stdout) {
        Ok(manifest) => manifest,
        Err(_) => return VerifyOutcome::Unknown,
    };
    if manifest.announcement_tag != tag {
        return VerifyOutcome::Conflicts;
    }
    let Some(release) = manifest
        .releases
        .iter()
        .find(|release| release.app_name == package)
    else {
        return VerifyOutcome::Unknown;
    };
    if release.app_version != version {
        return VerifyOutcome::Conflicts;
    }
    if !release.artifacts.is_empty()
        && release
            .artifacts
            .iter()
            .all(|artifact| observed.contains(artifact))
    {
        VerifyOutcome::Matches
    } else {
        VerifyOutcome::Missing
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
        Ok(observe_release_assets(ctx, &receipt.version, &[]))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }
}

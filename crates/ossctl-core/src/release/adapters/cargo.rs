//! Rust ecosystem adapter: `cargo-publish` (crates.io) and `cargo-dist`.
//!
//! `cargo-publish` publishes a crate to crates.io via `cargo publish`.
//! `cargo-dist` plans and builds distributable binaries locally (`dist`), but
//! its *upload* is the CI release workflow — so its publish body is
//! [`AdapterError::Unsupported`] from this host rather than a fabricated receipt
//! for a build-only command. `verify` (for `cargo-publish`) reconciles against
//! crates.io through [`RegistryQuery`](crate::ports::RegistryQuery) via the
//! adapter's default path.
//!
//! ## Multi-crate workspace publish (dep-order + index-wait)
//!
//! A single `cargo publish` cannot publish a workspace whose crates depend on
//! one another: crates.io rejects a crate whose sibling dependency is not yet
//! published (`no matching package named … found`). So the `cargo-publish` path
//! discovers the workspace's publishable members and their intra-workspace
//! dependency edges (read-only `cargo metadata`), publishes them in **topological
//! order** (a crate only after every workspace dependency it needs), and — after
//! each member that still has dependents to publish — **waits for crates.io to
//! index** the just-published version before publishing the next one (polling the
//! injected [`RegistryQuery`](crate::ports::RegistryQuery), bounded by a timeout).
//! A single-crate workspace degrades to exactly one `cargo publish` with no wait.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use serde::Deserialize;

use crate::contract::schema::{Adapter, Ecosystem};
use crate::protocol::release::{BuildArtifacts, DryRunReport, PlannedCommand, PublishReceipt};

use super::{make_receipt, run_all, AdapterError, AdapterTarget, EffectCtx, ReleaseAdapter};

/// Wall-clock ceiling for a single crate's crates.io index-wait, in seconds.
///
/// crates.io's sparse index is usually visible within seconds of a publish, but
/// the publish→index pipeline can lag under load; a generous per-crate ceiling
/// avoids a spurious failure while still bounding a hung wait so it can never
/// wedge a run.
const INDEX_WAIT_TIMEOUT_SECS: u64 = 300;

/// Interval between crates.io index polls while waiting for a just-published
/// version to appear.
const INDEX_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// One publishable workspace member the cut will `cargo publish`: its crate name
/// and the version its manifest declares (which may differ per member — a
/// workspace need not share one version, so the index-wait keys off each
/// member's own version, not the plan's).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishMember {
    /// The crate name (`cargo publish -p <name>`).
    name: String,
    /// The version this member publishes at — the value the index-wait polls for.
    version: String,
}

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
        ctx: &EffectCtx<'_>,
        t: &AdapterTarget,
    ) -> Result<DryRunReport, AdapterError> {
        if matches!(self.adapter, Adapter::CargoDist) {
            return Ok(DryRunReport {
                adapter: self.adapter,
                planned_commands: vec![PlannedCommand::new(
                    "dist",
                    &["plan", "--output-format=json"],
                )],
                notes: vec![],
            });
        }
        // Report the *whole* workspace publish plan: one `cargo publish … --dry-run`
        // per member in dependency order, with a note for each index-wait that a
        // real cut would perform between dependent publishes. `cargo metadata` is
        // read-only, so running it here keeps dry-run side-effect-free.
        let order = publish_order(ctx, t)?;
        let mut planned_commands = Vec::with_capacity(order.len());
        let mut notes = Vec::new();
        if order.len() > 1 {
            let chain = order
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            notes.push(format!("workspace publish order: {chain}"));
        }
        for (i, m) in order.iter().enumerate() {
            planned_commands.push(PlannedCommand::new(
                "cargo",
                &["publish", "-p", &m.name, "--dry-run"],
            ));
            if i + 1 < order.len() {
                notes.push(format!(
                    "then wait for crates.io to index `{}@{}` before publishing dependents",
                    m.name, m.version
                ));
            }
        }
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
        // injected runner (the port is the safety seam under test). Publish each
        // publishable member in dependency order, waiting for crates.io to index
        // each one before the next (dependent) member publishes. No `--no-verify`:
        // a resume that enters publish without re-running build must still let
        // cargo verify the package before it lands.
        let order = publish_order(ctx, t)?;
        let last = order.len().saturating_sub(1);
        for (i, m) in order.iter().enumerate() {
            run_all(
                ctx,
                &[PlannedCommand::new("cargo", &["publish", "-p", &m.name])],
            )?;
            // The final member has no dependents left in this cut, so its index
            // visibility is not a prerequisite for anything here — skip its wait.
            if i < last {
                wait_for_index(ctx, &m.name, &m.version)?;
            }
        }
        // SKELETON: a production publish parses the crates.io checksum from the
        // `cargo publish` output for `digest`; the canonical URL is well-known.
        // The receipt names the target's primary package (published last, so all
        // members have landed by the time it is stamped); the journal records one
        // receipt per ecosystem target.
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

/// Determine the publishable workspace members in topological publish order.
///
/// Runs read-only `cargo metadata` to discover the workspace graph, filters out
/// members marked `publish = false`, and orders them so a crate is published only
/// after every workspace-internal (non-dev) dependency it needs. Degrades to the
/// single named target package when no workspace graph is available (empty
/// `cargo metadata` output — a non-cargo host or a test double), so a plain
/// single-crate cut is unchanged.
fn publish_order(
    ctx: &EffectCtx<'_>,
    t: &AdapterTarget,
) -> Result<Vec<PublishMember>, AdapterError> {
    let single = || {
        vec![PublishMember {
            name: t.package.clone(),
            version: t.version.clone(),
        }]
    };
    let Some(meta) = load_metadata(ctx)? else {
        return Ok(single());
    };
    let members = publishable_members(&meta);
    if members.is_empty() {
        return Ok(single());
    }
    topo_sort(members)
}

/// Run `cargo metadata` and parse the workspace graph. `Ok(None)` when the
/// command succeeds but emits no output (a host without cargo, or a test double);
/// an `Err` only for a genuine command failure or unparseable non-empty output.
fn load_metadata(ctx: &EffectCtx<'_>) -> Result<Option<CargoMetadata>, AdapterError> {
    let cmd = PlannedCommand::new("cargo", &["metadata", "--no-deps", "--format-version", "1"]);
    let outputs = run_all(ctx, std::slice::from_ref(&cmd))?;
    let stdout = outputs[0].stdout.trim();
    if stdout.is_empty() {
        return Ok(None);
    }
    let meta = serde_json::from_str(stdout).map_err(|e| AdapterError::Command {
        command: cmd.rendered(),
        code: Some(0),
        stderr: format!("could not parse `cargo metadata` output: {e}"),
    })?;
    Ok(Some(meta))
}

/// Project the metadata onto the publishable members and their intra-workspace
/// (non-dev) dependency edges.
///
/// A member is publishable unless its manifest sets `publish = false` (which
/// `cargo metadata` reports as an empty `publish` array). Only edges to *other
/// publishable members* gate order; dev-dependencies are excluded (they never
/// gate publish order and can form legitimate cycles, e.g. a lib crate that
/// dev-depends on the CLI crate for integration tests).
fn publishable_members(meta: &CargoMetadata) -> Vec<Member> {
    let member_ids: HashSet<&str> = meta.workspace_members.iter().map(String::as_str).collect();
    let pkgs: Vec<&MetaPackage> = meta
        .packages
        .iter()
        .filter(|p| member_ids.contains(p.id.as_str()))
        // `publish = false` ⇒ `Some([])`; publishable ⇒ `None` or `Some([reg,…])`.
        .filter(|p| !matches!(&p.publish, Some(v) if v.is_empty()))
        .collect();
    let names: HashSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    pkgs.iter()
        .map(|p| {
            let mut deps: Vec<String> = p
                .dependencies
                .iter()
                .filter(|d| d.kind.as_deref() != Some("dev"))
                .filter(|d| d.name != p.name && names.contains(d.name.as_str()))
                .map(|d| d.name.clone())
                .collect();
            deps.sort();
            deps.dedup();
            Member {
                name: p.name.clone(),
                version: p.version.clone(),
                deps,
            }
        })
        .collect()
}

/// Topologically order members so each appears only after all its workspace
/// dependencies (Kahn's algorithm over a name-keyed [`BTreeMap`], so the order is
/// deterministic — ties broken alphabetically). Errors on a dependency cycle
/// among the publishable members (which would make a correct publish order
/// impossible).
fn topo_sort(members: Vec<Member>) -> Result<Vec<PublishMember>, AdapterError> {
    let mut graph: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for m in members {
        graph.insert(m.name.clone(), (m.version, m.deps));
    }
    let mut published: Vec<String> = Vec::with_capacity(graph.len());
    let mut remaining: Vec<String> = graph.keys().cloned().collect();
    while !remaining.is_empty() {
        // The first (alphabetically) member whose deps are all already published.
        let ready = remaining
            .iter()
            .find(|n| graph[*n].1.iter().all(|d| published.contains(d)))
            .cloned();
        match ready {
            Some(n) => {
                published.push(n.clone());
                remaining.retain(|x| x != &n);
            }
            None => {
                return Err(AdapterError::Command {
                    command: "cargo metadata".to_string(),
                    code: None,
                    stderr: format!(
                        "workspace publish order has a dependency cycle among: {remaining:?}"
                    ),
                });
            }
        }
    }
    Ok(published
        .into_iter()
        .map(|name| {
            let version = graph[&name].0.clone();
            PublishMember { name, version }
        })
        .collect())
}

/// Poll the crates.io index (through the injected [`RegistryQuery`]) until
/// `package@version` is visible, or the per-crate timeout elapses.
///
/// Between polls it waits [`INDEX_POLL_INTERVAL`] through the injected
/// [`Clock::sleep`](crate::ports::Clock::sleep) — real time in production, a
/// virtual advance under test — so the loop is bounded, never busy, and
/// deterministic in tests. A lookup error (a transient registry outage) is
/// treated as "not yet visible" and retried, not a hard failure; only exhausting
/// the timeout yields [`AdapterError::IndexTimeout`].
fn wait_for_index(ctx: &EffectCtx<'_>, package: &str, version: &str) -> Result<(), AdapterError> {
    let start = ctx.clock.now_unix();
    loop {
        if let Ok(versions) = ctx
            .registry
            .published_versions(Ecosystem::Rust.as_str(), package)
        {
            if versions.iter().any(|v| v == version) {
                return Ok(());
            }
        }
        if ctx.clock.now_unix().saturating_sub(start) >= INDEX_WAIT_TIMEOUT_SECS {
            return Err(AdapterError::IndexTimeout {
                package: package.to_string(),
                version: version.to_string(),
                waited_secs: INDEX_WAIT_TIMEOUT_SECS,
            });
        }
        ctx.clock.sleep(INDEX_POLL_INTERVAL);
    }
}

/// A publishable workspace member with its version and its intra-workspace
/// (non-dev) dependency names — the input to [`topo_sort`].
struct Member {
    name: String,
    version: String,
    deps: Vec<String>,
}

/// The subset of `cargo metadata --format-version 1 --no-deps` output the
/// publish-order discovery reads.
#[derive(Deserialize)]
struct CargoMetadata {
    /// Every package in the metadata; with `--no-deps` these are the workspace
    /// members only.
    packages: Vec<MetaPackage>,
    /// The package ids that are workspace members (matched against
    /// [`MetaPackage::id`] to be exact regardless of the id string format).
    workspace_members: Vec<String>,
}

/// One package entry from `cargo metadata`.
#[derive(Deserialize)]
struct MetaPackage {
    name: String,
    version: String,
    id: String,
    #[serde(default)]
    dependencies: Vec<MetaDep>,
    /// `null`/absent ⇒ publishable to any registry; `[]` ⇒ `publish = false`;
    /// `["<registry>",…]` ⇒ publishable to a restricted set (still publishable).
    #[serde(default)]
    publish: Option<Vec<String>>,
}

/// One dependency entry from `cargo metadata`.
#[derive(Deserialize)]
struct MetaDep {
    name: String,
    /// `null` (normal), `"dev"`, or `"build"`. Only normal/build deps gate
    /// publish order; dev-deps never do.
    #[serde(default)]
    kind: Option<String>,
}

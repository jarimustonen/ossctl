//! Unit tests for the sealed release plan: SHA-256 known-answer vectors,
//! `plan_id` stability (same inputs ⇒ same id), drift detection (any changed
//! input ⇒ a different id), the `verify` drift-reason surface, and the built
//! plan's shape (facts-resolved targets, invariant phase sequence).

use super::*;
use crate::contract::schema::{
    Adapter, Changelog, ChangelogMode, ChangelogSource, Contract, ContributionProvenance,
    DependencyBot, Distribution, DistributionAdapter, DocsSite, Ecosystem, Installer, Maturity,
    ProvenanceLevel, Registry, Release, ReleaseLayout, ReleaseModel, Status, Target,
    VersioningBase,
};
use crate::protocol::facts::{
    DistributionSurface, Facts, MaturitySignals, Package, RustWorkspace, WorkspaceMember,
    WorkspacePinOwner,
};
use crate::protocol::journal::Phase;
use crate::protocol::plan::PlanPhase;

// ── SHA-256 known-answer vectors (FIPS 180-4 / RFC 6234) ───────────────────

#[test]
fn sha256_empty_string() {
    assert_eq!(
        sha256::hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_abc() {
    assert_eq!(
        sha256::hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha256_multiblock() {
    // The 56-byte NIST vector: after the 0x80 + length pad it spans two 64-byte
    // blocks, exercising the multi-chunk path and the length pad.
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn sha256_one_million_a() {
    // The canonical NIST long-message vector: 1,000,000 'a' bytes (15,625
    // blocks) — the strongest correctness check on the round function, message
    // schedule, and length padding across many blocks.
    let data = vec![b'a'; 1_000_000];
    assert_eq!(
        sha256::hex(&data),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn sha256_pad_boundaries_are_deterministic_and_distinct() {
    // Lengths that bracket the 55/56 and 63/64/65 pad boundaries — where a
    // padding bug would cluster. We can't hardcode every NIST digest here, but
    // each length must hash deterministically and to a distinct value.
    let mut seen = std::collections::HashSet::new();
    for len in [54usize, 55, 56, 57, 63, 64, 65] {
        let data = vec![b'x'; len];
        let a = sha256::hex(&data);
        let b = sha256::hex(&data);
        assert_eq!(a, b, "len {len} must be deterministic");
        assert_eq!(a.len(), 64);
        assert!(seen.insert(a), "len {len} collided with another length");
    }
}

// ── Test fixtures ──────────────────────────────────────────────────────────

/// A representative single-ecosystem rust contract with a `null` package name
/// (the common case the plan resolves from facts).
fn rust_contract() -> Contract {
    Contract {
        schema_version: 1,
        status: Status::Approved,
        maturity: Maturity::Mvp,
        ecosystems: vec![Ecosystem::Rust],
        targets: vec![Target {
            ecosystem: Ecosystem::Rust,
            package: None,
            registry: Registry::CratesIo,
            adapter: Adapter::CargoPublish,
        }],
        distributions: vec![],
        versioning: VersioningBase::Semver,
        versioning_pattern: None,
        changelog: Changelog {
            mode: ChangelogMode::Curated,
            source: ChangelogSource::Manual,
            fragment_dir: "changelog/fragments".to_string(),
        },
        conventional_commits: false,
        release: Release {
            model: ReleaseModel::Gated,
            layout: ReleaseLayout::Single,
            bump_hook: None,
        },
        contribution_provenance: ContributionProvenance::None,
        provenance_level: ProvenanceLevel::None,
        dependency_bot: DependencyBot::None,
        health_badges: vec![],
        license: "MIT".to_string(),
        docs_site: DocsSite::None,
        extra_fields: serde_json::Map::new(),
        warnings: vec![],
    }
}

/// Facts naming a rust package `acme`, so the plan can resolve the contract's
/// `null` package.
fn rust_facts() -> Facts {
    Facts {
        repo_root: "/repo".to_string(),
        is_git: true,
        has_commits: true,
        ecosystems: vec![Ecosystem::Rust],
        packages: vec![Package {
            ecosystem: Ecosystem::Rust,
            manifest: "Cargo.toml".to_string(),
            package: Some("acme".to_string()),
            version: Some("0.1.0".to_string()),
        }],
        committers_total: 3,
        committers_recent_year: 2,
        tags: vec!["v0.1.0".to_string()],
        has_semver_tag: true,
        has_ge_1_0_release: false,
        has_ci: true,
        dependency_bot: None,
        has_issues_dir: false,
        readme_self_label: None,
        description: Some("An acme crate".to_string()),
        maturity_signals: MaturitySignals {
            production: false,
            spike: false,
        },
        inferred_maturity: Maturity::Mvp,
        distribution_surface: DistributionSurface {
            has_cargo_dist: false,
            cargo_dist_evidence: vec![],
            tag_triggered_workflows: vec![],
            tag_triggered_cargo_publish_workflows: vec![],
        },
        rust_workspace: None,
    }
}

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

// ── plan_id stability ──────────────────────────────────────────────────────

#[test]
fn plan_id_is_stable_for_identical_inputs() {
    let (c, f) = (rust_contract(), rust_facts());
    let a = build(&c, &f, HEAD, "1.2.0");
    let b = build(&c, &f, HEAD, "1.2.0");
    assert_eq!(
        a.plan_id, b.plan_id,
        "identical inputs must seal the same id"
    );
    // 64 lowercase hex chars.
    assert_eq!(a.plan_id.len(), 64);
    assert!(a
        .plan_id
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
}

#[test]
fn compute_plan_id_matches_build() {
    let (c, f) = (rust_contract(), rust_facts());
    let plan = build(&c, &f, HEAD, "1.2.0");
    assert_eq!(plan.plan_id, compute_plan_id(&c, &f, HEAD, "1.2.0"));
}

// ── drift: any changed input ⇒ a different id ──────────────────────────────

#[test]
fn changed_head_changes_the_id() {
    let (c, f) = (rust_contract(), rust_facts());
    let base = compute_plan_id(&c, &f, HEAD, "1.2.0");
    let other = compute_plan_id(&c, &f, "ffffffffffffffffffffffffffffffffffffffff", "1.2.0");
    assert_ne!(base, other);
}

#[test]
fn changed_version_changes_the_id() {
    let (c, f) = (rust_contract(), rust_facts());
    assert_ne!(
        compute_plan_id(&c, &f, HEAD, "1.2.0"),
        compute_plan_id(&c, &f, HEAD, "1.3.0")
    );
}

#[test]
fn changed_contract_field_changes_the_id() {
    let (c, f) = (rust_contract(), rust_facts());
    let base = compute_plan_id(&c, &f, HEAD, "1.2.0");
    let mut c2 = c.clone();
    c2.license = "Apache-2.0".to_string();
    assert_ne!(base, compute_plan_id(&c2, &f, HEAD, "1.2.0"));
}

#[test]
fn changed_resolved_package_changes_the_id() {
    // Same contract (package still null), but the repo manifest renamed the
    // crate — a change only visible through facts. Must still be drift.
    let c = rust_contract();
    let base = compute_plan_id(&c, &rust_facts(), HEAD, "1.2.0");
    let mut f2 = rust_facts();
    f2.packages[0].package = Some("renamed".to_string());
    assert_ne!(base, compute_plan_id(&c, &f2, HEAD, "1.2.0"));
}

#[test]
fn changed_adapter_changes_the_id() {
    let c = rust_contract();
    let base = compute_plan_id(&c, &rust_facts(), HEAD, "1.2.0");
    let mut c2 = c.clone();
    c2.targets[0].adapter = Adapter::CargoDist;
    assert_ne!(base, compute_plan_id(&c2, &rust_facts(), HEAD, "1.2.0"));
}

#[test]
fn changed_registry_changes_the_id() {
    let c = rust_contract();
    let base = compute_plan_id(&c, &rust_facts(), HEAD, "1.2.0");
    let mut c2 = c.clone();
    c2.targets[0].registry = Registry::TestPypi;
    assert_ne!(base, compute_plan_id(&c2, &rust_facts(), HEAD, "1.2.0"));
}

/// Golden vector: pins the exact `plan_id` bytes for a fixed input so an
/// accidental change to the pre-image (field reorder, a serde/`serde_json`
/// serialization change, an unbumped `SEAL_VERSION`) is caught — the
/// self-consistency tests above cannot detect that, since they call the same
/// implementation twice. If this fails after a *deliberate* pre-image change,
/// bump `SEAL_VERSION` and update the expected digest in the same commit.
#[test]
fn plan_id_golden_vector() {
    let plan = build(&rust_contract(), &rust_facts(), HEAD, "1.2.0");
    assert_eq!(
        plan.plan_id,
        "5d8c62995d97463cc72d81373e08d8201e4bfa6c5ca1f9673aeaf00ad6ca60ba"
    );
}

/// Golden vector for a contract carrying a POPULATED distribution — locks the
/// `Distribution` fields (incl. the new `package` association key) into the sealed
/// pre-image, so a serialization change to that struct is caught, not just the
/// top-level `distributions` array rename. Same discipline as
/// [`plan_id_golden_vector`]: a deliberate pre-image change bumps `SEAL_VERSION`
/// and updates this digest in one commit.
#[test]
fn plan_id_golden_vector_with_distribution() {
    let mut contract = rust_contract();
    contract.distributions = vec![Distribution {
        package: Some("acme".to_string()),
        adapter: DistributionAdapter::CargoDist,
        gh_releases: true,
        installers: vec![Installer::Shell, Installer::Homebrew],
        homebrew_tap: Some("acme/homebrew-acme".to_string()),
        platforms: vec![
            "aarch64-apple-darwin".to_string(),
            "x86_64-unknown-linux-musl".to_string(),
        ],
        extra_fields: serde_json::Map::new(),
    }];
    let plan = build(&contract, &rust_facts(), HEAD, "1.2.0");
    assert_eq!(
        plan.plan_id,
        "a845e00411f4f63126749dcc283f57b6564d1a26374cf24de8581c0008db6e24"
    );
    // The tap threads into the plan from the sole distribution.
    assert_eq!(plan.homebrew_tap.as_deref(), Some("acme/homebrew-acme"));
}

// ── verify: Ok when unchanged, PlanDrift (with reasons) when moved ─────────

#[test]
fn verify_ok_when_repo_unchanged() {
    let (c, f) = (rust_contract(), rust_facts());
    let approved = build(&c, &f, HEAD, "1.2.0");
    assert!(verify(&approved, &c, &f, HEAD).is_ok());
}

#[test]
fn verify_reports_head_drift() {
    let (c, f) = (rust_contract(), rust_facts());
    let approved = build(&c, &f, HEAD, "1.2.0");
    let moved = "ffffffffffffffffffffffffffffffffffffffff";
    let drift = verify(&approved, &c, &f, moved).unwrap_err();
    assert_eq!(drift.approved_plan_id, approved.plan_id);
    assert_ne!(drift.current_plan_id, approved.plan_id);
    assert!(
        drift.reasons.iter().any(|r| r.contains("HEAD moved")),
        "reasons: {:?}",
        drift.reasons
    );
}

#[test]
fn verify_reports_target_drift() {
    let (c, f) = (rust_contract(), rust_facts());
    let approved = build(&c, &f, HEAD, "1.2.0");
    let mut f2 = f.clone();
    f2.packages[0].package = Some("renamed".to_string());
    let drift = verify(&approved, &c, &f2, HEAD).unwrap_err();
    assert!(
        drift.reasons.iter().any(|r| r.contains("target set")),
        "reasons: {:?}",
        drift.reasons
    );
}

#[test]
fn verify_reports_contract_change_when_no_specific_probe_matches() {
    let (c, f) = (rust_contract(), rust_facts());
    let approved = build(&c, &f, HEAD, "1.2.0");
    let mut c2 = c.clone();
    c2.dependency_bot = DependencyBot::Dependabot; // not head/schema/target
    let drift = verify(&approved, &c2, &f, HEAD).unwrap_err();
    assert!(
        drift
            .reasons
            .iter()
            .any(|r| r.contains("normalized contract changed")),
        "reasons: {:?}",
        drift.reasons
    );
}

// ── built plan shape ───────────────────────────────────────────────────────

#[test]
fn build_resolves_null_package_from_facts() {
    let plan = build(&rust_contract(), &rust_facts(), HEAD, "1.2.0");
    assert_eq!(plan.targets.len(), 1);
    let t = &plan.targets[0];
    assert_eq!(t.ecosystem, Ecosystem::Rust);
    assert_eq!(t.package.as_deref(), Some("acme"));
    assert_eq!(t.registry, Registry::CratesIo);
    assert_eq!(t.adapter, Adapter::CargoPublish);
}

#[test]
fn build_leaves_package_null_when_facts_have_none() {
    let c = rust_contract();
    let mut f = rust_facts();
    f.packages.clear();
    let plan = build(&c, &f, HEAD, "1.2.0");
    assert_eq!(plan.targets[0].package, None);
}

#[test]
fn build_leaves_package_null_when_ecosystem_is_ambiguous() {
    // A monorepo: two named rust crates but a single null target. Resolution
    // must NOT silently pick the first — it leaves the package null (cut-time
    // inference) rather than mis-assign.
    let c = rust_contract();
    let mut f = rust_facts();
    f.packages = vec![
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/a/Cargo.toml".to_string(),
            package: Some("a".to_string()),
            version: Some("0.1.0".to_string()),
        },
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/b/Cargo.toml".to_string(),
            package: Some("b".to_string()),
            version: Some("0.1.0".to_string()),
        },
    ];
    let plan = build(&c, &f, HEAD, "1.2.0");
    assert_eq!(plan.targets[0].package, None);
}

#[test]
fn ambiguous_resolution_does_not_depend_on_facts_order() {
    // The ambiguous case must be order-independent: swapping the two candidate
    // packages must not change the (null) resolution or the plan_id.
    let c = rust_contract();
    let mut f1 = rust_facts();
    f1.packages = vec![
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/a/Cargo.toml".to_string(),
            package: Some("a".to_string()),
            version: None,
        },
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/b/Cargo.toml".to_string(),
            package: Some("b".to_string()),
            version: None,
        },
    ];
    let mut f2 = f1.clone();
    f2.packages.reverse();
    assert_eq!(
        compute_plan_id(&c, &f1, HEAD, "1.2.0"),
        compute_plan_id(&c, &f2, HEAD, "1.2.0")
    );
}

#[test]
fn build_with_no_targets_yields_empty_targets_and_stable_id() {
    let mut c = rust_contract();
    c.ecosystems.clear();
    c.targets.clear();
    let plan = build(&c, &rust_facts(), HEAD, "1.2.0");
    assert!(plan.targets.is_empty());
    // Still a well-formed, stable content address (a tag-only plan).
    assert_eq!(plan.plan_id.len(), 64);
    assert_eq!(
        plan.plan_id,
        compute_plan_id(&c, &rust_facts(), HEAD, "1.2.0")
    );
}

#[test]
fn build_keeps_explicit_contract_package_over_facts() {
    let mut c = rust_contract();
    c.targets[0].package = Some("explicit".to_string());
    let plan = build(&c, &rust_facts(), HEAD, "1.2.0");
    assert_eq!(plan.targets[0].package.as_deref(), Some("explicit"));
}

#[test]
fn build_emits_the_coordinator_phase_sequence() {
    let plan = build(&rust_contract(), &rust_facts(), HEAD, "1.2.0");
    let coordinator_phases: Vec<PlanPhase> = Phase::CUT_SEQUENCE
        .into_iter()
        .map(PlanPhase::from_coordinator)
        .collect();
    assert_eq!(plan.phases, coordinator_phases);
    assert_eq!(plan.phases.last(), Some(&PlanPhase::AdvanceBranch));
    assert_eq!(plan.contract_schema_version, 1);
    assert_eq!(plan.head_sha, HEAD);
    assert_eq!(plan.version, "1.2.0");
}

#[test]
fn build_preserves_multi_target_order() {
    // A uv-style [rust, python] repo: order follows the contract's targets.
    let mut c = rust_contract();
    c.ecosystems = vec![Ecosystem::Rust, Ecosystem::Python];
    c.targets = vec![
        Target {
            ecosystem: Ecosystem::Rust,
            package: None,
            registry: Registry::CratesIo,
            adapter: Adapter::CargoPublish,
        },
        Target {
            ecosystem: Ecosystem::Python,
            package: None,
            registry: Registry::Pypi,
            adapter: Adapter::GhActionPypiPublish,
        },
    ];
    let mut f = rust_facts();
    f.packages.push(Package {
        ecosystem: Ecosystem::Python,
        manifest: "pyproject.toml".to_string(),
        package: Some("acme-py".to_string()),
        version: Some("0.1.0".to_string()),
    });
    let plan = build(&c, &f, HEAD, "1.2.0");
    let seq: Vec<_> = plan.targets.iter().map(|t| t.ecosystem).collect();
    assert_eq!(seq, vec![Ecosystem::Rust, Ecosystem::Python]);
    assert_eq!(plan.targets[1].package.as_deref(), Some("acme-py"));
}

// ── version = projection of the manifest (single source of truth) ──────────
// The release version is derived SOLELY from the workspace manifest — there is no
// `--version` input (`release-drop-version-flag`). The version-source capability
// model (`version-source-fail-closed-nonrust`) decides skip-vs-fail-closed per
// target.

/// Build a `Target` in one line — the tests below assemble a variety of target sets.
fn target(ecosystem: Ecosystem, package: &str, registry: Registry, adapter: Adapter) -> Target {
    Target {
        ecosystem,
        package: Some(package.to_string()),
        registry,
        adapter,
    }
}

/// A detected `Package` fact in one line.
fn package(ecosystem: Ecosystem, manifest: &str, name: &str, version: Option<&str>) -> Package {
    Package {
        ecosystem,
        manifest: manifest.to_string(),
        package: Some(name.to_string()),
        version: version.map(str::to_string),
    }
}

#[test]
fn version_derived_from_the_manifest() {
    // Single source of truth: the release version IS the manifest version.
    // `rust_facts` declares `acme` at 0.1.0.
    let v = resolve_release_version(&rust_contract(), &rust_facts())
        .expect("the manifest version must resolve");
    assert_eq!(v, "0.1.0");
}

#[test]
fn a_lockstep_two_crate_workspace_derives_the_shared_version() {
    // Both crates at 0.1.0 (lockstep) — the shared version is the single source of
    // truth.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "acme",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "acme-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let mut f = rust_facts();
    f.packages.push(package(
        Ecosystem::Rust,
        "core/Cargo.toml",
        "acme-core",
        Some("0.1.0"),
    ));
    let v = resolve_release_version(&c, &f).expect("a lockstep workspace resolves");
    assert_eq!(v, "0.1.0");
}

#[test]
fn an_inconsistent_tree_has_no_single_source_of_truth() {
    // Two crates at DIFFERENT versions (0.1.0 and 0.2.0) — there is no single version
    // to project, so the tree is rejected as inconsistent.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "acme",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "acme-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let mut f = rust_facts();
    f.packages.push(package(
        Ecosystem::Rust,
        "core/Cargo.toml",
        "acme-core",
        Some("0.2.0"),
    ));
    let err =
        resolve_release_version(&c, &f).expect_err("a self-inconsistent tree must be rejected");
    match err {
        VersionResolveError::InconsistentTree { versions } => {
            let pairs: Vec<(&str, &str)> = versions
                .iter()
                .map(|m| (m.package.as_str(), m.manifest_version.as_str()))
                .collect();
            assert_eq!(pairs, vec![("acme", "0.1.0"), ("acme-core", "0.2.0")]);
        }
        other => panic!("expected an InconsistentTree, got {other:?}"),
    }
}

#[test]
fn no_manifest_version_anywhere_is_undeterminable() {
    // A single crates.io target whose package is absent from facts (no resolvable
    // manifest version) is not checkable; with `--version` removed there is no
    // fallback, so the version is undeterminable.
    let mut c = rust_contract();
    // The package IS in facts but with NO version — that is a fail-closed case, so use
    // a package that is absent from facts entirely (unresolved → not counted here).
    c.targets = vec![Target {
        ecosystem: Ecosystem::Rust,
        package: None,
        registry: Registry::CratesIo,
        adapter: Adapter::CargoPublish,
    }];
    let mut f = rust_facts();
    // Drop the named rust package so the `None`-package target resolves to nothing.
    f.packages.clear();
    assert!(matches!(
        resolve_release_version(&c, &f),
        Err(VersionResolveError::Undeterminable)
    ));
}

// ── version-source capability model (version-source-fail-closed-nonrust) ────

#[test]
fn version_source_classifies_every_ecosystem() {
    // Ecosystems that carry the package version in a tree manifest.
    for e in [Ecosystem::Rust, Ecosystem::Node, Ecosystem::Python] {
        assert_eq!(VersionSource::of(e), VersionSource::Manifest, "{e:?}");
    }
    // Ecosystems with no tree-manifest version: a raw binary (artifact-versioned) and
    // a Go module (VCS-tag-versioned).
    for e in [Ecosystem::Binary, Ecosystem::Go] {
        assert_eq!(VersionSource::of(e), VersionSource::Distribution, "{e:?}");
    }
}

#[test]
fn a_manifest_versioned_npm_target_without_a_detected_version_fails_closed() {
    // THE non-Rust fail-OPEN gap this closes: an npm target (manifest-versioned) whose
    // package.json version the detector did NOT read. The old guard silently skipped
    // it (fail open); now it fails closed rather than publish an unchecked version.
    let mut c = rust_contract();
    c.ecosystems = vec![Ecosystem::Node];
    c.targets = vec![target(
        Ecosystem::Node,
        "acme-js",
        Registry::Npm,
        Adapter::NpmPublish,
    )];
    let mut f = rust_facts();
    f.packages = vec![package(Ecosystem::Node, "package.json", "acme-js", None)];
    let err =
        resolve_release_version(&c, &f).expect_err("a versionless npm target must fail closed");
    match err {
        VersionResolveError::MissingManifestVersion { targets } => {
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].package, "acme-js");
            assert_eq!(targets[0].ecosystem, Ecosystem::Node);
            assert_eq!(targets[0].registry, Registry::Npm);
        }
        other => panic!("expected MissingManifestVersion, got {other:?}"),
    }
}

#[test]
fn a_versionless_pypi_target_fails_closed_even_beside_a_versioned_rust_target() {
    // A mixed [rust, python] repo where the python package.json/pyproject version is
    // missing: the readable rust version must NOT paper over the unreadable python one
    // — the guard fails closed on the python target rather than derive from rust alone.
    let mut c = rust_contract();
    c.ecosystems = vec![Ecosystem::Rust, Ecosystem::Python];
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "acme",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Python,
            "acme-py",
            Registry::Pypi,
            Adapter::GhActionPypiPublish,
        ),
    ];
    let mut f = rust_facts(); // rust `acme` @ 0.1.0
    f.packages.push(package(
        Ecosystem::Python,
        "pyproject.toml",
        "acme-py",
        None,
    ));
    let err = resolve_release_version(&c, &f)
        .expect_err("the versionless python target must fail closed");
    match err {
        VersionResolveError::MissingManifestVersion { targets } => {
            let pkgs: Vec<&str> = targets.iter().map(|t| t.package.as_str()).collect();
            assert_eq!(
                pkgs,
                vec!["acme-py"],
                "only the unreadable target is reported"
            );
        }
        other => panic!("expected MissingManifestVersion, got {other:?}"),
    }
}

#[test]
fn a_binary_distribution_target_is_skipped_not_failed() {
    // A `binary` gh-releases (cargo-dist) target carries NO tree-manifest version by
    // design — legitimately skipped. Beside a versioned rust crate the version still
    // resolves from the crate; the binary target never demands a version.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "acme",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        // A binary distribution to gh-releases — ecosystem `binary` ⇒ Distribution.
        target(
            Ecosystem::Binary,
            "acme",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
    ];
    let v = resolve_release_version(&c, &rust_facts())
        .expect("the binary target is skipped; the crate version resolves");
    assert_eq!(v, "0.1.0");
}

#[test]
fn a_distribution_only_rust_repo_resolves_from_the_crate_manifest() {
    // REGRESSION GUARD: a Rust crate published ONLY via distribution destinations (a
    // Homebrew tap, gh-releases) still reads its version from `Cargo.toml`. Keying the
    // capability on the ECOSYSTEM (not the registry) means a rust crate repackaged for
    // homebrew is manifest-versioned — so the version resolves even with no crates.io
    // target, rather than the (registry-keyed) misfire that returned Undeterminable
    // for a version plainly in the tree.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "acme",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
        target(
            Ecosystem::Rust,
            "acme",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
    ];
    let v = resolve_release_version(&c, &rust_facts())
        .expect("a distribution-only rust repo resolves its version from Cargo.toml");
    assert_eq!(v, "0.1.0");
}

#[test]
fn an_all_distribution_ecosystem_repo_has_no_derivable_version() {
    // Only distribution ECOSYSTEMS (binary/go) — no tree manifest carries a version, so
    // there is nothing to derive from and (with `--version` removed) no fallback.
    let mut c = rust_contract();
    c.ecosystems = vec![Ecosystem::Binary, Ecosystem::Go];
    c.targets = vec![
        target(
            Ecosystem::Binary,
            "acme",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
        target(
            Ecosystem::Go,
            "acme",
            Registry::ProxyGolangOrg,
            Adapter::Goreleaser,
        ),
    ];
    assert!(matches!(
        resolve_release_version(&c, &rust_facts()),
        Err(VersionResolveError::Undeterminable)
    ));
}

// ── multi-crate workspace derivation (release-rust-workspace-multicrate) ─────

/// A `WorkspaceMember` in one line (deps are intra-workspace crate names). Each dep is
/// given a lockstep `=<version>` requirement, the convention the pin-rewrite derivation
/// keys on — matching what `detect_rust_workspace` records for the `octl-core = { path,
/// version = "=X" }` shape this feature targets.
fn member(name: &str, version: &str, deps: &[&str]) -> WorkspaceMember {
    WorkspaceMember {
        package: name.to_string(),
        version: Some(version.to_string()),
        workspace_deps: deps.iter().map(|d| (*d).to_string()).collect(),
        dep_reqs: deps
            .iter()
            .map(|d| ((*d).to_string(), format!("={version}")))
            .collect(),
        pin_reqs: deps
            .iter()
            .map(|d| ((*d).to_string(), vec![Some(format!("={version}"))]))
            .collect(),
    }
}

fn pin_owner(name: &str, version: &str, deps: &[&str]) -> WorkspacePinOwner {
    WorkspacePinOwner {
        package: name.to_string(),
        pin_reqs: deps
            .iter()
            .map(|dep| ((*dep).to_string(), vec![Some(format!("={version}"))]))
            .collect(),
    }
}

/// Facts carrying a lib+bin Rust workspace graph (`lib` ← `bin` depends on it),
/// both crates.io-publishable — the orchestratectl shape.
fn lib_bin_workspace_facts(lib: &str, bin: &str) -> Facts {
    let mut f = rust_facts();
    f.rust_workspace = Some(RustWorkspace {
        members: vec![member(lib, "0.1.6", &[]), member(bin, "0.1.6", &[lib])],
        pin_owners: vec![
            pin_owner(lib, "0.1.6", &[]),
            pin_owner(bin, "0.1.6", &[lib]),
        ],
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });
    f
}

/// The resolved package names of a plan's targets, in plan order.
fn target_packages(plan: &ReleasePlan) -> Vec<Option<&str>> {
    plan.targets.iter().map(|t| t.package.as_deref()).collect()
}

#[test]
fn two_crate_workspace_declaring_only_the_bin_derives_both_members_lib_first() {
    // THE headline gap: a contract that declares ONLY the bin crate as a target must
    // now plan BOTH crates as ordered publish units (lib before bin), where before it
    // planned only the bin — which would `cargo publish <bin>` while the `=`-pinned
    // lib is not yet on crates.io (the orchestratectl failure).
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        target_packages(&plan),
        vec![Some("octl-core"), Some("orchestratectl")],
        "the lib is derived as its own target and ordered before the bin"
    );
    // Both are crates.io / cargo-publish Rust targets.
    for t in &plan.targets {
        assert_eq!(t.ecosystem, Ecosystem::Rust);
        assert_eq!(t.registry, Registry::CratesIo);
        assert_eq!(t.adapter, Adapter::CargoPublish);
    }
}

#[test]
fn a_publish_none_contract_derives_its_version_from_the_tree_manifest() {
    // A publish-none repo is still version-tracked and tagged, but it has no target to
    // project the version through. The version comes from the tree's own manifests for
    // the declared ecosystems, so `release plan` can derive a tag version instead of
    // refusing with `version_undeterminable`.
    let mut c = rust_contract();
    c.targets = vec![];
    let f = rust_facts(); // Cargo.toml: acme 0.1.0
    assert_eq!(resolve_release_version(&c, &f).unwrap(), "0.1.0");

    // A workspace MEMBER at another version does not veto the root: a private
    // workspace whose support crate is versioned independently is ordinary, and the
    // root manifest is the repo's own package — so it is the version authority.
    let mut with_member = f.clone();
    with_member.packages.push(Package {
        ecosystem: Ecosystem::Rust,
        manifest: "crates/other/Cargo.toml".to_string(),
        package: Some("other".to_string()),
        version: Some("0.2.0".to_string()),
    });
    assert_eq!(resolve_release_version(&c, &with_member).unwrap(), "0.1.0");

    // With no root package (a virtual workspace), the members ARE the authority and
    // must agree — two versions leave no single version to tag.
    let mut virtual_ws = f.clone();
    virtual_ws.packages = vec![
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/a/Cargo.toml".to_string(),
            package: Some("a".to_string()),
            version: Some("0.1.0".to_string()),
        },
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/b/Cargo.toml".to_string(),
            package: Some("b".to_string()),
            version: Some("0.2.0".to_string()),
        },
    ];
    assert!(matches!(
        resolve_release_version(&c, &virtual_ws),
        Err(VersionResolveError::InconsistentTree { .. })
    ));

    // And a tree with no version anywhere is undeterminable, as before.
    let mut versionless = f.clone();
    versionless.packages.clear();
    assert_eq!(
        resolve_release_version(&c, &versionless),
        Err(VersionResolveError::Undeterminable)
    );
}

#[test]
fn a_publish_none_contract_plans_no_targets_even_with_a_publishable_workspace() {
    // The workspace expansion derives a crate's dependencies only for a DECLARED
    // crates.io target; with `targets: []` there is no seed, so it must not resurrect
    // the workspace's crates as publish units. A publish-none contract plans a
    // tag-only release, and the plan is still sealable/executable (an empty target set
    // is a valid state, not an unplannable one).
    let mut c = rust_contract();
    c.targets = vec![];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let plan = build(&c, &f, HEAD, "0.1.6");
    assert!(
        plan.targets.is_empty(),
        "publish-none planned targets: {:?}",
        target_packages(&plan)
    );
    assert_eq!(plan.plan_id.len(), 64, "an empty target set still seals");
    crate::release::coordinator::validate_plan(&plan)
        .expect("a tag-only plan is executable, not a refusal");
}

#[test]
fn a_contract_declaring_only_the_bin_yields_the_same_target_set_as_declaring_both() {
    // The derivation is behavior-equivalent to a fully-declared contract: whether the
    // repo declares only the bin or both crates (shipshape's shape), the RESOLVED,
    // dependency-ordered publish set is identical. (The `plan_id` still differs — the
    // seal hashes the full contract text, which differs by one declared target — but
    // what a cut EXECUTES, the target set, is the same.)
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let mut only_bin = rust_contract();
    only_bin.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];

    let mut both = rust_contract();
    both.targets = vec![
        target(
            Ecosystem::Rust,
            "octl-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];

    let derived = build(&only_bin, &f, HEAD, "0.1.6");
    let declared = build(&both, &f, HEAD, "0.1.6");
    assert_eq!(
        target_packages(&derived),
        target_packages(&declared),
        "derived and fully-declared target sets match"
    );
    assert_eq!(
        derived.targets, declared.targets,
        "the derivation is a strict superset — the executed target set is identical"
    );
}

#[test]
fn shipshape_steady_state_plan_publishes_core_then_cli_and_keeps_product_names_distinct() {
    // After the one-time 0.11.0 recovery, both registry crates are normal publish
    // units again. The exact workspace edge forces core before CLI, while distribution
    // targets retain the product/binary name `shipshape`.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "shipshape-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "shipshape-cli",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "shipshape",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
        target(
            Ecosystem::Rust,
            "shipshape",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
    ];
    let mut f = rust_facts();
    f.packages = vec![
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/shipshape-core/Cargo.toml".into(),
            package: Some("shipshape-core".into()),
            version: Some("0.11.0".into()),
        },
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/shipshape-cli/Cargo.toml".into(),
            package: Some("shipshape-cli".into()),
            version: Some("0.11.0".into()),
        },
        Package {
            ecosystem: Ecosystem::Rust,
            manifest: "crates/shipshape-dist/Cargo.toml".into(),
            package: Some("shipshape".into()),
            version: Some("0.11.0".into()),
        },
    ];
    f.rust_workspace = Some(RustWorkspace {
        members: vec![
            member("shipshape-core", "0.11.0", &[]),
            member("shipshape-cli", "0.11.0", &["shipshape-core"]),
        ],
        pin_owners: Vec::new(),
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });
    assert_eq!(
        resolve_release_version(&c, &f).unwrap(),
        "0.11.0",
        "registry packages and product wrapper must project one version"
    );

    let plan = build(&c, &f, HEAD, "0.11.0");
    assert_eq!(
        target_packages(&plan),
        vec![
            Some("shipshape-core"),
            Some("shipshape-cli"),
            Some("shipshape"),
            Some("shipshape"),
        ]
    );
    assert_eq!(plan.targets.len(), 4);
}

#[test]
fn derivation_reorders_a_bin_first_declaration_into_dependency_order() {
    // Even if the contract lists the bin BEFORE the lib, the derived plan orders the
    // dependency first (the graph is the ordering authority, not the contract order).
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "octl-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        target_packages(&plan),
        vec![Some("octl-core"), Some("orchestratectl")]
    );
}

#[test]
fn derived_members_are_spliced_before_dist_and_homebrew_targets() {
    // A full cargo-dist contract: crates.io publish (bin only, in the contract) +
    // cargo-dist + homebrew. The derived lib+bin land where the crates.io target was;
    // the cargo-dist and homebrew targets keep their trailing position.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
    ];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build(&c, &f, HEAD, "0.1.6");
    let shape: Vec<(&str, Registry)> = plan
        .targets
        .iter()
        .map(|t| (t.package.as_deref().unwrap(), t.registry))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("octl-core", Registry::CratesIo),
            ("orchestratectl", Registry::CratesIo),
            ("orchestratectl", Registry::GhReleases),
            ("orchestratectl", Registry::Homebrew),
        ]
    );
}

#[test]
fn derivation_is_a_superset_keeping_a_declared_package_absent_from_the_graph() {
    // Robustness: a contract-declared crates.io crate that the parsed graph did not
    // capture (an odd manifest the detector missed) is still planned — never dropped.
    // It has no graph edges, so its closure is just itself: the UNRELATED workspace
    // members are NOT pulled in.
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "extra-crate",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        target_packages(&plan),
        vec![Some("extra-crate")],
        "a declared crate absent from the graph is planned alone — no unrelated members"
    );
}

#[test]
fn derivation_publishes_the_closure_not_every_member() {
    // SAFETY (the over-publish fix): a workspace with an UNRELATED publishable member
    // the contract deliberately omitted (e.g. a not-yet-release-ready crate) must NOT
    // be published. The plan is the declared bin + its dependency closure (the lib) —
    // never the unrelated `experimental` crate.
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = rust_facts();
    f.rust_workspace = Some(RustWorkspace {
        members: vec![
            member("octl-core", "0.1.6", &[]),
            member("orchestratectl", "0.1.6", &["octl-core"]),
            member("experimental", "0.1.6", &[]), // publishable but unrelated + undeclared
        ],
        pin_owners: Vec::new(),
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });
    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        target_packages(&plan),
        vec![Some("octl-core"), Some("orchestratectl")],
        "only the declared crate and its dependency closure are published"
    );
    assert!(
        !target_packages(&plan).contains(&Some("experimental")),
        "an unrelated undeclared member must never be pulled into an irreversible publish"
    );
}

#[test]
fn derivation_pulls_a_transitive_dependency_closure() {
    // A three-crate chain declared only by the top: app → mid → core. The closure adds
    // both mid and core, ordered core → mid → app.
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "app",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = rust_facts();
    f.rust_workspace = Some(RustWorkspace {
        members: vec![
            member("app", "1.0.0", &["mid"]),
            member("mid", "1.0.0", &["core"]),
            member("core", "1.0.0", &[]),
            member("unrelated", "1.0.0", &[]),
        ],
        pin_owners: Vec::new(),
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });
    let plan = build(&c, &f, HEAD, "1.0.0");
    assert_eq!(
        target_packages(&plan),
        vec![Some("core"), Some("mid"), Some("app")]
    );
}

#[test]
fn an_ambiguous_null_rust_target_is_never_expanded_to_publish_everything() {
    // A monorepo the facts could not disambiguate leaves the crates.io target
    // `package: None`. The derivation must NOT turn that into a workspace-wide publish
    // — it leaves the plan untouched so the downstream null-package guard refuses it.
    let mut c = rust_contract();
    c.targets = vec![Target {
        ecosystem: Ecosystem::Rust,
        package: None,
        registry: Registry::CratesIo,
        adapter: Adapter::CargoPublish,
    }];
    let mut f = rust_facts();
    // Facts cannot resolve a single package name (several rust packages), so the
    // target stays null; a workspace graph is nonetheless present.
    f.packages = vec![
        package(Ecosystem::Rust, "crates/a/Cargo.toml", "a", Some("1.0.0")),
        package(Ecosystem::Rust, "crates/b/Cargo.toml", "b", Some("1.0.0")),
    ];
    f.rust_workspace = Some(RustWorkspace {
        members: vec![member("a", "1.0.0", &[]), member("b", "1.0.0", &["a"])],
        pin_owners: Vec::new(),
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });
    let plan = build(&c, &f, HEAD, "1.0.0");
    assert_eq!(
        target_packages(&plan),
        vec![None],
        "an unresolved rust target is preserved, never expanded into publish-everything"
    );
}

#[test]
fn a_single_crate_repo_is_unaffected_by_the_derivation() {
    // No workspace graph (a single root crate) ⇒ the plan is exactly the 1:1
    // contract-resolved target set.
    let c = rust_contract();
    let plan = build(&c, &rust_facts(), HEAD, "0.1.0");
    assert_eq!(target_packages(&plan), vec![Some("acme")]);
}

#[test]
fn homebrew_tap_carries_into_a_multi_crate_plan() {
    // Facet 4: the per-tool tap from the distribution block threads onto the plan
    // (non-null) even for a derived multi-crate workspace.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
    ];
    c.distributions = vec![Distribution {
        package: None,
        adapter: DistributionAdapter::CargoDist,
        gh_releases: true,
        installers: vec![Installer::Homebrew],
        homebrew_tap: Some("jarimustonen/orchestratectl".to_string()),
        platforms: vec!["aarch64-apple-darwin".to_string()],
        extra_fields: serde_json::Map::new(),
    }];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        plan.homebrew_tap.as_deref(),
        Some("jarimustonen/orchestratectl")
    );
    // And the derivation still produced both crates.io members.
    assert_eq!(
        plan.targets
            .iter()
            .filter(|t| t.registry == Registry::CratesIo)
            .count(),
        2
    );
}

#[test]
fn orchestratectl_plan_id_differs_once_the_lib_target_is_derived() {
    // The resolved target set grew (bin-only → lib+bin), so the sealed plan_id
    // changes — a stale single-target plan re-derives to a different id and `verify`
    // reports drift, exactly as intended (the plan genuinely changed).
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f_no_graph = rust_facts();
    f_no_graph.rust_workspace = None;
    let f_graph = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let before = build(&c, &f_no_graph, HEAD, "0.1.6");
    let after = build(&c, &f_graph, HEAD, "0.1.6");
    assert_eq!(target_packages(&before), vec![Some("orchestratectl")]);
    assert_ne!(before.plan_id, after.plan_id);
}

// ── topological member ordering ──────────────────────────────────────────────

#[test]
fn topo_order_puts_dependencies_before_dependents() {
    // A three-crate chain: core ← mid ← app (app depends on mid, mid on core).
    let members = vec![
        member("app", "1.0.0", &["mid"]),
        member("mid", "1.0.0", &["core"]),
        member("core", "1.0.0", &[]),
    ];
    assert_eq!(topo_order_members(&members), vec!["core", "mid", "app"]);
}

#[test]
fn topo_order_is_deterministic_for_independent_members() {
    // Independent members keep declaration order (a stable, reproducible tie-break —
    // a requirement of the content-addressed plan).
    let members = vec![member("zeta", "1.0.0", &[]), member("alpha", "1.0.0", &[])];
    assert_eq!(topo_order_members(&members), vec!["zeta", "alpha"]);
}

#[test]
fn topo_order_appends_a_cycle_deterministically_without_looping() {
    // A pathological cycle (a ← b ← a) cannot be ordered; the members are still all
    // emitted (declaration order), never dropped or hung on.
    let members = vec![member("a", "1.0.0", &["b"]), member("b", "1.0.0", &["a"])];
    assert_eq!(topo_order_members(&members), vec!["a", "b"]);
}

// ── engine-owned version-bump plan (`release-rust-workspace-multicrate` f2/f3) ─

/// Build a `--bump` plan on the lib+bin workspace fixture (both members at `0.1.6`).
/// The engine computes `to_version` from `0.1.6` + `level` (no literal is passed).
fn bump_plan(level: BumpLevel) -> ReleasePlan {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    build_with_bump(&c, &f, HEAD, "0.1.6", level).expect("0.1.6 is strict semver")
}

#[test]
fn the_no_bump_path_is_unchanged_and_opt_in() {
    // `build` (no `--bump`) carries no bump plan, no `bump` phase, and the version is
    // the manifest version verbatim — a strict superset guarantee: opting out is the
    // old behavior byte-for-byte (its plan_id is asserted stable by the golden vector).
    let (c, f) = (rust_contract(), rust_facts());
    let plan = build(&c, &f, HEAD, "0.1.0");
    assert!(plan.bump.is_none(), "no --bump ⇒ no bump plan");
    assert_eq!(
        plan.phases,
        PlanPhase::SEQUENCE.to_vec(),
        "no leading bump phase"
    );
    assert!(!plan.phases.contains(&PlanPhase::Bump));
}

#[test]
fn a_bump_plan_prepends_the_bump_phase_and_carries_the_computed_version() {
    let plan = bump_plan(BumpLevel::Minor);
    assert_eq!(
        plan.phases[0],
        PlanPhase::Bump,
        "bump runs before every barrier"
    );
    assert_eq!(
        &plan.phases[1..],
        PlanPhase::SEQUENCE.as_slice(),
        "the rest of the pipeline is unchanged after the bump phase"
    );
    // The release version threaded into every publish/tag is the COMPUTED new version.
    assert_eq!(plan.version, "0.2.0");
    let bump = plan.bump.as_ref().expect("a --bump plan carries a bump");
    assert_eq!(bump.level, BumpLevel::Minor);
    assert_eq!(bump.from_version, "0.1.6");
    assert_eq!(bump.to_version, "0.2.0");
}

#[test]
fn the_bump_derives_the_intra_workspace_pin_rewrite() {
    // THE lockstep pin: the bin's `octl-core = "=0.1.6"` must become `= "=0.2.0"`.
    let plan = bump_plan(BumpLevel::Minor);
    let bump = plan.bump.unwrap();
    assert_eq!(
        bump.pin_rewrites.len(),
        1,
        "one lib←bin edge ⇒ one pin rewrite"
    );
    let r = &bump.pin_rewrites[0];
    assert_eq!(r.in_package, "orchestratectl");
    assert_eq!(r.dependency, "octl-core");
    assert_eq!(r.from, "=0.1.6");
    assert_eq!(r.to, "=0.2.0");
}

#[test]
fn non_published_wrapper_pin_is_owned_by_the_sealed_edit_set() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "shipshape-cli",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("shipshape-core", "shipshape-cli");
    f.rust_workspace
        .as_mut()
        .unwrap()
        .pin_owners
        .push(pin_owner("shipshape", "0.1.6", &["shipshape-cli"]));

    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap();
    let rewrites = &plan.bump.as_ref().unwrap().pin_rewrites;
    let wrapper = rewrites
        .iter()
        .find(|rewrite| rewrite.in_package == "shipshape")
        .expect("the non-published wrapper's exact pin is sealed");
    assert_eq!(wrapper.dependency, "shipshape-cli");
    assert_eq!(wrapper.from, "=0.1.6");
    assert_eq!(wrapper.to, "=0.2.0");
    assert!(plan
        .targets
        .iter()
        .all(|target| target.package.as_deref() != Some("shipshape")));
}

#[test]
fn inherited_workspace_exact_pin_is_owned_by_the_sealed_edit_set() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let workspace = f.rust_workspace.as_mut().unwrap();
    workspace.pin_owners[1]
        .pin_reqs
        .insert("octl-core".into(), vec![None]);
    workspace
        .workspace_pin_reqs
        .insert("octl-core".into(), vec![Some("=0.1.6".into())]);

    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap();
    let rewrites = &plan.bump.as_ref().unwrap().pin_rewrites;
    assert_eq!(
        rewrites.len(),
        1,
        "the inherited root pin is the owned literal"
    );
    let root = rewrites
        .iter()
        .find(|rewrite| rewrite.workspace_root)
        .unwrap();
    assert_eq!(root.in_package, "workspace");
    assert_eq!(root.dependency, "octl-core");
    assert_eq!(root.from, "=0.1.6");
    assert_eq!(root.to, "=0.2.0");
}

#[test]
fn stale_exact_workspace_pin_is_refused_at_plan_time() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let workspace = f.rust_workspace.as_mut().unwrap();
    workspace.pin_owners[1]
        .pin_reqs
        .insert("octl-core".into(), vec![None]);
    workspace
        .workspace_pin_reqs
        .insert("octl-core".into(), vec![Some("=0.1.5".into())]);

    let err = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap_err();
    assert!(
        err.reason.contains("exact internal pin `octl-core`"),
        "{}",
        err.reason
    );
}

#[test]
fn pin_parser_failure_refuses_plan_instead_of_meaning_no_pins() {
    let c = rust_contract();
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    f.rust_workspace.as_mut().unwrap().pin_parse_error =
        Some("crates/cli/Cargo.toml: invalid inline table".into());

    let err = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap_err();
    assert!(err.reason.contains("could not be parsed"), "{}", err.reason);
}

#[test]
fn equivalent_duplicate_pins_plan_once_and_rewrite_every_declaration() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let cli = &mut f.rust_workspace.as_mut().unwrap().pin_owners[1];
    cli.pin_reqs.insert(
        "octl-core".into(),
        vec![Some("=0.1.6".into()), Some("=0.1.6".into())],
    );

    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap();
    let rewrites = &plan.bump.as_ref().unwrap().pin_rewrites;
    assert_eq!(rewrites.len(), 1, "one sealed equivalent declaration set");

    let manifest = "[dependencies]\noctl-core = { path = \"../core\", version = \"=0.1.6\" }\n[dev-dependencies]\noctl-core = { path = \"../core\", version = \"=0.1.6\" }\n";
    let r = &rewrites[0];
    let bumped =
        crate::release::bump::rewrite_pin(manifest, &r.dependency, &r.from, &r.to).unwrap();
    assert_eq!(bumped.matches("version = \"=0.2.0\"").count(), 2);
}

#[test]
fn non_equivalent_duplicate_pins_are_refused_while_planning() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    f.rust_workspace.as_mut().unwrap().pin_owners[1]
        .pin_reqs
        .insert(
            "octl-core".into(),
            vec![Some("=0.1.6".into()), Some("^0.1".into())],
        );

    let err = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap_err();
    assert!(
        err.reason.contains("differ from `=0.1.6`"),
        "{}",
        err.reason
    );
    assert!(err
        .reason
        .contains("refusing to seal an ambiguous pin rewrite"));
}

#[test]
fn path_only_duplicate_is_neutral_during_planning() {
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let mut f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    f.rust_workspace.as_mut().unwrap().pin_owners[1]
        .pin_reqs
        .insert("octl-core".into(), vec![Some("=0.1.6".into()), None]);

    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap();
    assert_eq!(plan.bump.unwrap().pin_rewrites.len(), 1);
}

#[test]
fn build_with_bump_computes_the_version_from_the_level_not_a_literal() {
    // The core constructor OWNS the arithmetic — a caller supplies only the level, so a
    // plan can never seal a to_version that contradicts its declared level.
    let mut c = rust_contract();
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Major).unwrap();
    assert_eq!(plan.version, "1.0.0");
    assert_eq!(plan.bump.unwrap().to_version, "1.0.0");
}

#[test]
fn build_with_bump_fails_closed_on_a_non_semver_from_version() {
    let c = rust_contract();
    let f = rust_facts();
    let err = build_with_bump(&c, &f, HEAD, "not-semver", BumpLevel::Patch).unwrap_err();
    assert_eq!(err.version, "not-semver");
}

#[test]
fn a_single_crate_workspace_has_no_pin_rewrites() {
    // No intra-workspace edges ⇒ nothing to rewrite (shipshape's own two crates DO pin,
    // but a lone crate does not).
    let c = rust_contract();
    let f = rust_facts(); // rust_workspace: None
    let plan = build_with_bump(&c, &f, HEAD, "0.1.0", BumpLevel::Patch).unwrap();
    assert!(plan.bump.unwrap().pin_rewrites.is_empty());
}

#[test]
fn the_bump_finalizes_a_curated_changelog_but_not_an_automated_one() {
    // curated/fragment: the engine promotes `[Unreleased]`. automated: a release bot
    // owns the CHANGELOG, so the engine must not also rewrite it.
    let curated = bump_plan(BumpLevel::Patch);
    let curated_bump = curated.bump.unwrap();
    assert!(curated_bump.changelog_finalize);
    let changelog = curated_bump
        .changelog
        .expect("fresh plans seal changelog inputs");
    assert_eq!(changelog.mode, ChangelogMode::Curated);
    assert_eq!(changelog.source, ChangelogSource::Manual);
    assert_eq!(changelog.fragment_dir, "changelog/fragments");

    let mut c = rust_contract();
    c.changelog.mode = ChangelogMode::Automated;
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let auto = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Patch).unwrap();
    let auto_bump = auto.bump.unwrap();
    assert!(!auto_bump.changelog_finalize);
    assert!(auto_bump.changelog.is_none());
}

#[test]
fn trailer_range_uses_the_engine_previous_tag_and_sealed_head() {
    let mut contract = rust_contract();
    contract.changelog.source = ChangelogSource::IssuectlTrailers;
    let mut facts = rust_facts();
    facts.tags = vec![
        "nightly".into(),
        "v0.1.0".into(),
        "v0.2.0-rc.1".into(),
        "v0.1.5".into(),
    ];
    let plan = build_with_bump(&contract, &facts, HEAD, "0.1.0", BumpLevel::Patch).unwrap();
    assert_eq!(
        plan.bump
            .unwrap()
            .changelog
            .unwrap()
            .issuectl_range
            .as_deref(),
        Some("v0.1.0..0123456789abcdef0123456789abcdef01234567")
    );
}

#[test]
fn a_declared_bump_hook_rides_on_the_bump_plan() {
    // facet 3: the contract's `release.bump_hook` is carried into the bump plan so the
    // executor runs it (e.g. regenerating version-embedding snapshots).
    let mut c = rust_contract();
    c.release.bump_hook = Some("cargo insta test --accept".to_string());
    c.targets = vec![target(
        Ecosystem::Rust,
        "orchestratectl",
        Registry::CratesIo,
        Adapter::CargoPublish,
    )];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
    let plan = build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap();
    assert_eq!(
        plan.bump.unwrap().bump_hook.as_deref(),
        Some("cargo insta test --accept")
    );
}

#[test]
fn an_absent_bump_hook_is_none_on_the_bump_plan() {
    let plan = bump_plan(BumpLevel::Patch);
    assert!(plan.bump.unwrap().bump_hook.is_none());
}

// ── content addressing: the bump is sealed ─────────────────────────────────

#[test]
fn a_bump_plan_has_a_different_id_than_the_no_bump_plan() {
    // The bump phase + the computed version are part of the content address: a plan
    // that owns a bump can never collide with one that publishes the tree version.
    let no_bump = {
        let mut c = rust_contract();
        c.targets = vec![target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        )];
        let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
        build(&c, &f, HEAD, "0.1.6")
    };
    let bumped = bump_plan(BumpLevel::Patch);
    assert_ne!(no_bump.plan_id, bumped.plan_id);
}

#[test]
fn different_bump_levels_seal_different_ids() {
    // Sealing the level (not just the resulting version) means approving `--bump minor`
    // and cutting `--bump major` is drift even if, pathologically, the versions matched.
    let minor = bump_plan(BumpLevel::Minor);
    let patch = bump_plan(BumpLevel::Patch);
    assert_ne!(minor.plan_id, patch.plan_id);
}

#[test]
fn a_bump_plan_is_stable_for_identical_inputs() {
    let a = bump_plan(BumpLevel::Minor);
    let b = bump_plan(BumpLevel::Minor);
    assert_eq!(a.plan_id, b.plan_id, "determinism: same inputs ⇒ same id");
}

#[test]
fn a_declared_bump_hook_changes_the_bump_plan_id() {
    // The hook rides in the pre-image (via the hashed contract AND the bump plan), so
    // adding one is drift — a cut can't silently drop a declared regen step.
    let without = bump_plan(BumpLevel::Minor);
    let with = {
        let mut c = rust_contract();
        c.release.bump_hook = Some("cargo insta test --accept".to_string());
        c.targets = vec![target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        )];
        let f = lib_bin_workspace_facts("octl-core", "orchestratectl");
        build_with_bump(&c, &f, HEAD, "0.1.6", BumpLevel::Minor).unwrap()
    };
    assert_ne!(without.plan_id, with.plan_id);
}

// ── mixed engine/CI-delegated workspace ordering (release-ci-publish-mode) ────

#[test]
fn an_engine_target_depending_on_a_ci_delegated_crate_is_a_conflict() {
    // The unsatisfiable shape: `bin` (engine-published) depends on `lib`, whose
    // publish is CI-delegated. publish-all runs BEFORE the tag push that triggers CI,
    // so `lib` can never be on the index when `bin` needs it — the cut would burn the
    // index-wait and fail, and no retry or resume could ever fix it.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "octl-core",
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let plan = build(&c, &f, HEAD, "0.1.6");
    let conflicts = delegated_dependency_conflicts(&plan, &f);
    assert_eq!(
        conflicts,
        vec![DelegatedDependencyConflict {
            engine_package: "orchestratectl".to_string(),
            delegated_package: "octl-core".to_string(),
        }]
    );
    let message = &delegated_dependency_messages(&conflicts)[0];
    assert!(message.contains("orchestratectl") && message.contains("octl-core"));
}

#[test]
fn the_reverse_dependency_direction_is_not_a_conflict() {
    // Engine-published dependency, CI-published dependent: the engine publishes `lib`
    // in publish-all, then the tag triggers CI to publish `bin`. That ordering works,
    // and refusing it would block the mode's most natural migration path.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "octl-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        ),
    ];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let plan = build(&c, &f, HEAD, "0.1.6");
    assert!(delegated_dependency_conflicts(&plan, &f).is_empty());
}

#[test]
fn an_all_delegated_workspace_has_no_conflict() {
    // Nothing the engine publishes ⇒ nothing that can wait on a delegated crate. CI
    // owns the whole ordering, which is exactly what the mode delegates to it.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "octl-core",
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        ),
        target(
            Ecosystem::Rust,
            "orchestratectl",
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        ),
    ];
    let f = lib_bin_workspace_facts("octl-core", "orchestratectl");

    let plan = build(&c, &f, HEAD, "0.1.6");
    assert!(delegated_dependency_conflicts(&plan, &f).is_empty());
}

#[test]
fn a_transitive_delegated_dependency_is_still_a_conflict() {
    // The walk is a closure, not one hop: bin → mid → core, with only `core`
    // delegated, is the same unsatisfiable wait one level deeper. `mid` is reported
    // too — the engine's own workspace-closure expansion derives it as an
    // engine-published target, and it depends on the delegated crate directly.
    let mut c = rust_contract();
    c.targets = vec![
        target(
            Ecosystem::Rust,
            "core",
            Registry::CratesIo,
            Adapter::CargoPublishCi,
        ),
        target(
            Ecosystem::Rust,
            "bin",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let mut f = rust_facts();
    f.rust_workspace = Some(RustWorkspace {
        members: vec![
            member("core", "0.1.6", &[]),
            member("mid", "0.1.6", &["core"]),
            member("bin", "0.1.6", &["mid"]),
        ],
        pin_owners: Vec::new(),
        workspace_pin_reqs: std::collections::BTreeMap::new(),
        pin_parse_error: None,
    });

    let plan = build(&c, &f, HEAD, "0.1.6");
    assert_eq!(
        delegated_dependency_conflicts(&plan, &f),
        vec![
            DelegatedDependencyConflict {
                engine_package: "bin".to_string(),
                delegated_package: "core".to_string(),
            },
            DelegatedDependencyConflict {
                engine_package: "mid".to_string(),
                delegated_package: "core".to_string(),
            },
        ]
    );
}

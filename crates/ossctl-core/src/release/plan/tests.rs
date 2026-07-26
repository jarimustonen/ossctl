//! Unit tests for the sealed release plan: SHA-256 known-answer vectors,
//! `plan_id` stability (same inputs ⇒ same id), drift detection (any changed
//! input ⇒ a different id), the `verify` drift-reason surface, and the built
//! plan's shape (facts-resolved targets, invariant phase sequence).

use super::*;
use crate::contract::schema::{
    Adapter, Changelog, ChangelogMode, ChangelogSource, Contract, ContributionProvenance,
    DependencyBot, DocsSite, Ecosystem, Maturity, ProvenanceLevel, Registry, Release,
    ReleaseLayout, ReleaseModel, Status, Target, VersioningBase,
};
use crate::protocol::facts::{Facts, MaturitySignals, Package};
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
    // A > 64-byte input to exercise the multi-chunk path and the length pad.
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
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
fn build_keeps_explicit_contract_package_over_facts() {
    let mut c = rust_contract();
    c.targets[0].package = Some("explicit".to_string());
    let plan = build(&c, &rust_facts(), HEAD, "1.2.0");
    assert_eq!(plan.targets[0].package.as_deref(), Some("explicit"));
}

#[test]
fn build_emits_the_invariant_phase_sequence() {
    let plan = build(&rust_contract(), &rust_facts(), HEAD, "1.2.0");
    assert_eq!(
        plan.phases,
        vec![
            PlanPhase::DryRunAll,
            PlanPhase::BuildAll,
            PlanPhase::PublishAll,
            PlanPhase::Tag
        ]
    );
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

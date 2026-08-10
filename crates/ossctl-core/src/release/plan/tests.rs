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
        "400133f96f974c3bfe7a6b71d20861c291497c8cde5b23fe79ff96c94c51f855"
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
        "f165e38fac7ac4df378a54a7d2aaa0802ccc3ca166f7eebe17ec542c5b900d20"
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
fn build_emits_the_invariant_phase_sequence() {
    let plan = build(&rust_contract(), &rust_facts(), HEAD, "1.2.0");
    assert_eq!(
        plan.phases,
        vec![
            PlanPhase::DryRunAll,
            PlanPhase::BuildAll,
            PlanPhase::PublishAll,
            PlanPhase::Tag,
            PlanPhase::Dist
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

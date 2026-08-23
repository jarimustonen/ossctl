//! Journal-id derivation: backward-compatible single-target ids, per-target
//! disambiguation for multi-target ecosystems, and determinism.

use super::journal_target_ids;
use crate::contract::schema::{Adapter, Ecosystem, Registry};
use crate::protocol::plan::PlanTarget;

fn t(ecosystem: Ecosystem, package: &str, registry: Registry, adapter: Adapter) -> PlanTarget {
    PlanTarget {
        ecosystem,
        package: Some(package.to_string()),
        registry,
        adapter,
    }
}

#[test]
fn lone_target_per_ecosystem_keeps_the_bare_ecosystem_id() {
    // The historical shape: one target per ecosystem ⇒ the ecosystem wire string
    // is the id, unchanged from before this feature (no journal churn).
    let targets = vec![
        t(
            Ecosystem::Rust,
            "tool",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(Ecosystem::Node, "tool", Registry::Npm, Adapter::NpmPublish),
    ];
    assert_eq!(journal_target_ids(&targets), vec!["rust", "node"]);
}

#[test]
fn two_crates_in_one_ecosystem_disambiguate_by_package() {
    // The issue's canonical case: two crates.io crates in `rust`. Package alone is
    // enough to make them distinct.
    let targets = vec![
        t(
            Ecosystem::Rust,
            "shipshape-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    assert_eq!(
        journal_target_ids(&targets),
        vec!["rust:shipshape-core", "rust:shipshape"]
    );
}

#[test]
fn same_package_different_channels_disambiguate_by_registry() {
    // `shipshape`'s real four-target contract: one crate published to crates.io, plus
    // the same crate as a gh-releases and a homebrew target — package collides, so
    // the id escalates to `package:registry`.
    let targets = vec![
        t(
            Ecosystem::Rust,
            "shipshape-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::GhReleases,
            Adapter::CargoDist,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
    ];
    assert_eq!(
        journal_target_ids(&targets),
        vec![
            "rust:shipshape-core:crates.io",
            "rust:shipshape:crates.io",
            "rust:shipshape:gh-releases",
            "rust:shipshape:homebrew",
        ]
    );
    // Every id is distinct — the property the coordinator's journal keying needs.
    let ids = journal_target_ids(&targets);
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "ids must be unique");
}

#[test]
fn identical_targets_collide_so_the_coordinator_can_reject_them() {
    // Two byte-identical targets cannot be told apart even at full qualification;
    // the derivation yields colliding ids on purpose so `validate_plan` refuses the
    // degenerate duplicate rather than silently keying them the same.
    let targets = vec![
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
    ];
    let ids = journal_target_ids(&targets);
    assert_eq!(
        ids[0], ids[1],
        "identical targets must produce a colliding id"
    );
}

#[test]
fn is_deterministic() {
    let targets = vec![
        t(
            Ecosystem::Rust,
            "shipshape-core",
            Registry::CratesIo,
            Adapter::CargoPublish,
        ),
        t(
            Ecosystem::Rust,
            "shipshape",
            Registry::Homebrew,
            Adapter::HomebrewTap,
        ),
        t(Ecosystem::Node, "tool", Registry::Npm, Adapter::NpmPublish),
    ];
    assert_eq!(journal_target_ids(&targets), journal_target_ids(&targets));
}

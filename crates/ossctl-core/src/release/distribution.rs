//! Facts-to-contract checks for binary-distribution release surfaces.
//!
//! A release contract is authoritative, but a repository can still contain
//! distribution infrastructure it forgot to declare. These checks make that
//! mismatch visible during validation and refuse an irreversible cut before the
//! tag phase can collide with cargo-dist.

use crate::contract::schema::{Registry, Target};
use crate::protocol::facts::DistributionSurface;

/// One under-declared distribution surface found by [`find_undeclared_distribution`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndeclaredDistribution {
    /// cargo-dist configuration or a tag-triggered workflow lacks its delegated
    /// GitHub Release target.
    GhReleases {
        /// Configuration files and workflows establishing the delegated release.
        evidence: Vec<String>,
    },
    /// A declared tap has no engine-owned Homebrew target.
    Homebrew,
}

/// Find distribution infrastructure that the contract's targets omit.
#[must_use]
pub fn find_undeclared_distribution(
    targets: &[Target],
    surface: &DistributionSurface,
    has_unserved_homebrew_tap: bool,
) -> Vec<UndeclaredDistribution> {
    let has_gh_releases = targets
        .iter()
        .any(|target| target.registry == Registry::GhReleases);
    let has_homebrew = targets
        .iter()
        .any(|target| target.registry == Registry::Homebrew);
    let mut findings = Vec::new();

    if (surface.has_cargo_dist || !surface.tag_triggered_workflows.is_empty()) && !has_gh_releases {
        let mut evidence = surface.cargo_dist_evidence.clone();
        evidence.extend(
            surface
                .tag_triggered_workflows
                .iter()
                .map(|name| format!(".github/workflows/{name} (tag-triggered push workflow)")),
        );
        findings.push(UndeclaredDistribution::GhReleases { evidence });
    }
    if has_unserved_homebrew_tap && !has_homebrew {
        findings.push(UndeclaredDistribution::Homebrew);
    }
    findings
}

/// Render findings as validation/plan warnings.
#[must_use]
pub fn undeclared_distribution_warnings(findings: &[UndeclaredDistribution]) -> Vec<String> {
    findings
        .iter()
        .map(|finding| match finding {
            UndeclaredDistribution::GhReleases { evidence } => format!(
                "{} detected, but the contract has no 'gh-releases' target — the tag phase would create the GitHub Release itself and collide with the repo's cargo-dist workflow, dropping its binaries and Homebrew publish. Add a target with registry: gh-releases, adapter: cargo-dist and re-plan",
                evidence.join(", ")
            ),
            UndeclaredDistribution::Homebrew => "distribution.homebrew_tap is set, but the contract has no 'homebrew' target — the tap leg would be silently skipped. Add a target with registry: homebrew, adapter: homebrew-tap and re-plan".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::schema::{Adapter, Ecosystem};

    fn target(registry: Registry) -> Target {
        Target {
            ecosystem: Ecosystem::Rust,
            package: Some("demo".to_string()),
            registry,
            adapter: Adapter::CargoDist,
        }
    }

    #[test]
    fn finds_both_undeclared_surfaces() {
        let surface = DistributionSurface {
            has_cargo_dist: true,
            cargo_dist_evidence: vec!["dist-workspace.toml".to_string()],
            tag_triggered_workflows: vec!["release.yml".to_string()],
        };
        let findings = find_undeclared_distribution(&[], &surface, true);
        assert_eq!(findings.len(), 2);
        let warnings = undeclared_distribution_warnings(&findings);
        assert!(warnings[0].contains("dist-workspace.toml"));
        assert!(warnings[0].contains("release.yml"));
        assert!(warnings[1].contains("homebrew_tap"));
    }

    #[test]
    fn fully_declared_surface_is_green() {
        let surface = DistributionSurface {
            has_cargo_dist: true,
            cargo_dist_evidence: vec!["Cargo.toml ([workspace.metadata.dist])".to_string()],
            tag_triggered_workflows: vec!["release.yml".to_string()],
        };
        let targets = vec![target(Registry::GhReleases), target(Registry::Homebrew)];
        assert!(find_undeclared_distribution(&targets, &surface, true).is_empty());
    }
}

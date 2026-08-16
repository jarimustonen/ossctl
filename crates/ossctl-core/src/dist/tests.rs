//! Unit tests for the cargo-dist config generator.

use super::*;
use crate::contract::schema::{Distribution, DistributionAdapter, Installer};

/// A `Distribution` with the given installers + platforms (adapter cargo-dist).
fn dist(installers: Vec<Installer>, platforms: Vec<&str>) -> Distribution {
    Distribution {
        package: None,
        adapter: DistributionAdapter::CargoDist,
        gh_releases: true,
        installers,
        homebrew_tap: None,
        platforms: platforms.into_iter().map(str::to_string).collect(),
        extra_fields: serde_json::Map::new(),
    }
}

/// The cross-platform default set (what the normalizer materializes for an
/// omitted `platforms`).
fn cross_platform() -> Vec<&'static str> {
    vec![
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
    ]
}

#[test]
fn renders_the_reference_shape() {
    let g = generate(&dist(
        vec![Installer::Shell, Installer::Powershell],
        cross_platform(),
    ));
    let toml = &g.toml;
    // The fixed reference-shape keys are all present.
    assert!(toml.contains("[workspace]"), "{toml}");
    assert!(toml.contains("members = [\"cargo:.\"]"), "{toml}");
    assert!(toml.contains("[dist]"), "{toml}");
    assert!(
        toml.contains(&format!(
            "cargo-dist-version = \"{PINNED_CARGO_DIST_VERSION}\""
        )),
        "{toml}"
    );
    assert!(toml.contains("ci = \"github\""), "{toml}");
    assert!(toml.contains("hosting = \"github\""), "{toml}");
    assert!(toml.contains("github-attestations = true"), "{toml}");
    assert!(toml.contains("pr-run-mode = \"skip\""), "{toml}");
    // No repository-local runner override leaks into a downstream config.
    assert!(
        !toml.contains("github-custom-runners"),
        "the personal self-hosted-runner override must never be generated: {toml}"
    );
}

#[test]
fn targets_are_copied_verbatim_in_order() {
    let platforms = cross_platform();
    let g = generate(&dist(vec![Installer::Shell], platforms.clone()));
    assert_eq!(
        g.targets,
        platforms
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>()
    );
    // Each triple appears as its own quoted array line, in order.
    let idx: Vec<usize> = platforms
        .iter()
        .map(|t| g.toml.find(&format!("\"{t}\",")).expect("triple present"))
        .collect();
    assert!(
        idx.windows(2).all(|w| w[0] < w[1]),
        "targets keep order: {}",
        g.toml
    );
}

#[test]
fn shell_is_ensured_when_omitted() {
    // A contract that lists only powershell still gets shell (the Unix path).
    let g = generate(&dist(vec![Installer::Powershell], cross_platform()));
    assert_eq!(g.installers, vec!["shell", "powershell"], "shell prepended");
    assert!(
        g.warnings
            .iter()
            .any(|w| w.contains("added the 'shell' installer")),
        "warns it added shell: {:?}",
        g.warnings
    );
    assert!(
        g.toml.contains("installers = [\"shell\", \"powershell\"]"),
        "{}",
        g.toml
    );
}

#[test]
fn shell_present_produces_no_added_shell_warning() {
    let g = generate(&dist(vec![Installer::Shell], cross_platform()));
    assert_eq!(g.installers, vec!["shell"]);
    assert!(
        !g.warnings.iter().any(|w| w.contains("added the 'shell'")),
        "no shell-added warning when already present: {:?}",
        g.warnings
    );
}

#[test]
fn homebrew_installer_is_excluded_but_shell_kept() {
    // homebrew is owned by the tap adapter, not cargo-dist.
    let g = generate(&dist(
        vec![Installer::Shell, Installer::Homebrew],
        cross_platform(),
    ));
    assert_eq!(
        g.installers,
        vec!["shell"],
        "homebrew dropped from cargo-dist set"
    );
    assert!(
        !g.toml.contains("homebrew"),
        "no homebrew installer in toml: {}",
        g.toml
    );
    assert!(
        g.warnings
            .iter()
            .any(|w| w.contains("'homebrew' installer")),
        "warns it excluded homebrew: {:?}",
        g.warnings
    );
}

#[test]
fn homebrew_only_still_yields_a_shell_installer() {
    // A contract with homebrew as its only installer must not produce an empty
    // installer set — shell is ensured so there is always a Unix curl path.
    let g = generate(&dist(vec![Installer::Homebrew], cross_platform()));
    assert_eq!(g.installers, vec!["shell"]);
    assert!(g.toml.contains("installers = [\"shell\"]"), "{}", g.toml);
}

#[test]
fn linux_less_platform_set_is_warned() {
    let g = generate(&dist(vec![Installer::Shell], vec!["aarch64-apple-darwin"]));
    assert!(
        g.warnings.iter().any(|w| w.contains("no Linux target")),
        "macOS-only platform set must warn: {:?}",
        g.warnings
    );
}

#[test]
fn cross_platform_default_has_no_linux_warning() {
    let g = generate(&dist(vec![Installer::Shell], cross_platform()));
    assert!(
        !g.warnings.iter().any(|w| w.contains("no Linux target")),
        "the cross-platform default covers Linux: {:?}",
        g.warnings
    );
}

#[test]
fn windows_target_carries_powershell_through() {
    let mut platforms = cross_platform();
    platforms.push("x86_64-pc-windows-msvc");
    let g = generate(&dist(
        vec![Installer::Shell, Installer::Powershell],
        platforms,
    ));
    assert!(g.installers.contains(&"powershell".to_string()));
    assert!(g.toml.contains("x86_64-pc-windows-msvc"), "{}", g.toml);
}

#[test]
fn special_characters_in_a_target_are_escaped() {
    // Defensive: the normalizer never emits such a triple, but a hand-built
    // Distribution must still not produce syntactically-broken TOML.
    let g = generate(&dist(vec![Installer::Shell], vec!["evil\"-\\-linux"]));
    assert!(
        g.toml.contains("\"evil\\\"-\\\\-linux\""),
        "quote and backslash escaped: {}",
        g.toml
    );
    // The rendered value never contains a raw unescaped quote mid-token.
    assert!(
        !g.toml.contains("evil\"-\\-linux"),
        "raw value must not leak: {}",
        g.toml
    );
}

#[test]
fn output_is_deterministic() {
    let d = dist(
        vec![Installer::Shell, Installer::Powershell],
        cross_platform(),
    );
    assert_eq!(generate(&d), generate(&d), "same input → identical output");
}

#[test]
fn generated_toml_is_well_formed() {
    // Structural guard against a rendering bug (no `toml` dep in ossctl-core, so
    // this checks the shape lexically rather than via a parser).
    let g = generate(&dist(
        vec![Installer::Shell, Installer::Powershell, Installer::Homebrew],
        cross_platform(),
    ));
    let toml = &g.toml;
    // Exactly one [workspace] and one [dist] table header (a line that IS the
    // header, not a comment mentioning it).
    assert_eq!(
        toml.lines().filter(|l| *l == "[workspace]").count(),
        1,
        "{toml}"
    );
    assert_eq!(toml.lines().filter(|l| *l == "[dist]").count(), 1, "{toml}");
    // Every non-comment, non-blank line inside a table is a `key = value` pair,
    // a bare table header, or part of the multi-line `targets` array.
    let mut in_targets = false;
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
            continue;
        }
        if in_targets {
            if trimmed == "]" {
                in_targets = false;
            } else {
                assert!(
                    trimmed.starts_with('"') && trimmed.ends_with("\","),
                    "target array line must be a quoted, comma-terminated triple: {line:?}"
                );
            }
            continue;
        }
        if trimmed == "targets = [" {
            in_targets = true;
            continue;
        }
        assert!(
            trimmed.contains(" = "),
            "expected a `key = value` line, got: {line:?}"
        );
    }
    assert!(
        !in_targets,
        "the targets array must be closed with ]: {toml}"
    );
}

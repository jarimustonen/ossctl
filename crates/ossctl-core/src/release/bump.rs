//! The engine-owned version-bump arithmetic (`release-rust-workspace-multicrate`
//! facet 2).
//!
//! `ossctl release plan --bump major|minor|patch` supplies only a semantic level;
//! the engine **computes** the new version from the current manifest version — there
//! is no hand-typed literal version (`--version` was removed in 0.3.0,
//! `release-drop-version-flag`, and stays removed). This module is the pure,
//! side-effect-free core of that computation: parse a strict `X.Y.Z` version and
//! apply a [`BumpLevel`]. It fails **closed** on a non-semver manifest version rather
//! than guess, so a malformed version aborts `release plan` instead of publishing an
//! unintended number.
//!
//! The bump is strict `MAJOR.MINOR.PATCH` (three non-negative integers): a
//! pre-release or build-metadata version (`1.2.3-rc.1`, `1.2.3+build`) is refused —
//! bumping such a version is ambiguous, and a release cut publishes a plain release
//! version, so refusing is the safe, unsurprising behaviour.

use crate::protocol::plan::BumpLevel;

/// Why a manifest version could not be bumped: it is not a strict `X.Y.Z` semver
/// core, so the engine will not guess a new number (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BumpError {
    /// The offending version string, echoed for the CLI's `invalid_value`.
    pub version: String,
    /// Why it was rejected (human-readable), e.g. "expected MAJOR.MINOR.PATCH".
    pub reason: String,
}

/// Compute the next version by applying `level` to a strict `X.Y.Z` `current`
/// version.
///
/// - `major` → `(X+1).0.0`
/// - `minor` → `X.(Y+1).0`
/// - `patch` → `X.Y.(Z+1)`
///
/// # Errors
/// [`BumpError`] when `current` is not a strict `MAJOR.MINOR.PATCH` of three
/// non-negative integers (a pre-release/build suffix, a missing/extra component, a
/// non-numeric or empty component, or a `u64`-overflowing component). Failing closed
/// here means a malformed manifest version aborts the plan rather than silently
/// producing a wrong release version.
pub fn bump_version(level: BumpLevel, current: &str) -> Result<String, BumpError> {
    let (major, minor, patch) = parse_semver_core(current)?;
    let (major, minor, patch) = match level {
        // A checked add keeps the (practically unreachable) `u64::MAX` overflow a loud
        // error rather than a wrapped, silently-wrong version.
        BumpLevel::Major => (checked_incr(major, current)?, 0, 0),
        BumpLevel::Minor => (major, checked_incr(minor, current)?, 0),
        BumpLevel::Patch => (major, minor, checked_incr(patch, current)?),
    };
    Ok(format!("{major}.{minor}.{patch}"))
}

/// Parse a strict `MAJOR.MINOR.PATCH` core into its three integers, rejecting
/// anything else.
fn parse_semver_core(v: &str) -> Result<(u64, u64, u64), BumpError> {
    let reject = |reason: &str| BumpError {
        version: v.to_string(),
        reason: reason.to_string(),
    };
    // A pre-release (`-`) or build-metadata (`+`) suffix is not a plain release
    // version — refuse rather than bump ambiguously.
    if v.contains('-') || v.contains('+') {
        return Err(reject(
            "a pre-release or build-metadata version cannot be bumped; expected a plain \
             MAJOR.MINOR.PATCH release version",
        ));
    }
    let mut parts = v.split('.');
    let mut next = |which: &str| -> Result<u64, BumpError> {
        let comp = parts
            .next()
            .ok_or_else(|| reject("expected MAJOR.MINOR.PATCH (a component is missing)"))?;
        parse_component(comp, which, v)
    };
    let major = next("major")?;
    let minor = next("minor")?;
    let patch = next("patch")?;
    // A fourth component (or trailing dot) is not `X.Y.Z`.
    if parts.next().is_some() {
        return Err(reject(
            "expected exactly MAJOR.MINOR.PATCH (too many components)",
        ));
    }
    Ok((major, minor, patch))
}

/// Parse one version component as a non-negative integer, rejecting empty,
/// non-digit, or leading-zero forms (`01`) so the version is canonical.
fn parse_component(comp: &str, which: &str, full: &str) -> Result<u64, BumpError> {
    let reject = |reason: String| BumpError {
        version: full.to_string(),
        reason,
    };
    if comp.is_empty() {
        return Err(reject(format!("the {which} component is empty")));
    }
    if !comp.bytes().all(|b| b.is_ascii_digit()) {
        return Err(reject(format!(
            "the {which} component `{comp}` is not a non-negative integer"
        )));
    }
    // Reject a non-canonical leading zero (`01`) — `0` itself is fine.
    if comp.len() > 1 && comp.starts_with('0') {
        return Err(reject(format!(
            "the {which} component `{comp}` has a leading zero"
        )));
    }
    comp.parse::<u64>().map_err(|_| {
        reject(format!(
            "the {which} component `{comp}` does not fit in a u64"
        ))
    })
}

/// Increment a component, turning the (unreachable in practice) overflow into a
/// loud [`BumpError`] rather than a wrapped value.
fn checked_incr(n: u64, full: &str) -> Result<u64, BumpError> {
    n.checked_add(1).ok_or_else(|| BumpError {
        version: full.to_string(),
        reason: "a version component would overflow on bump".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_minor_major_from_a_normal_version() {
        assert_eq!(bump_version(BumpLevel::Patch, "0.4.0").unwrap(), "0.4.1");
        assert_eq!(bump_version(BumpLevel::Minor, "0.4.0").unwrap(), "0.5.0");
        assert_eq!(bump_version(BumpLevel::Major, "0.4.0").unwrap(), "1.0.0");
    }

    #[test]
    fn minor_and_major_reset_lower_components() {
        assert_eq!(bump_version(BumpLevel::Minor, "1.2.3").unwrap(), "1.3.0");
        assert_eq!(bump_version(BumpLevel::Major, "1.2.3").unwrap(), "2.0.0");
        assert_eq!(bump_version(BumpLevel::Patch, "1.2.3").unwrap(), "1.2.4");
    }

    #[test]
    fn zero_versions_bump_canonically() {
        assert_eq!(bump_version(BumpLevel::Patch, "0.0.0").unwrap(), "0.0.1");
        assert_eq!(bump_version(BumpLevel::Minor, "0.0.0").unwrap(), "0.1.0");
        assert_eq!(bump_version(BumpLevel::Major, "0.0.0").unwrap(), "1.0.0");
    }

    #[test]
    fn a_pre_release_or_build_version_is_refused() {
        assert!(bump_version(BumpLevel::Patch, "1.2.3-rc.1").is_err());
        assert!(bump_version(BumpLevel::Patch, "1.2.3+build.5").is_err());
    }

    #[test]
    fn a_non_xyz_version_is_refused() {
        for bad in ["1.2", "1.2.3.4", "1", "", "v1.2.3", "1.2.x", "1..2", "1.2."] {
            assert!(
                bump_version(BumpLevel::Patch, bad).is_err(),
                "expected `{bad}` to be refused"
            );
        }
    }

    #[test]
    fn a_leading_zero_component_is_refused() {
        assert!(bump_version(BumpLevel::Patch, "1.02.3").is_err());
        assert!(bump_version(BumpLevel::Patch, "01.2.3").is_err());
        // But a bare zero component is canonical and fine.
        assert!(bump_version(BumpLevel::Patch, "0.1.0").is_ok());
    }

    #[test]
    fn the_error_carries_the_offending_version() {
        let err = bump_version(BumpLevel::Patch, "not-semver").unwrap_err();
        assert_eq!(err.version, "not-semver");
        assert!(!err.reason.is_empty());
    }
}

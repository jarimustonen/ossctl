//! The ONE canonical serde model for `OSS-RELEASE.md` (ADR-0003 §1).
//!
//! These types are the single normalization model that `contract show`,
//! `contract validate`, `audit`, the facts consumers, and release planning all
//! use — no second parser anywhere. Their serialized form is the canonical
//! JSON contract every `/oss-*` member reads (SCHEMA.md §4, preserved
//! byte-for-shape by the migration rule). Public wire access goes through
//! [`crate::protocol::contract`], which re-exports these types and owns the
//! wire-version declaration so internals and wire can diverge under the
//! migration rule (ADR-0001 §2).
//!
//! Hot file (ADR-0001): a change here ripples to every family member. Every
//! enum's [`as_str`](Status::as_str) form is the wire string; a change to one is
//! a `schema_version` bump, never silent.

use serde::Serialize;

/// The contract `schema_version` this build knows how to read.
///
/// A config declaring a higher version is refused rather than guessed
/// (SCHEMA.md §2 floor 5) — skills upgrade independently of the repos they act
/// on. Distinct from the wire-envelope [`crate::SCHEMA_VERSION`]; both are `1`
/// today but version different things (the contract document vs. the JSON
/// envelope). Mirrors the Python `KNOWN_SCHEMA_VERSION`.
pub const KNOWN_SCHEMA_VERSION: u32 = 1;

/// The changelog fragment directory materialized when the config omits it.
pub const DEFAULT_FRAGMENT_DIR: &str = "changelog/fragments";

/// Define a closed enum whose variants each map to a fixed wire string.
///
/// Generates the enum (with the standard derives), [`as_str`] (variant → wire),
/// [`parse`] (wire → variant), the `VALID` slice of wire strings for error
/// messages, and a `Serialize` impl that emits the wire string. `Deserialize`
/// is intentionally not generated: the normalizer reads strings out of the
/// parsed YAML and validates each with [`parse`] so it can collect *all* errors
/// and substitute a default (mirroring the Python normalizer), rather than
/// fail-fast on the first bad enum.
macro_rules! wire_enum {
    (
        $(#[$emeta:meta])*
        $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
    ) => {
        $(#[$emeta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                #[doc = concat!("Wire value `", $wire, "`.")]
                $variant,
            )+
        }

        impl $name {
            /// The wire string for this variant (matches SCHEMA.md §4).
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }

            /// Parse a wire string into the variant, or `None` if unrecognized.
            #[must_use]
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($wire => Some(Self::$variant),)+ _ => None }
            }

            /// Every valid wire string, for "must be one of …" messages.
            pub const VALID: &'static [&'static str] = &[$($wire),+];
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(self.as_str())
            }
        }
    };
}

wire_enum! {
    /// Machine-readable approval gate (SCHEMA.md §1). `/oss-init` writes `draft`
    /// and stops; a human flips it to `approved`. Mutating members refuse a
    /// draft (they pass `--require-approved`).
    Status { Draft => "draft", Approved => "approved" }
}

wire_enum! {
    /// Master maturity dial that gates every member's output (SCHEMA.md §1).
    /// Required — inference is `/oss-init`'s job, not the normalizer's.
    Maturity { Spike => "spike", Mvp => "mvp", Production => "production" }
}

wire_enum! {
    /// A packaging ecosystem. `homebrew` is a distribution *target*, never an
    /// ecosystem. Listed in the canonical order used for stable de-dup/expansion.
    Ecosystem {
        Rust => "rust", Node => "node", Python => "python", Go => "go", Binary => "binary"
    }
}

wire_enum! {
    /// Base versioning scheme (SCHEMA.md §1). A `calver:<pattern>` config splits
    /// into `Calver` + a separate [`Contract::versioning_pattern`]; the wire form
    /// never carries the `calver:` prefix.
    VersioningBase { Semver => "semver", Calver => "calver", Zerover => "zerover" }
}

wire_enum! {
    /// How the changelog is produced (SCHEMA.md §1).
    ChangelogMode { Curated => "curated", Automated => "automated", Fragment => "fragment" }
}

wire_enum! {
    /// The changelog's structured input (SCHEMA.md §1).
    ChangelogSource {
        IssuectlTrailers => "issuectl-trailers",
        ConventionalCommits => "conventional-commits",
        Manual => "manual"
    }
}

wire_enum! {
    /// Release trigger model (SCHEMA.md §1). `auto` installs an on-merge
    /// workflow; it never publishes from a chat turn.
    ReleaseModel { Gated => "gated", Auto => "auto" }
}

wire_enum! {
    /// Repository release layout (SCHEMA.md §1). `monorepo` drives per-package
    /// versions/tags and flips the node adapter default to `changesets`.
    ReleaseLayout { Single => "single", Monorepo => "monorepo" }
}

wire_enum! {
    /// Contributor sign-off requirement, read by `/oss-contributing`.
    ContributionProvenance { Dco => "dco", Cla => "cla", None => "none" }
}

wire_enum! {
    /// Build-provenance level (SCHEMA.md §1). `slsa-l3` is production-only (floor).
    ProvenanceLevel { None => "none", Keyless => "keyless", SlsaL3 => "slsa-l3" }
}

wire_enum! {
    /// Which dependency-update bot `/oss-ci` emits.
    DependencyBot { Dependabot => "dependabot", Renovate => "renovate", None => "none" }
}

wire_enum! {
    /// A README health badge (SCHEMA.md §1). Every badge needs its producer
    /// enabled (floor 4).
    HealthBadge {
        Ci => "ci", Registry => "registry", License => "license",
        Coverage => "coverage", Scorecard => "scorecard", Discord => "discord"
    }
}

wire_enum! {
    /// Optional documentation-site generator (SCHEMA.md §1); production-tier.
    DocsSite {
        None => "none", Mkdocs => "mkdocs", Vitepress => "vitepress",
        Docusaurus => "docusaurus", Sphinx => "sphinx", Mintlify => "mintlify"
    }
}

wire_enum! {
    /// A publish destination for a [`Target`] (SCHEMA.md §1).
    Registry {
        CratesIo => "crates.io", Npm => "npm", Pypi => "pypi", TestPypi => "testpypi",
        GhReleases => "gh-releases", ProxyGolangOrg => "proxy.golang.org",
        Homebrew => "homebrew"
    }
}

wire_enum! {
    /// The release tool pinned for a [`Target`] so it is not re-inferred each cut.
    Adapter {
        CargoPublish => "cargo-publish", CargoDist => "cargo-dist",
        ReleasePlease => "release-please", Changesets => "changesets",
        GhActionPypiPublish => "gh-action-pypi-publish", Twine => "twine",
        Goreleaser => "goreleaser", HomebrewTap => "homebrew-tap",
        HomebrewCore => "homebrew-core", NpmPublish => "npm-publish", Manual => "manual"
    }
}

impl Ecosystem {
    /// The default registry for this ecosystem when `targets` is expanded
    /// (SCHEMA.md §1 default-expansion table).
    #[must_use]
    pub fn default_registry(self) -> Registry {
        match self {
            Self::Rust => Registry::CratesIo,
            Self::Node => Registry::Npm,
            Self::Python => Registry::Pypi,
            Self::Go => Registry::ProxyGolangOrg,
            Self::Binary => Registry::GhReleases,
        }
    }

    /// The default adapter for this ecosystem/layout when `targets` is expanded
    /// (SCHEMA.md §1). Node's default is layout-sensitive: `single` →
    /// `release-please`, `monorepo` → `changesets`.
    #[must_use]
    pub fn default_adapter(self, layout: ReleaseLayout) -> Adapter {
        match self {
            Self::Rust => Adapter::CargoPublish,
            Self::Node => match layout {
                ReleaseLayout::Monorepo => Adapter::Changesets,
                ReleaseLayout::Single => Adapter::ReleasePlease,
            },
            Self::Python => Adapter::GhActionPypiPublish,
            Self::Go => Adapter::Goreleaser,
            Self::Binary => Adapter::Manual,
        }
    }
}

/// One concrete `ecosystem → package → registry` publish destination.
///
/// Always concrete in the canonical output: expanded from `ecosystems` when the
/// source omitted `targets`. `package` may be `null` (the executor infers it
/// from the manifest); every other field is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    /// The ecosystem this target publishes for.
    pub ecosystem: Ecosystem,
    /// The package/crate name, or `null` when inferred from the manifest.
    pub package: Option<String>,
    /// The publish destination.
    pub registry: Registry,
    /// The release tool pinned for this target.
    pub adapter: Adapter,
}

/// The changelog block of the contract (SCHEMA.md §1). `fragment_dir` is always
/// present, even for non-`fragment` modes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Changelog {
    /// How the changelog is produced.
    pub mode: ChangelogMode,
    /// The changelog's structured input.
    pub source: ChangelogSource,
    /// Where changelog fragments live (relative path inside the repo).
    pub fragment_dir: String,
}

/// The release block of the contract (SCHEMA.md §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    /// Release trigger model.
    pub model: ReleaseModel,
    /// Repository release layout.
    pub layout: ReleaseLayout,
}

/// The canonical, fully-defaulted, `targets`-expanded `OSS-RELEASE.md` contract.
///
/// This is the exact shape of SCHEMA.md §4 (the stable machine contract). Every
/// field is present and defaulted; `versioning` is the base enum with the
/// calver pattern split into [`Self::versioning_pattern`]; `extra_fields` holds
/// preserved unknown frontmatter keys (forward-compat); `warnings` holds the
/// non-fatal notes. Field order matches SCHEMA.md §4 for readable output; JSON
/// consumers key-access, so order is not part of the contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contract {
    /// Contract schema version (bounded by [`KNOWN_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Approval gate.
    pub status: Status,
    /// Maturity dial.
    pub maturity: Maturity,
    /// Packaging ecosystems, de-duplicated to canonical order.
    pub ecosystems: Vec<Ecosystem>,
    /// Concrete publish targets (expanded from `ecosystems` when omitted).
    pub targets: Vec<Target>,
    /// Base versioning scheme.
    pub versioning: VersioningBase,
    /// The calver pattern string, or `null` for non-calver schemes.
    pub versioning_pattern: Option<String>,
    /// Changelog configuration.
    pub changelog: Changelog,
    /// Whether `/oss-release-cut` may derive the bump from commit types.
    pub conventional_commits: bool,
    /// Release model + layout.
    pub release: Release,
    /// Contributor sign-off requirement.
    pub contribution_provenance: ContributionProvenance,
    /// Build-provenance level.
    pub provenance_level: ProvenanceLevel,
    /// Dependency-update bot.
    pub dependency_bot: DependencyBot,
    /// README health badges.
    pub health_badges: Vec<HealthBadge>,
    /// SPDX license id/expression.
    pub license: String,
    /// Optional documentation-site generator.
    pub docs_site: DocsSite,
    /// Preserved unknown frontmatter keys under a known `schema_version`
    /// (forward-compat); never dropped.
    pub extra_fields: serde_json::Map<String, serde_json::Value>,
    /// Non-fatal notes (aspirational draft producers, the unknown-field report).
    pub warnings: Vec<String>,
}

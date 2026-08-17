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
/// on. Distinct from the wire-envelope [`crate::SCHEMA_VERSION`], which versions
/// the JSON envelope, not the contract document. Mirrors the Python
/// `KNOWN_SCHEMA_VERSION`.
///
/// **Bumped `1` → `2`** for the monorepo-distribution change: the single
/// top-level [`Distribution`] key `distribution` (an object-or-`null`) became the
/// collection [`Contract::distributions`] (`distributions`, always a JSON array),
/// and every distribution gained an association key [`Distribution::package`].
/// Renaming the canonical key and re-shaping the value is a **breaking** change,
/// not a pure addition, so it bumps deliberately (never silently). This tool
/// still *reads* a v1 document — a bare `distribution:` mapping deserializes as a
/// one-element `distributions` list — but *emits* the v2 canonical shape.
///
/// A purely additive field (a new optional top-level key defaulting to
/// absent/`null`) does NOT bump: an older reader preserves the unknown key under
/// [`Contract::extra_fields`] and warns rather than failing. The migration rule
/// bumps only on a **breaking** change — renaming/removing a field or re-meaning
/// an existing one — never on a pure addition, which the forward-compat mechanism
/// absorbs.
pub const KNOWN_SCHEMA_VERSION: u32 = 2;

/// The changelog fragment directory materialized when the config omits it.
pub const DEFAULT_FRAGMENT_DIR: &str = "changelog/fragments";

/// The cross-platform default [`Distribution::platforms`] set materialized when a
/// distribution block omits `platforms`: macOS (`aarch64` + `x86_64`) and Linux
/// (`aarch64` + `x86_64`). This is the KEYSTONE of the cross-platform install
/// requirement — a distribution that OMITS `platforms` covers Linux **by
/// default**, so a repo that never thinks about it still ships Linux binaries. (A
/// repo that sets `platforms` explicitly owns its own coverage; the cross-platform
/// `audit` — not this default — flags a Linux-less explicit set.) musl over gnu for
/// Linux: for a pure-Rust CLI a musl target links statically and sidesteps the
/// glibc-version cliff — though choosing a musl *target* does not by itself
/// guarantee a static build, and a repo with C/native dependencies (`openssl-sys`,
/// `libgit2`, …) may need to override to gnu. Windows is a deliberate omission (a
/// bonus a repo opts into by listing it explicitly, never the default). The set
/// always contains at least one Linux triple.
pub const DEFAULT_CROSS_PLATFORM_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];

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
    ///
    /// `cargo-publish-ci` is the **CI-delegated** sibling of `cargo-publish`: the
    /// crate reaches crates.io through a tag-triggered CI workflow (a repo-secret
    /// `CARGO_REGISTRY_TOKEN` running `cargo publish` in Actions), never through a
    /// `cargo publish` on the maintainer's host. A repo that forbids the local
    /// publish declares it, and the engine's cut then gates + tags + **observes**
    /// instead of publishing — the same delegation vocabulary the `cargo-dist`
    /// gh-releases / homebrew targets already use
    /// ([`is_ci_delegated`](crate::release::adapters::ReleaseAdapter::is_ci_delegated)).
    ///
    /// A **new enum value is additive**, so it does not bump
    /// [`KNOWN_SCHEMA_VERSION`]: no existing field is renamed or re-meant, and a
    /// contract that does not use it serializes byte-for-byte as before. An older
    /// reader meeting the value reports it as an invalid adapter (the normalizer's
    /// closed-enum error path) rather than mis-executing it — fail-closed, which is
    /// the property that matters for a publish identity.
    Adapter {
        CargoPublish => "cargo-publish", CargoPublishCi => "cargo-publish-ci",
        CargoDist => "cargo-dist",
        ReleasePlease => "release-please", Changesets => "changesets",
        GhActionPypiPublish => "gh-action-pypi-publish", Twine => "twine",
        Goreleaser => "goreleaser", HomebrewTap => "homebrew-tap",
        HomebrewCore => "homebrew-core", NpmPublish => "npm-publish", Manual => "manual"
    }
}

wire_enum! {
    /// The binary-distribution engine that produces multi-platform GitHub-Release
    /// artifacts plus a generated installer set (distinct from a registry
    /// [`Adapter`]). Owned by a tag-triggered `release.yml` the family must NOT
    /// regenerate — hence first-class in the contract.
    DistributionAdapter {
        CargoDist => "cargo-dist", Goreleaser => "goreleaser", Manual => "manual"
    }
}

wire_enum! {
    /// An installer flavor a [`Distribution`] emits. `homebrew` requires a
    /// [`Distribution::homebrew_tap`] (floor).
    Installer {
        Shell => "shell", Powershell => "powershell", Homebrew => "homebrew",
        Msi => "msi", Npm => "npm"
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

/// The binary-distribution block: multi-platform GitHub-Release binaries, a
/// generated installer set, and an optional Homebrew tap — produced by a
/// tag-triggered release workflow (cargo-dist / goreleaser).
///
/// SEPARATE from [`Target`] (registry publishes): a cargo-dist repo attaches
/// per-platform binaries to its GitHub Release, ships a shell/Homebrew installer,
/// **and** independently publishes its crate to crates.io — the crates.io publish
/// is a [`Target`]; everything binary-distribution is this block. The two coexist,
/// which is exactly the "registry publish alongside a cargo-dist release" the
/// contract could not express before. First-class (not prose) so downstream
/// members SEE the tap + installer and neither under-describe the release nor
/// regenerate the existing `release.yml`.
///
/// One element of [`Contract::distributions`]: a registry-only repo has an empty
/// list, a single-binary repo one entry, a **monorepo** one entry per
/// independently-distributed binary (each with its own installers / tap /
/// platforms), tagged by [`Self::package`].
///
/// Keeps `Eq` even after gaining [`Self::extra_fields`]: `serde_json::Value`
/// (and `serde_json::Map`) implement `Eq` — JSON numbers exclude non-finite
/// floats — so the added field does not weaken the derive (unlike the sibling
/// [`Contract`], which is `PartialEq`-only for unrelated historical reasons).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Distribution {
    /// The package/target this distribution belongs to (matches a
    /// [`Target::package`] or a manifest package name), or `null` for the sole /
    /// unassociated distribution.
    ///
    /// The monorepo association key: a repo shipping multiple independently
    /// distributed binaries gives each its own [`Distribution`] tagged with the
    /// package it builds. `null` is the single-distribution back-compat case (a
    /// bare `distribution:` mapping carries no package). The normalizer requires a
    /// non-null, **unique** `package` on every entry once there are two or more
    /// distributions — otherwise a monorepo's entries would be indistinguishable.
    pub package: Option<String>,
    /// The binary-distribution engine that owns the tag-triggered release
    /// workflow.
    pub adapter: DistributionAdapter,
    /// Whether multi-platform binaries are attached to the GitHub Release.
    pub gh_releases: bool,
    /// Installer flavors this release produces (may be empty), canonically
    /// ordered and de-duplicated.
    pub installers: Vec<Installer>,
    /// The Homebrew tap repo (`owner/repo`) the generated formula is pushed to,
    /// or `null` when no tap is used. Required when `installers` includes
    /// `homebrew` (floor).
    pub homebrew_tap: Option<String>,
    /// The platform target set — the target-triples this binary distribution builds
    /// and ships, in Rust target-triple form (the vocabulary the `cargo-dist`
    /// adapter consumes; a `goreleaser`/`manual` remodel into an adapter-neutral
    /// shape is deliberately left to a follow-up). Always non-empty in the canonical
    /// output: defaulted to the cross-platform [`DEFAULT_CROSS_PLATFORM_TARGETS`] set
    /// (macOS + Linux) when the source OMITS it (an explicit empty list is rejected,
    /// not defaulted), so a distribution that doesn't specify platforms still covers
    /// Linux (the cross-platform install requirement). An explicit set is validated
    /// per triple and de-duplicated, preserving the author's order. Validation is
    /// STRUCTURAL, not semantic — a well-formed triple whose OS component stays
    /// inspectable, so the cross-platform `audit` can flag a Linux-less explicit set;
    /// the normalizer guarantees only that the field is present and every triple
    /// well-formed, never that the set covers any particular OS or that the toolchain
    /// will build it.
    pub platforms: Vec<String>,
    /// Preserved unknown keys inside the `distribution` block under a known
    /// `schema_version` (forward-compat), so an older reader round-trips a newer
    /// contract's distribution sub-keys rather than dropping them. Mirrors
    /// [`Contract::extra_fields`] at the nested level; empty for a contract with
    /// no unknown distribution keys.
    ///
    /// An EMPTY map is OMITTED from canonical JSON (`skip_serializing_if`), so a
    /// distribution with no unknown keys carries no `extra_fields` key at all — the
    /// "additive = absent-by-default" migration rule holds literally, and a
    /// populated map serializes exactly as before. Kept symmetric with
    /// [`Contract::extra_fields`] (both omit-when-empty).
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra_fields: serde_json::Map<String, serde_json::Value>,
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
    /// An optional repo-provided command the engine runs in the clean checkout during
    /// the engine-owned version-bump phase, **after** the version edits and **before**
    /// the bump commit (`release-rust-workspace-multicrate` facet 3). Its purpose is to
    /// regenerate version-embedding artifacts — the canonical case is a repo whose test
    /// snapshots embed the version (insta `envelope_snapshots__version_*`), which go
    /// stale on a bump and red CI unless regenerated against the new version. Keeping it
    /// a declared command keeps the engine out of per-repo test-harness specifics: the
    /// engine folds the hook's file changes into the bump commit and **fails closed** if
    /// the hook exits non-zero. `null` (absent) = no hook, the default.
    ///
    /// **Security (trust boundary).** A declared hook is **arbitrary code the engine
    /// runs during a release**, with whatever environment the cut carries (potentially
    /// registry-publish credentials), before the publish barrier. It is equivalent in
    /// trust to a `build.rs` / a test the release already runs, but it lives in the
    /// contract, which may be reviewed less carefully than Rust source — so a malicious
    /// `bump_hook` is a supply-chain surface. The engine surfaces the hook **verbatim**
    /// as a plan-time warning so an approver sees exactly what will run, and the
    /// executor's invocation contract (shell vs argv, working directory, timeout,
    /// environment/secret policy, permitted file changes) is specified where execution
    /// is wired — until then no hook is ever run (`release cut` refuses a bump plan).
    ///
    /// A **purely additive** optional field: it is omitted from the canonical JSON when
    /// absent (`skip_serializing_if`), so a contract that declares no hook serializes
    /// byte-for-byte as before and its `plan_id` is unchanged. By the migration rule an
    /// additive optional field does not bump `schema_version`; and this codebase parses
    /// the contract through a hand-written normalizer (no serde `Deserialize`), so there
    /// is no missing-field deser hazard for older readers, which in any case lack
    /// `--bump` and so never act on the hook.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bump_hook: Option<String>,
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
    /// Concrete registry publish targets. Expanded from `ecosystems` when the
    /// `targets` key is OMITTED (or written as a bare `targets:` / `targets: null`,
    /// which read as absent); an explicit empty `targets: []` — the literal empty
    /// sequence — is the author's authoritative "never publish anywhere" and is
    /// honored as an empty set (not re-expanded), the machine-readable way to
    /// declare a version-tracked but unpublished repo. An empty set is a valid,
    /// honored state, not a misconfiguration. This re-meaning of the specific `[]`
    /// value did NOT bump [`KNOWN_SCHEMA_VERSION`] deliberately: the serialized
    /// shape is a JSON array either way (an empty array is already producible today
    /// by a contract with no ecosystems), so every consumer that reads `targets`
    /// already handles `[]` — no reader breaks.
    pub targets: Vec<Target>,
    /// The binary-distribution blocks (cargo-dist / goreleaser binaries +
    /// installers + Homebrew tap). Empty for a registry-only repo, one entry for
    /// a single-binary repo, one per independently-distributed binary for a
    /// **monorepo** (each tagged by [`Distribution::package`]). Always a JSON
    /// array in canonical output. Coexists with `targets` — a cargo-dist repo has
    /// both. A bare `distribution:` mapping in the source deserializes as a
    /// one-element list (v1 back-compat); a `distributions:` sequence carries many.
    pub distributions: Vec<Distribution>,
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
    ///
    /// An EMPTY map is OMITTED from canonical JSON (`skip_serializing_if`), so a
    /// contract with no unknown keys carries no `extra_fields` key at all — the
    /// "additive = absent-by-default" migration rule holds literally, and a
    /// populated map serializes exactly as before. Kept symmetric with
    /// [`Distribution::extra_fields`] (both omit-when-empty).
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra_fields: serde_json::Map<String, serde_json::Value>,
    /// Non-fatal notes (aspirational draft producers, the unknown-field report).
    pub warnings: Vec<String>,
}

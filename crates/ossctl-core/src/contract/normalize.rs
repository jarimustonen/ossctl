//! Normalization pipeline: validate every field/enum/cross-field floor,
//! materialize all defaults, and expand `targets` from `ecosystems`.
//!
//! A faithful port of `check-oss-release.py`'s `normalize`. `contract show`
//! emits the canonical (normalized) form; `contract validate` runs the identical
//! pipeline and discards the document, emitting only pass/fail (ADR-0001 §1).
//!
//! Every field is validated independently and **all** problems are collected —
//! an invalid enum records an error *and* substitutes a default so the pass
//! continues to surface every other problem (mirroring the Python `Problems`
//! collector). The built [`Contract`] is only meaningful when
//! [`Normalized::is_valid`] holds; callers gate on that (or, in the CLI, on the
//! process exit code), never on parsing a document that failed.

use std::io;
use std::path::{Component, Path, PathBuf};

use serde_yaml::{Mapping, Value};

use crate::contract::schema::{
    Adapter, Changelog, ChangelogMode, ChangelogSource, Contract, ContributionProvenance,
    DependencyBot, Distribution, DistributionAdapter, DocsSite, Ecosystem, HealthBadge, Installer,
    Maturity, ProvenanceLevel, Registry, Release, ReleaseLayout, ReleaseModel, Status, Target,
    VersioningBase, DEFAULT_CROSS_PLATFORM_TARGETS, DEFAULT_FRAGMENT_DIR, KNOWN_SCHEMA_VERSION,
};
use crate::contract::spdx::spdx_valid;
use crate::ports::Fs;

/// The contract file the normalizer reads, relative to the repo root.
pub const CONTRACT_FILENAME: &str = "OSS-RELEASE.md";

/// Canonical ecosystem order — used to de-duplicate and stably order the
/// `ecosystems` list (mirrors the Python `VALID_ECOSYSTEMS` ordered list).
const ECOSYSTEM_ORDER: [Ecosystem; 5] = [
    Ecosystem::Rust,
    Ecosystem::Node,
    Ecosystem::Python,
    Ecosystem::Go,
    Ecosystem::Binary,
];

/// Known top-level frontmatter keys; anything else is preserved under
/// [`Contract::extra_fields`] (forward-compat).
///
/// **Invariant:** this list MUST stay in sync with the [`Contract`] struct
/// fields — every parsed field has its source key here. A field added to
/// [`Contract`] without its key here would be captured as an "unknown" field on
/// input; the `all_known_keys_*` tests guard against that drift.
///
/// The two trailing entries — `extra_fields` and `warnings` — are the canonical
/// *output* metadata keys, reserved here so canonical JSON (which carries them)
/// re-fed to the normalizer as YAML does NOT re-capture them into a nested
/// `extra_fields.extra_fields` on each pass. They are handled asymmetrically:
/// `extra_fields`'s mapping contents are merged back into the captured map (see
/// [`capture_unknown_fields`]) so the block round-trips losslessly; `warnings` is
/// derived diagnostic output, regenerated every pass, so any input value under it
/// is intentionally ignored (not preserved — it is not user contract data).
const KNOWN_KEYS: &[&str] = &[
    "schema_version",
    "status",
    "maturity",
    "ecosystems",
    "targets",
    "distribution",
    "versioning",
    "changelog",
    "conventional_commits",
    "release",
    "contribution_provenance",
    "provenance_level",
    "dependency_bot",
    "health_badges",
    "license",
    "docs_site",
    // Reserved canonical-output metadata keys (not parsed) — see doc above.
    "extra_fields",
    "warnings",
];

/// Collected fatal errors and non-fatal warnings from a normalization pass.
#[derive(Debug, Default)]
pub struct Problems {
    /// Fatal validation errors; a non-empty list means the config would not
    /// normalize (the CLI exits non-zero with the §10 error envelope).
    pub errors: Vec<String>,
    /// Non-fatal notes (aspirational draft producers, the unknown-field report).
    pub warnings: Vec<String>,
}

impl Problems {
    fn err(&mut self, msg: String) {
        self.errors.push(msg);
    }

    fn warn(&mut self, msg: String) {
        self.warnings.push(msg);
    }
}

/// The result of a normalization pass: the canonical [`Contract`] plus the
/// [`Problems`] gathered while building it.
#[derive(Debug)]
pub struct Normalized {
    /// The canonical contract. Only meaningful when [`Self::is_valid`] holds.
    pub contract: Contract,
    /// Errors and warnings gathered during normalization.
    pub problems: Problems,
}

impl Normalized {
    /// Whether the config normalized cleanly (no fatal errors).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.problems.errors.is_empty()
    }
}

/// Why the contract file could not be loaded (distinct from a *validation*
/// failure, which is carried by [`Problems`]). Maps to a §2 exit-2 system error.
#[derive(Debug)]
pub enum LoadError {
    /// No `OSS-RELEASE.md` at the expected path.
    NotFound(PathBuf),
    /// The file exists but could not be read.
    Io(PathBuf, io::Error),
    /// The file is not valid UTF-8.
    Utf8(PathBuf),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(
                f,
                "no {CONTRACT_FILENAME} at {} (run /oss-init to generate one)",
                p.display()
            ),
            Self::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Self::Utf8(p) => write!(f, "{} is not valid UTF-8", p.display()),
        }
    }
}

/// Read `<repo_root>/OSS-RELEASE.md` through the [`Fs`] port and normalize it.
///
/// # Errors
/// Returns [`LoadError`] when the file is missing, unreadable, or not UTF-8. A
/// *validation* failure is not an error here — it is carried in the returned
/// [`Normalized::problems`]; check [`Normalized::is_valid`].
pub fn normalize(repo_root: &Path, fs: &dyn Fs) -> Result<Normalized, LoadError> {
    let path = repo_root.join(CONTRACT_FILENAME);
    let bytes = fs.read(&path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => LoadError::NotFound(path.clone()),
        _ => LoadError::Io(path.clone(), e),
    })?;
    let text = String::from_utf8(bytes).map_err(|_| LoadError::Utf8(path.clone()))?;
    Ok(normalize_str(&text, repo_root, fs))
}

/// Normalize the full text of an `OSS-RELEASE.md` (frontmatter + body).
///
/// Split from [`normalize`] so tests can exercise the pipeline on a string
/// without a real file. `repo_root` and `fs` are still needed for the
/// filesystem-dependent floors (the fragment-dir path floor and its advisory
/// existence check).
#[must_use]
pub fn normalize_str(text: &str, repo_root: &Path, fs: &dyn Fs) -> Normalized {
    let mut p = Problems::default();
    let map = match split_frontmatter(text, &mut p) {
        Some(fm) => parse_frontmatter(&fm, &mut p),
        None => Mapping::new(),
    };
    let contract = build(&map, &mut p, repo_root, fs);
    Normalized {
        contract,
        problems: p,
    }
}

/// Read a required enum field, or record an error and fall back to `default`.
/// Absent → `default` silently; present-but-invalid → error + `default`
/// (matching the Python default-substitution behavior).
macro_rules! enum_field {
    ($map:expr, $key:expr, $ty:ty, $default:expr, $p:expr) => {{
        match $map.get($key) {
            None => $default,
            Some(v) => match v.as_str().and_then(<$ty>::parse) {
                Some(x) => x,
                None => {
                    $p.err(format!(
                        "{} {} invalid — must be one of {:?}",
                        $key,
                        yaml_display(v),
                        <$ty>::VALID
                    ));
                    $default
                }
            },
        }
    }};
}

#[allow(clippy::too_many_lines)]
fn build(map: &Mapping, p: &mut Problems, repo_root: &Path, fs: &dyn Fs) -> Contract {
    // schema_version — bound first; a too-new config is a hard stop.
    let schema_version = match map.get("schema_version") {
        None => KNOWN_SCHEMA_VERSION,
        Some(v) => match v.as_i64() {
            Some(n) if n > i64::from(KNOWN_SCHEMA_VERSION) => {
                p.err(format!(
                    "schema_version {n} exceeds what this tool knows ({KNOWN_SCHEMA_VERSION}); \
                     upgrade the OSS-release skills before reading this config (refusing rather \
                     than guessing)."
                ));
                u32::try_from(n).unwrap_or(KNOWN_SCHEMA_VERSION)
            }
            Some(n) if n < 1 => {
                p.err(format!("schema_version {n} is invalid (must be >= 1)"));
                KNOWN_SCHEMA_VERSION
            }
            Some(n) => u32::try_from(n).unwrap_or(KNOWN_SCHEMA_VERSION),
            None => {
                p.err(format!(
                    "schema_version must be an integer, got {}",
                    yaml_display(v)
                ));
                KNOWN_SCHEMA_VERSION
            }
        },
    };

    let status = enum_field!(map, "status", Status, Status::Draft, p);

    // maturity — required (inference is /oss-init's job, not the normalizer's).
    let maturity = match map.get("maturity") {
        None => {
            p.err("maturity is required (spike|mvp|production) — /oss-init infers it".to_string());
            Maturity::Mvp
        }
        Some(v) => {
            if let Some(m) = v.as_str().and_then(Maturity::parse) {
                m
            } else {
                p.err(format!(
                    "maturity {} invalid — must be one of {:?}",
                    yaml_display(v),
                    Maturity::VALID
                ));
                Maturity::Mvp
            }
        }
    };

    // ecosystems — validate, then de-dup into canonical order.
    let mut parsed_ecos: Vec<Ecosystem> = Vec::new();
    for item in as_list(map.get("ecosystems")) {
        match item.as_str().and_then(Ecosystem::parse) {
            Some(e) => parsed_ecos.push(e),
            None => p.err(format!(
                "ecosystems: {} invalid — must be one of {:?}",
                yaml_display(&item),
                Ecosystem::VALID
            )),
        }
    }
    let ecosystems: Vec<Ecosystem> = ECOSYSTEM_ORDER
        .into_iter()
        .filter(|e| parsed_ecos.contains(e))
        .collect();

    // versioning — split the base enum from the calver pattern.
    let (versioning, versioning_pattern) = parse_versioning(map.get("versioning"), p);

    // release (model + layout).
    let (model, layout) = match map.get("release") {
        None | Some(Value::Null) => (ReleaseModel::Gated, ReleaseLayout::Single),
        Some(Value::Mapping(m)) => (
            enum_field!(m, "model", ReleaseModel, ReleaseModel::Gated, p),
            enum_field!(m, "layout", ReleaseLayout, ReleaseLayout::Single, p),
        ),
        Some(_) => {
            p.err("release must be a mapping with model/layout".to_string());
            (ReleaseModel::Gated, ReleaseLayout::Single)
        }
    };

    // targets — expand from ecosystems when the key is OMITTED; but an explicit
    // empty list is the author's authoritative "never publish anywhere" and is
    // honored as-is (not re-expanded). Distinguishing *absent* from *explicit
    // empty* is the whole point: a version-tracked/changelogged repo with no
    // registry publish (a private service deployed by its own script) must be
    // expressible. An empty target set is a valid, honored state — every floor
    // and downstream consumer already treats "no targets" gracefully (no
    // registry-license floor, no `registry` health badge, "nothing to publish"
    // in the release engine).
    let targets = match map.get("targets") {
        None | Some(Value::Null) => expand_targets(&ecosystems, layout),
        Some(Value::Sequence(seq)) if seq.is_empty() => Vec::new(),
        Some(Value::Sequence(seq)) => validate_targets(seq, &ecosystems, layout, p),
        Some(_) => {
            p.err(
                "targets must be a list of {ecosystem, package?, registry, adapter?} maps"
                    .to_string(),
            );
            Vec::new()
        }
    };

    // distribution — the binary-distribution block (cargo-dist/goreleaser); a
    // registry-only repo omits it (→ None), leaving its contract shape unchanged.
    // A `homebrew`-registry target is the OTHER consumer of `homebrew_tap` (the
    // release engine's homebrew-tap adapter pushes the formula in its `dist`
    // phase), so it is passed in to suppress the dead-config warning.
    let has_homebrew_target = targets.iter().any(|t| t.registry == Registry::Homebrew);
    let distribution = parse_distribution(
        map.get("distribution"),
        has_homebrew_target,
        schema_version,
        p,
    );

    // changelog (mode + source + fragment_dir).
    let changelog = match map.get("changelog") {
        None | Some(Value::Null) => Changelog {
            mode: ChangelogMode::Curated,
            source: ChangelogSource::Manual,
            fragment_dir: DEFAULT_FRAGMENT_DIR.to_string(),
        },
        Some(Value::Mapping(m)) => {
            let mode = enum_field!(m, "mode", ChangelogMode, ChangelogMode::Curated, p);
            let source = enum_field!(m, "source", ChangelogSource, ChangelogSource::Manual, p);
            let fragment_dir = match m.get("fragment_dir") {
                None => DEFAULT_FRAGMENT_DIR.to_string(),
                Some(v) => {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        p.err("changelog.fragment_dir must be a string path".to_string());
                        DEFAULT_FRAGMENT_DIR.to_string()
                    }
                }
            };
            Changelog {
                mode,
                source,
                fragment_dir,
            }
        }
        Some(_) => {
            p.err("changelog must be a mapping with mode/source".to_string());
            Changelog {
                mode: ChangelogMode::Curated,
                source: ChangelogSource::Manual,
                fragment_dir: DEFAULT_FRAGMENT_DIR.to_string(),
            }
        }
    };
    // fragment_dir must be a relative path inside the repo (floor 6).
    if !path_inside_repo(&changelog.fragment_dir) {
        p.err(format!(
            "floor: changelog.fragment_dir '{}' must be a relative path inside the repo (an \
             absolute or '../'-escaping path is refused)",
            changelog.fragment_dir
        ));
    }

    // conventional_commits.
    let conventional_commits = match map.get("conventional_commits") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(v) => {
            p.err(format!(
                "conventional_commits must be true|false, got {}",
                yaml_display(v)
            ));
            false
        }
    };

    let contribution_provenance = enum_field!(
        map,
        "contribution_provenance",
        ContributionProvenance,
        ContributionProvenance::None,
        p
    );
    let provenance_level = enum_field!(
        map,
        "provenance_level",
        ProvenanceLevel,
        ProvenanceLevel::None,
        p
    );

    let dep_default = if maturity == Maturity::Spike {
        DependencyBot::None
    } else {
        DependencyBot::Dependabot
    };
    let dependency_bot = enum_field!(map, "dependency_bot", DependencyBot, dep_default, p);

    // license — a valid SPDX expression when set (default MIT).
    let license = match map.get("license") {
        None => "MIT".to_string(),
        Some(v) => match v.as_str() {
            Some(s) if !s.trim().is_empty() => {
                if !spdx_valid(s) {
                    p.err(format!(
                        "license '{s}' is not a valid SPDX expression (unknown id or malformed \
                         AND/OR/WITH grammar)"
                    ));
                }
                s.to_string()
            }
            _ => {
                p.err("license must be a non-empty SPDX id/expression (default MIT)".to_string());
                "MIT".to_string()
            }
        },
    };

    let docs_site = enum_field!(map, "docs_site", DocsSite, DocsSite::None, p);

    // health_badges — validate when present (key-presence, per Python), else
    // materialize a floor-clean default (maturity/target aware).
    let health_badges = if map.contains_key("health_badges") {
        let mut out = Vec::new();
        for item in as_list(map.get("health_badges")) {
            match item.as_str().and_then(HealthBadge::parse) {
                Some(hb) => out.push(hb),
                None => p.err(format!(
                    "health_badges: {} invalid — must be one of {:?}",
                    yaml_display(&item),
                    HealthBadge::VALID
                )),
            }
        }
        out
    } else {
        default_health_badges(maturity, &targets)
    };

    // ── Cross-field floors (§2) — config-internal, ALWAYS hard errors ────────
    if model == ReleaseModel::Auto && maturity == Maturity::Spike {
        p.err(
            "floor: release.model 'auto' is not allowed on maturity 'spike' — a spike is not \
             being published; raise maturity or set release.model: gated"
                .to_string(),
        );
    }
    if provenance_level == ProvenanceLevel::SlsaL3 && maturity != Maturity::Production {
        p.err(format!(
            "floor: provenance_level 'slsa-l3' is production-only — current maturity is '{}'",
            maturity.as_str()
        ));
    }
    // A target with a registry requires a valid SPDX license. Every expanded
    // target carries a registry, so "any registry" reduces to "any target".
    if !targets.is_empty() && !spdx_valid(&license) {
        p.err(format!(
            "floor: a target has a registry (crates.io/npm/PyPI/… require a license) but license \
             '{license}' is not a valid SPDX expression"
        ));
    }
    check_badge_producers(&health_badges, maturity, &targets, p);
    // A distribution block ships public binaries (GH-Release artifacts, a curl-pipe
    // installer, a Homebrew tap PR) — that is publishing, and a spike is not being
    // published. Mirrors the `release.model: auto` floor: raise maturity or drop the
    // block. (Absent block → no constraint; registry-only spikes are unaffected.)
    if distribution.is_some() && maturity == Maturity::Spike {
        p.err(
            "floor: a distribution block ships public binaries (installer + tap) — not allowed on \
             maturity 'spike' (a spike is not being published); raise maturity or drop distribution"
                .to_string(),
        );
    }

    // ── Filesystem/producer-existence semantic check — ADVISORY, never fatal ─
    if changelog.mode == ChangelogMode::Fragment
        && path_inside_repo(&changelog.fragment_dir)
        && !fs.is_dir(&repo_root.join(&changelog.fragment_dir))
    {
        p.warn(format!(
            "changelog.mode 'fragment' but the fragment dir '{}' does not exist yet under {} — \
             /oss-changelog creates it; /oss-readiness reports it as a gap until then",
            changelog.fragment_dir,
            repo_root.display()
        ));
    }

    // ── Forward-compat: preserve unknown fields, report once ─────────────────
    let extra_fields =
        capture_unknown_fields(map, KNOWN_KEYS, CaptureScope::TopLevel, schema_version, p);

    let warnings = p.warnings.clone();
    Contract {
        schema_version,
        status,
        maturity,
        ecosystems,
        targets,
        distribution,
        versioning,
        versioning_pattern,
        changelog,
        conventional_commits,
        release: Release { model, layout },
        contribution_provenance,
        provenance_level,
        dependency_bot,
        health_badges,
        license,
        docs_site,
        extra_fields,
        warnings,
    }
}

fn parse_versioning(value: Option<&Value>, p: &mut Problems) -> (VersioningBase, Option<String>) {
    let Some(v) = value else {
        return (VersioningBase::Semver, None);
    };
    let Some(s) = v.as_str() else {
        p.err(format!(
            "versioning {} invalid — must be semver | calver:<pattern> | zerover",
            yaml_display(v)
        ));
        return (VersioningBase::Semver, None);
    };
    if let Some(rest) = s.strip_prefix("calver:") {
        let pattern = rest.trim();
        if pattern.is_empty() {
            p.err(
                "versioning 'calver:' carries no pattern — e.g. calver:YYYY.MM.MICRO".to_string(),
            );
        }
        (VersioningBase::Calver, Some(pattern.to_string()))
    } else if s == "calver" {
        p.err(
            "versioning 'calver' must carry its pattern (calver:YYYY.MM.MICRO), not a bare label"
                .to_string(),
        );
        (VersioningBase::Calver, None)
    } else if let Some(base) = VersioningBase::parse(s) {
        (base, None)
    } else {
        p.err(format!(
            "versioning '{s}' invalid — must be semver | calver:<pattern> | zerover"
        ));
        (VersioningBase::Semver, None)
    }
}

/// Derive one target per ecosystem with default registry + adapter.
fn expand_targets(ecosystems: &[Ecosystem], layout: ReleaseLayout) -> Vec<Target> {
    ecosystems
        .iter()
        .map(|&e| Target {
            ecosystem: e,
            package: None,
            registry: e.default_registry(),
            adapter: e.default_adapter(layout),
        })
        .collect()
}

fn validate_targets(
    seq: &[Value],
    ecosystems: &[Ecosystem],
    layout: ReleaseLayout,
    p: &mut Problems,
) -> Vec<Target> {
    let mut out = Vec::new();
    for (idx, item) in seq.iter().enumerate() {
        let Value::Mapping(m) = item else {
            p.err(format!(
                "targets[{idx}] must be a map with at least {{ecosystem, registry}}"
            ));
            continue;
        };

        let ecosystem = if let Some(s) = m.get("ecosystem").and_then(Value::as_str) {
            if let Some(e) = Ecosystem::parse(s) {
                if !ecosystems.is_empty() && !ecosystems.contains(&e) {
                    p.err(format!(
                        "targets[{idx}].ecosystem '{s}' is not in ecosystems {:?}",
                        ecosystems.iter().map(|e| e.as_str()).collect::<Vec<_>>()
                    ));
                }
                Some(e)
            } else {
                p.err(format!(
                    "targets[{idx}].ecosystem '{s}' invalid — one of {:?}",
                    Ecosystem::VALID
                ));
                None
            }
        } else {
            p.err(format!(
                "targets[{idx}].ecosystem invalid — one of {:?}",
                Ecosystem::VALID
            ));
            None
        };

        let registry = match m.get("registry").and_then(Value::as_str) {
            None => {
                p.err(format!(
                    "targets[{idx}] has no registry (required — the publish destination)"
                ));
                None
            }
            Some(s) => {
                if let Some(r) = Registry::parse(s) {
                    Some(r)
                } else {
                    p.err(format!(
                        "targets[{idx}].registry '{s}' invalid — one of {:?}",
                        Registry::VALID
                    ));
                    None
                }
            }
        };

        let adapter = match m.get("adapter") {
            None => ecosystem.map(|e| e.default_adapter(layout)),
            Some(v) => {
                if let Some(a) = v.as_str().and_then(Adapter::parse) {
                    Some(a)
                } else {
                    p.err(format!(
                        "targets[{idx}].adapter {} invalid — one of {:?}",
                        yaml_display(v),
                        Adapter::VALID
                    ));
                    None
                }
            }
        };

        // On the error path, placeholders keep the strong type; the document is
        // never emitted when problems.errors is non-empty.
        out.push(Target {
            ecosystem: ecosystem.unwrap_or(Ecosystem::Binary),
            package: m.get("package").and_then(Value::as_str).map(str::to_string),
            registry: registry.unwrap_or(Registry::GhReleases),
            adapter: adapter.unwrap_or(Adapter::Manual),
        });
    }
    out
}

/// Known `distribution`-block keys; anything else is preserved under
/// [`Distribution::extra_fields`] (forward-compat), the nested analogue of
/// [`KNOWN_KEYS`].
///
/// **Invariant:** this list MUST stay in sync with the [`Distribution`] struct
/// fields (see the [`KNOWN_KEYS`] note). The trailing `extra_fields` entry is the
/// reserved canonical-output metadata key — its contents are merged back rather
/// than nested (see [`capture_unknown_fields`]). A [`Distribution`] carries no
/// `warnings` (those live only at the top level), so only `extra_fields` needs
/// reserving here.
const KNOWN_DISTRIBUTION_KEYS: &[&str] = &[
    "adapter",
    "gh_releases",
    "installers",
    "homebrew_tap",
    "platforms",
    // Reserved canonical-output metadata key (not parsed) — see doc above.
    "extra_fields",
];

/// Canonical installer order — used to de-duplicate and stably order the
/// `distribution.installers` list (mirrors [`ECOSYSTEM_ORDER`]'s role).
const INSTALLER_ORDER: [Installer; 5] = [
    Installer::Shell,
    Installer::Powershell,
    Installer::Homebrew,
    Installer::Msi,
    Installer::Npm,
];

/// Parse the optional `distribution` block (the cargo-dist/goreleaser binary
/// layer). Absent/null → `None`, leaving a registry-only contract unchanged. A
/// present-but-non-mapping value is an error (with `None` on the error path — the
/// document is never emitted while `problems.errors` is non-empty).
#[allow(clippy::too_many_lines)]
fn parse_distribution(
    value: Option<&Value>,
    has_homebrew_target: bool,
    schema_version: u32,
    p: &mut Problems,
) -> Option<Distribution> {
    let m = match value {
        None | Some(Value::Null) => return None,
        Some(Value::Mapping(m)) => m,
        Some(_) => {
            p.err(
                "distribution must be a mapping with {adapter?, gh_releases?, installers?, \
                 homebrew_tap?, platforms?}"
                    .to_string(),
            );
            return None;
        }
    };

    // adapter — required when the block is present. Which tool OWNS the existing
    // tag-triggered release workflow is not the normalizer's to guess (it renames
    // release semantics and picks a Rust-specific default); inference is
    // /oss-init's job, exactly as for `maturity`. A bare `distribution: {}` is
    // therefore an error, not a silent "cargo-dist owns this repo".
    let adapter = match m.get("adapter") {
        None => {
            p.err(
                "distribution.adapter is required when a distribution block is present \
                 (cargo-dist|goreleaser|manual) — /oss-init infers it"
                    .to_string(),
            );
            DistributionAdapter::CargoDist
        }
        Some(v) => {
            if let Some(a) = v.as_str().and_then(DistributionAdapter::parse) {
                a
            } else {
                p.err(format!(
                    "distribution.adapter {} invalid — must be one of {:?}",
                    yaml_display(v),
                    DistributionAdapter::VALID
                ));
                DistributionAdapter::CargoDist
            }
        }
    };

    let gh_releases = match m.get("gh_releases") {
        // cargo-dist/goreleaser attach per-platform binaries by default.
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(v) => {
            p.err(format!(
                "distribution.gh_releases must be true|false, got {}",
                yaml_display(v)
            ));
            true
        }
    };

    // installers — validate each, then de-dup into canonical order.
    let mut parsed_installers: Vec<Installer> = Vec::new();
    for item in as_list(m.get("installers")) {
        match item.as_str().and_then(Installer::parse) {
            Some(i) => parsed_installers.push(i),
            None => p.err(format!(
                "distribution.installers: {} invalid — must be one of {:?}",
                yaml_display(&item),
                Installer::VALID
            )),
        }
    }
    let installers: Vec<Installer> = INSTALLER_ORDER
        .into_iter()
        .filter(|i| parsed_installers.contains(i))
        .collect();

    let homebrew_tap = match m.get("homebrew_tap") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_str() {
            Some(s) if is_tap_slug(s) => Some(s.to_string()),
            // An invalid slug substitutes `None` (not the bad value) so the
            // built `Distribution` never carries a malformed tap — matching the
            // "placeholders keep the strong type" error-path rule the rest of the
            // normalizer follows, and letting the homebrew-needs-tap floor below
            // still fire (a present-but-invalid tap is no tap).
            Some(s) => {
                p.err(format!(
                    "distribution.homebrew_tap '{s}' invalid — must be an 'owner/repo' slug"
                ));
                None
            }
            None => {
                p.err("distribution.homebrew_tap must be an 'owner/repo' string".to_string());
                None
            }
        },
    };

    let wants_homebrew = installers.contains(&Installer::Homebrew);
    // Floor: a `homebrew` installer needs a tap to push the generated formula to.
    if wants_homebrew && homebrew_tap.is_none() {
        p.err(
            "floor: distribution.installers includes 'homebrew' but no distribution.homebrew_tap \
             is set — the generated formula has nowhere to be pushed"
                .to_string(),
        );
    }
    // Advisory: a tap with NEITHER a `homebrew` installer NOR a `homebrew`-registry
    // target is dead config — no formula is ever generated, so the tap is never
    // pushed to. A warning, not a floor: the contract is internally consistent,
    // just wasteful. The tap has TWO possible consumers: cargo-dist's `homebrew`
    // installer, and the release engine's homebrew-tap adapter target (whose `dist`
    // phase generates + pushes the formula). Either one means the tap IS updated —
    // so the warning fires only when both are absent.
    if homebrew_tap.is_some() && !wants_homebrew && !has_homebrew_target {
        p.warn(
            "distribution.homebrew_tap is set but there is neither a 'homebrew' installer in \
             distribution.installers nor a 'homebrew'-registry target — no formula is generated, \
             so the tap will never be updated"
                .to_string(),
        );
    }

    // platforms — the binary target-triple set. Omitted/null → the cross-platform
    // default (macOS + Linux musl), so a distribution that doesn't specify
    // platforms covers Linux by default (the cross-platform install requirement).
    // An explicit list is validated per triple and de-duplicated, preserving the
    // author's order (like the sibling `targets` list — there is no canonical
    // triple ordering to impose). An explicit *empty* list is NOT the same as
    // omitted: it is a mistake, and silently defaulting it would surprise the
    // author with targets they never listed and erase the intent the downstream
    // cross-platform audit needs — so it is a hard error.
    let platforms = match m.get("platforms") {
        None | Some(Value::Null) => default_cross_platform_targets(),
        Some(Value::Sequence(seq)) if seq.is_empty() => {
            // Default fallback keeps error-collection going; the contract is never
            // emitted while `problems.errors` is non-empty.
            p.err(
                "distribution.platforms is an empty list — omit the key to accept the \
                 cross-platform default (macOS + Linux) or list explicit target-triples; a \
                 distribution with no platforms builds nothing"
                    .to_string(),
            );
            default_cross_platform_targets()
        }
        Some(Value::Sequence(seq)) => {
            let mut out: Vec<String> = Vec::new();
            for item in seq {
                match item.as_str() {
                    Some(s) if looks_like_target_triple(s) => {
                        let triple = s.to_string();
                        if !out.contains(&triple) {
                            out.push(triple);
                        }
                    }
                    Some(s) => p.err(format!(
                        "distribution.platforms: '{s}' is not a well-formed target-triple \
                         (e.g. x86_64-unknown-linux-musl, aarch64-apple-darwin) — structural \
                         check only; the toolchain is the final authority on what builds"
                    )),
                    None => p.err(format!(
                        "distribution.platforms: {} invalid — each entry must be a \
                         target-triple string",
                        yaml_display(item)
                    )),
                }
            }
            out
        }
        Some(v) => {
            p.err(format!(
                "distribution.platforms must be a list of target-triple strings, got {}",
                yaml_display(v)
            ));
            default_cross_platform_targets()
        }
    };

    // Cross-check: an OS-specific installer whose target OS is absent from the
    // resolved `platforms` set is dead config — the generated installer points at
    // a binary the release never builds ("the installer has nothing to install").
    // A warning, not a floor (mirrors the `homebrew_tap`-without-consumer advisory
    // above): the contract is internally consistent, just wasteful. Only the
    // OS-specific installers constrain the set — see [`installer_os_need`] for the
    // full installer→OS table; npm/shell/powershell are not cross-checked.
    //
    // Gated on a clean parse: this is a cross-field semantic advisory, so it must
    // read only well-formed triples. A malformed triple (rejected above) that
    // happens to contain an OS keyword must neither satisfy nor spuriously fail
    // the coverage check — otherwise the warning would flip as the author fixes an
    // unrelated error. Errors already block emission, so gating here loses nothing.
    if p.errors.is_empty() {
        let has_windows = platforms.iter().any(|t| is_windows_triple(t));
        let has_macos = platforms.iter().any(|t| is_macos_triple(t));
        let has_linux = platforms.iter().any(|t| is_linux_triple(t));
        for &installer in &installers {
            let unmet = match installer_os_need(installer) {
                OsNeed::Unchecked => None,
                OsNeed::Windows => (!has_windows).then_some(
                    "distribution.installers includes 'msi' but the resolved \
                     distribution.platforms set has no Windows (*-windows-*) target — the MSI \
                     installer has nothing to install",
                ),
                // Homebrew serves macOS natively AND Linux via Linuxbrew, so a
                // single Linux triple satisfies it just as a darwin triple does;
                // the warning fires only when NEITHER is present (the issue's stated
                // intent when the darwin-vs-linux question is ambiguous).
                OsNeed::MacosOrLinux => (!has_macos && !has_linux).then_some(
                    "distribution.installers includes 'homebrew' but the resolved \
                     distribution.platforms set has no macOS (*-apple-darwin) or Linux \
                     (*-linux-*) target — the Homebrew formula has nothing to install",
                ),
            };
            if let Some(msg) = unmet {
                p.warn(msg.to_string());
            }
        }
    }

    // Forward-compat: preserve unknown distribution sub-keys (the nested analogue
    // of the top-level `extra_fields` scan), so an older reader round-trips a
    // newer contract's distribution keys rather than dropping them. Reported once,
    // scoped to the block via the `Distribution` scope, mirroring the top-level
    // unknown-field warning — the shared helper keeps the two from drifting.
    let extra_fields = capture_unknown_fields(
        m,
        KNOWN_DISTRIBUTION_KEYS,
        CaptureScope::Distribution,
        schema_version,
        p,
    );

    Some(Distribution {
        adapter,
        gh_releases,
        installers,
        homebrew_tap,
        platforms,
        extra_fields,
    })
}

/// The cross-platform default `distribution.platforms` set as owned strings —
/// materialized when the block omits `platforms` (or gives an empty list). Always
/// contains at least one Linux triple (the cross-platform install requirement).
fn default_cross_platform_targets() -> Vec<String> {
    DEFAULT_CROSS_PLATFORM_TARGETS
        .iter()
        .map(|&s| s.to_string())
        .collect()
}

/// The OS coverage an installer needs from `distribution.platforms` to install
/// anything — the small installer→OS spec behind the installer↔platform
/// cross-check warning. Kept as one table (see [`installer_os_need`]) rather than
/// scattered conditionals so the mapping stays inspectable in one place.
enum OsNeed {
    /// Not cross-checked — this installer never constrains `platforms`.
    Unchecked,
    /// Needs at least one Windows triple.
    Windows,
    /// Needs at least one macOS OR Linux triple.
    MacosOrLinux,
}

/// The OS an installer's generated artifact can actually install onto — the spec
/// that lets the normalizer flag an installer whose target OS is absent from
/// `platforms`. Only `msi` and `homebrew` are OS-gated; the rest are deliberately
/// left `Unchecked` (a scoping choice, not a claim that they run everywhere):
///
/// | installer    | need              | rationale                                          |
/// |--------------|-------------------|----------------------------------------------------|
/// | `msi`        | Windows           | an `.msi` installs only on Windows                 |
/// | `homebrew`   | macOS **or** Linux| Homebrew serves macOS natively and Linux (Linuxbrew) |
/// | `shell`      | — (not checked)   | a POSIX script; only msi/homebrew are gated for now |
/// | `powershell` | — (not checked)   | Windows-oriented; only msi/homebrew are gated for now |
/// | `npm`        | — (not checked)   | published to a registry, not tied to one OS's artifact |
fn installer_os_need(i: Installer) -> OsNeed {
    match i {
        Installer::Msi => OsNeed::Windows,
        Installer::Homebrew => OsNeed::MacosOrLinux,
        Installer::Shell | Installer::Powershell | Installer::Npm => OsNeed::Unchecked,
    }
}

/// The OS ("system") component of a target-triple — the 3rd `-`-separated field
/// in the `<arch>-<vendor>-<os>[-<env>]` shape the shipped desktop triples use
/// (`x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`).
/// `None` for a 2-component triple that names no vendor (`wasm32-wasip1`). Matching
/// the OS *positionally* (rather than "any component equals …") is what keeps
/// `aarch64-linux-android` out of the Linux bucket: its `linux` sits in the vendor
/// slot and the real OS component is `android`.
fn triple_os(s: &str) -> Option<&str> {
    s.split('-').nth(2)
}

/// Whether a target-triple targets Windows — OS component `windows` (covering
/// both `-windows-msvc` and `-windows-gnu`).
fn is_windows_triple(s: &str) -> bool {
    triple_os(s) == Some("windows")
}

/// Whether a target-triple targets macOS — OS component `darwin` (e.g.
/// `aarch64-apple-darwin`). Apple's non-macOS triples (`*-apple-ios`, `-tvos`, …)
/// carry a different OS component and are correctly excluded.
fn is_macos_triple(s: &str) -> bool {
    triple_os(s) == Some("darwin")
}

/// Whether a target-triple targets Linux — OS component `linux` (e.g.
/// `x86_64-unknown-linux-musl`), covering the Linuxbrew case for `homebrew`.
/// Android (`aarch64-linux-android`) has `android` as its OS component and does
/// not count.
fn is_linux_triple(s: &str) -> bool {
    triple_os(s) == Some("linux")
}

/// Whether `s` is a *structurally* plausible target-triple — 2–4 `-`-separated
/// components, each a non-empty run of `[a-z0-9_.]`. Deliberately LEXICAL, not
/// semantic: the real triple set is open and rustc-defined, so this is a
/// well-formedness gate, not a whitelist. It rejects what could never be a triple
/// (empty parts, uppercase, whitespace, punctuation, injection chars, wrong shape)
/// and accepts real triples including dotted arch names like
/// `thumbv8m.main-none-eabi` — but it also accepts structurally-valid nonsense like
/// `aa-bb`, because the toolchain is the final authority on whether a triple
/// actually builds. The OS component stays intact and inspectable so the
/// cross-platform `audit` can classify a set downstream.
fn looks_like_target_triple(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    (2..=4).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.')
                })
        })
}

/// Whether `s` is a plausible `owner/repo` tap slug — exactly one `/`, and each
/// part a non-empty run of the GitHub-name character set (ASCII alphanumeric plus
/// `-`, `_`, `.`), with `.`/`..` rejected. Lexical only — existence is not
/// checked. Deliberately strict: this value flows into `brew tap` and repo URLs
/// downstream, so arbitrary punctuation, whitespace, or path traversal
/// (`owner/..`) must not pass.
fn is_tap_slug(s: &str) -> bool {
    fn valid_part(part: &str) -> bool {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }
    match s.split_once('/') {
        Some((owner, repo)) => valid_part(owner) && valid_part(repo) && !repo.contains('/'),
        None => false,
    }
}

/// A floor-clean default badge set: `ci` at mvp+, `registry` when a publishable
/// target exists, `license` always.
fn default_health_badges(maturity: Maturity, targets: &[Target]) -> Vec<HealthBadge> {
    let mut badges = Vec::new();
    if matches!(maturity, Maturity::Mvp | Maturity::Production) {
        badges.push(HealthBadge::Ci);
    }
    if !targets.is_empty() {
        badges.push(HealthBadge::Registry);
    }
    badges.push(HealthBadge::License);
    badges
}

/// Every enabled badge must have its producer enabled (floor 4).
fn check_badge_producers(
    badges: &[HealthBadge],
    maturity: Maturity,
    targets: &[Target],
    p: &mut Problems,
) {
    let has_registry_target = !targets.is_empty();
    for b in badges {
        match b {
            HealthBadge::Ci if maturity == Maturity::Spike => p.err(
                "floor: health_badge 'ci' has no producer at maturity 'spike' (no CI until mvp) — \
                 drop it or raise maturity"
                    .to_string(),
            ),
            HealthBadge::Registry if !has_registry_target => p.err(
                "floor: health_badge 'registry' has no producer — no target has a registry to \
                 publish to"
                    .to_string(),
            ),
            HealthBadge::Coverage if maturity != Maturity::Production => p.err(format!(
                "floor: health_badge 'coverage' has no producer — the coverage gate is a \
                 production-tier /oss-ci output; current maturity is '{}'",
                maturity.as_str()
            )),
            HealthBadge::Scorecard if maturity != Maturity::Production => p.err(format!(
                "floor: health_badge 'scorecard' has no producer — the Scorecard action is a \
                 production-tier output; current maturity is '{}'",
                maturity.as_str()
            )),
            _ => {}
        }
    }
}

// ── Frontmatter extraction + parse ───────────────────────────────────────────

/// A `---` fence line (exactly three dashes plus optional trailing whitespace).
fn is_fence(line: &str) -> bool {
    let t = line.trim_end();
    t == "---" || (t.starts_with("---") && t[3..].chars().all(char::is_whitespace))
}

/// Split the YAML frontmatter block out of the document. Returns the frontmatter
/// text (body discarded — the normalizer never reads it), or `None` on a
/// structural error (recorded on `p`).
fn split_frontmatter(text: &str, p: &mut Problems) -> Option<String> {
    let mut lines = text.lines();
    match lines.next() {
        Some(first) if is_fence(first) => {}
        _ => {
            p.err("frontmatter missing: file must begin with a '---' YAML block".to_string());
            return None;
        }
    }
    let mut fm = String::new();
    for line in lines {
        if is_fence(line) {
            return Some(fm);
        }
        fm.push_str(line);
        fm.push('\n');
    }
    p.err("frontmatter not closed: no terminating '---' line found".to_string());
    None
}

/// Parse the frontmatter into a YAML mapping. `serde_yaml` rejects duplicate
/// keys natively; a non-mapping top level or any YAML error is recorded on `p`.
fn parse_frontmatter(fm: &str, p: &mut Problems) -> Mapping {
    if fm.trim().is_empty() {
        return Mapping::new();
    }
    match serde_yaml::from_str::<Value>(fm) {
        Ok(Value::Null) => Mapping::new(),
        Ok(Value::Mapping(m)) => m,
        Ok(_) => {
            p.err("frontmatter: top level must be a mapping".to_string());
            Mapping::new()
        }
        Err(e) => {
            p.err(format!("frontmatter: invalid YAML — {e}"));
            Mapping::new()
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Coerce a value to a list: a sequence stays; absent/null → empty; a scalar
/// becomes a one-element list (mirrors the Python `_as_list`).
fn as_list(v: Option<&Value>) -> Vec<Value> {
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(seq)) => seq.clone(),
        Some(other) => vec![other.clone()],
    }
}

/// A compact display of a YAML scalar for error messages (strings are quoted).
fn yaml_display(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
        Value::Sequence(_) => "<list>".to_string(),
        Value::Mapping(_) => "<map>".to_string(),
        Value::Tagged(t) => yaml_display(&t.value),
    }
}

/// Whether `rel` is a relative path that stays inside the repo — no absolute
/// path, no `../` escape — the fragment-dir floor. Lexical, so the path need not
/// exist. The check is purely on `rel`'s own component depth, so it holds
/// whether the repo root is absolute or relative (notably `--repo-root .`,
/// where `repo_root` normalizes to an empty path): a `..` is an escape the
/// moment it would pop above the repo root, exactly the Python
/// `_path_inside_repo` verdict (which rejects any `rel` that normalizes to an
/// escaping path). Joining `rel` onto a relative root and testing containment —
/// the previous approach — silently accepted `../etc` under a `.` root, because
/// an empty normalized root is a prefix of every path.
fn path_inside_repo(rel: &str) -> bool {
    let mut depth: usize = 0;
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                // An escape above the repo root the instant depth would go < 0.
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            // An absolute path (or a Windows drive prefix) never stays inside a
            // relative repo root.
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

/// Which mapping the [`capture_unknown_fields`] scan is running over — scopes the
/// forward-compat warning text and error messages. An enum (rather than a bare
/// string prefix) so a new call site cannot silently pass a mis-spaced label and
/// produce `unknown distributionfield(s)`.
#[derive(Clone, Copy)]
enum CaptureScope {
    /// The top-level frontmatter mapping ([`KNOWN_KEYS`]).
    TopLevel,
    /// The nested `distribution` block ([`KNOWN_DISTRIBUTION_KEYS`]).
    Distribution,
}

impl CaptureScope {
    /// The infix woven into the warning/error text — `""` for the top level,
    /// `"distribution "` for the block — so a message reads `unknown field(s) …`
    /// vs `unknown distribution field(s) …`.
    fn label(self) -> &'static str {
        match self {
            Self::TopLevel => "",
            Self::Distribution => "distribution ",
        }
    }
}

/// Scan a YAML mapping for keys outside `known` and preserve them under a
/// forward-compat `extra_fields` map, warning once when any were captured. The
/// single implementation behind BOTH the top-level ([`KNOWN_KEYS`]) and nested
/// `distribution` ([`KNOWN_DISTRIBUTION_KEYS`]) scans, so the two cannot drift
/// (the nested warning once silently omitted `schema_version`).
///
/// The guarantee is that an unknown **string** key is never dropped, never
/// double-captured, and round-trips predictably:
/// - A string key not in `known` is captured verbatim.
/// - A known string key is skipped (parsed as its field, not double-captured).
/// - The reserved canonical-output key `extra_fields` (in `known`) is not
///   re-captured into a nested `extra_fields.extra_fields`; instead its mapping
///   contents are **merged back** into the returned map (see
///   [`merge_reserved_extra_fields`]) so a hand-authored — or, defensively, a
///   re-fed canonical — `extra_fields` block round-trips losslessly rather than
///   being silently dropped. A key present both in that block and as a sibling
///   unknown field is an ambiguity error, not a silent overwrite.
///
/// Non-string keys (`42:`, `true:`, a list/map key — legal YAML) are a **structural
/// error**, not silently coerced: they can never be a forward-compatible schema
/// field (canonical JSON object keys are strings), and coercing them through the
/// display formatter would collapse distinct keys onto the same string (`42` and
/// `"42"`; every list key onto `<list>`) and silently drop a value — the opposite
/// of the never-drop intent. Rejecting keeps the invariant vacuously (an invalid
/// contract's output is never consumed) and matches the normalizer's
/// error-collection style.
fn capture_unknown_fields(
    m: &Mapping,
    known: &[&str],
    scope: CaptureScope,
    schema_version: u32,
    p: &mut Problems,
) -> serde_json::Map<String, serde_json::Value> {
    let label = scope.label();
    let mut extra_fields = serde_json::Map::new();
    // Merge an explicit `extra_fields` block first (reserved metadata key), so a
    // sibling unknown key colliding with it is detected below rather than
    // silently overwriting it.
    if let Some(v) = m.get("extra_fields") {
        merge_reserved_extra_fields(v, scope, &mut extra_fields, p);
    }
    for (k, v) in m {
        match k {
            Value::String(key) => {
                if known.contains(&key.as_str()) {
                    continue;
                }
                if extra_fields.contains_key(key) {
                    p.err(format!(
                        "{label}field '{key}' appears both as an unknown top-level key and inside \
                         the reserved '{label}extra_fields' block — refusing to drop either value; \
                         remove one"
                    ));
                } else {
                    extra_fields.insert(key.clone(), yaml_to_json(v));
                }
            }
            other => p.err(format!(
                "{label}field key {} must be a string — a non-string key is not a \
                 forward-compatible schema shape and cannot be preserved losslessly (distinct \
                 non-string keys collapse onto the same JSON key)",
                yaml_display(other)
            )),
        }
    }
    if !extra_fields.is_empty() {
        // serde_json::Map is ordered (BTreeMap, no `preserve_order`) → keys already
        // sorted. Rendered as a single-quoted list to match the Python normalizer.
        let keys = extra_fields
            .keys()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ");
        p.warn(format!(
            "unknown {label}field(s) preserved under schema_version {schema_version} \
             (forward-compat): [{keys}]"
        ));
    }
    extra_fields
}

/// Merge the contents of a reserved `extra_fields` block (a hand-authored, or
/// defensively a re-fed canonical, mapping under the reserved `extra_fields` key)
/// into `out`, upholding the never-drop invariant for that block rather than
/// silently discarding it now that the key is reserved in `known`. A non-mapping
/// value, or a non-string key inside it, is a structural error (same rationale as
/// the sibling scan in [`capture_unknown_fields`]). Sibling-key collisions are
/// detected back in the caller, after this has seeded `out`.
fn merge_reserved_extra_fields(
    v: &Value,
    scope: CaptureScope,
    out: &mut serde_json::Map<String, serde_json::Value>,
    p: &mut Problems,
) {
    let label = scope.label();
    match v {
        Value::Null => {}
        Value::Mapping(inner) => {
            for (k, val) in inner {
                match k {
                    Value::String(key) => {
                        out.insert(key.clone(), yaml_to_json(val));
                    }
                    other => p.err(format!(
                        "reserved '{label}extra_fields' block has a non-string key {} — its keys \
                         must be strings",
                        yaml_display(other)
                    )),
                }
            }
        }
        other => p.err(format!(
            "reserved '{label}extra_fields' must be a mapping when present, got {}",
            yaml_display(other)
        )),
    }
}

/// Convert an arbitrary YAML value to JSON, for `extra_fields` preservation.
fn yaml_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                J::from(i)
            } else if let Some(u) = n.as_u64() {
                J::from(u)
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f).map_or(J::Null, J::Number)
            } else {
                J::Null
            }
        }
        Value::String(s) => J::String(s.clone()),
        Value::Sequence(seq) => J::Array(seq.iter().map(yaml_to_json).collect()),
        Value::Mapping(m) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in m {
                let key = match k {
                    Value::String(s) => s.clone(),
                    other => yaml_display(other),
                };
                obj.insert(key, yaml_to_json(val));
            }
            J::Object(obj)
        }
        Value::Tagged(t) => yaml_to_json(&t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A fake `Fs`: `normalize_str` never `read`s, so only the directory set
    /// matters (for the fragment-dir advisory check).
    struct FakeFs {
        dirs: HashSet<PathBuf>,
    }

    impl FakeFs {
        fn empty() -> Self {
            Self {
                dirs: HashSet::new(),
            }
        }

        fn with_dirs<const N: usize>(dirs: [&str; N]) -> Self {
            Self {
                dirs: dirs.iter().map(PathBuf::from).collect(),
            }
        }
    }

    impl Fs for FakeFs {
        fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
        fn exists(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }
        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }
        fn is_file(&self, _path: &Path) -> bool {
            // The contract normalizer models only directories (fragment-dir).
            false
        }
        fn read_dir(&self, _dir: &Path) -> io::Result<Vec<String>> {
            // The contract normalizer never lists directories.
            Ok(Vec::new())
        }
    }

    fn repo() -> &'static Path {
        Path::new("/repo")
    }

    fn norm(text: &str) -> Normalized {
        normalize_str(text, repo(), &FakeFs::empty())
    }

    fn norm_with(text: &str, fs: &dyn Fs) -> Normalized {
        normalize_str(text, repo(), fs)
    }

    fn assert_error_contains(n: &Normalized, needle: &str) {
        assert!(
            !n.is_valid(),
            "expected invalid, got clean normalize: {:?}",
            n.contract
        );
        assert!(
            n.problems.errors.iter().any(|e| e.contains(needle)),
            "no error contained {needle:?}; errors were {:?}",
            n.problems.errors
        );
    }

    const MINIMAL: &str = "---\nstatus: approved\nmaturity: mvp\n---\n";

    #[test]
    fn materializes_all_defaults() {
        let c = norm(MINIMAL).contract;
        assert_eq!(c.schema_version, 1);
        assert_eq!(c.status, Status::Approved);
        assert_eq!(c.maturity, Maturity::Mvp);
        assert!(c.ecosystems.is_empty());
        assert!(c.targets.is_empty());
        assert_eq!(c.versioning, VersioningBase::Semver);
        assert_eq!(c.versioning_pattern, None);
        assert_eq!(c.changelog.mode, ChangelogMode::Curated);
        assert_eq!(c.changelog.source, ChangelogSource::Manual);
        assert_eq!(c.changelog.fragment_dir, DEFAULT_FRAGMENT_DIR);
        assert!(!c.conventional_commits);
        assert_eq!(c.release.model, ReleaseModel::Gated);
        assert_eq!(c.release.layout, ReleaseLayout::Single);
        assert_eq!(c.contribution_provenance, ContributionProvenance::None);
        assert_eq!(c.provenance_level, ProvenanceLevel::None);
        assert_eq!(c.dependency_bot, DependencyBot::Dependabot); // mvp default
        assert_eq!(c.license, "MIT");
        assert_eq!(c.docs_site, DocsSite::None);
        // mvp, no publishable target → [ci, license].
        assert_eq!(c.health_badges, vec![HealthBadge::Ci, HealthBadge::License]);
        assert!(c.extra_fields.is_empty());
    }

    #[test]
    fn spike_defaults_no_bot_no_ci_badge() {
        let c = norm("---\nstatus: approved\nmaturity: spike\n---\n").contract;
        assert_eq!(c.dependency_bot, DependencyBot::None);
        assert_eq!(c.health_badges, vec![HealthBadge::License]);
    }

    #[test]
    fn maturity_is_required() {
        assert_error_contains(
            &norm("---\nstatus: approved\n---\n"),
            "maturity is required",
        );
    }

    #[test]
    fn expands_targets_from_ecosystems() {
        let c = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [python]\n---\n").contract;
        assert_eq!(c.targets.len(), 1);
        assert_eq!(c.targets[0].ecosystem, Ecosystem::Python);
        assert_eq!(c.targets[0].package, None);
        assert_eq!(c.targets[0].registry, Registry::Pypi);
        assert_eq!(c.targets[0].adapter, Adapter::GhActionPypiPublish);
    }

    /// Option B (publish-target-none): an explicit empty `targets: []` is the
    /// author's authoritative "never publish" and is honored as an empty set —
    /// NOT re-expanded into the ecosystem default. This is the whole fix: a
    /// version-tracked repo with a registry ecosystem but no publish must be
    /// expressible.
    #[test]
    fn explicit_empty_targets_is_honored_not_expanded() {
        let n =
            norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n---\n");
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let c = n.contract;
        // The rust ecosystem is still recorded …
        assert_eq!(c.ecosystems, vec![Ecosystem::Rust]);
        // … but NO crates.io target is force-expanded: the empty set is honored.
        assert!(
            c.targets.is_empty(),
            "explicit targets:[] must stay empty, got {:?}",
            c.targets
        );
    }

    /// The counterpart to the above: OMITTING `targets` keeps the unchanged
    /// ecosystem-default expansion. Absent ≠ explicit-empty.
    #[test]
    fn omitted_targets_still_expands_to_ecosystem_default() {
        let c = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n").contract;
        assert_eq!(c.targets.len(), 1);
        assert_eq!(c.targets[0].ecosystem, Ecosystem::Rust);
        assert_eq!(c.targets[0].registry, Registry::CratesIo);
        assert_eq!(c.targets[0].adapter, Adapter::CargoPublish);
    }

    /// A YAML `targets:` with a null value (not a list) is treated as *absent*,
    /// not as an explicit empty set — it still expands. Only a genuine empty
    /// sequence `[]` is the authoritative "never publish".
    #[test]
    fn null_targets_expands_like_omitted() {
        let c = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n---\n")
            .contract;
        assert_eq!(c.targets.len(), 1);
        assert_eq!(c.targets[0].registry, Registry::CratesIo);
    }

    /// An empty-targets contract round-trips through canonical JSON unchanged:
    /// `targets` serializes as an empty array `[]` (faithfully reporting the
    /// never-publish intent, not omitting or defaulting it), and re-feeding that
    /// canonical `targets` value back through the normalizer preserves the empty
    /// set — the intent survives a normalize→serialize→normalize cycle.
    #[test]
    fn empty_targets_round_trips_through_canonical_json() {
        let n =
            norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n---\n");
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let json = serde_json::to_value(&n.contract).unwrap();
        // The canonical output faithfully reports the empty set as `[]`.
        assert_eq!(json["targets"], serde_json::json!([]));

        // Re-feed the canonical `targets` value as frontmatter; the empty set is
        // preserved (still no expansion), proving the round-trip is stable.
        let targets_yaml = serde_yaml::to_string(&json["targets"]).unwrap();
        let refed = format!(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: {}\n---\n",
            targets_yaml.trim()
        );
        let n2 = norm(&refed);
        assert!(n2.is_valid(), "errors: {:?}", n2.problems.errors);
        assert_eq!(n2.contract.targets, n.contract.targets);
        assert!(n2.contract.targets.is_empty());
    }

    #[test]
    fn node_monorepo_adapter_is_changesets() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [node]\n\
                    release:\n  model: gated\n  layout: monorepo\n---\n";
        let c = norm(text).contract;
        assert_eq!(c.targets[0].adapter, Adapter::Changesets);
    }

    #[test]
    fn ecosystems_dedup_to_canonical_order() {
        let c =
            norm("---\nstatus: approved\nmaturity: mvp\necosystems: [python, rust, python]\n---\n")
                .contract;
        assert_eq!(c.ecosystems, vec![Ecosystem::Rust, Ecosystem::Python]);
    }

    #[test]
    fn calver_splits_base_and_pattern() {
        let c = norm(
            "---\nstatus: approved\nmaturity: mvp\nversioning: \"calver:YYYY.MM.MICRO\"\n---\n",
        )
        .contract;
        assert_eq!(c.versioning, VersioningBase::Calver);
        assert_eq!(c.versioning_pattern.as_deref(), Some("YYYY.MM.MICRO"));
    }

    #[test]
    fn bare_calver_is_rejected() {
        assert_error_contains(
            &norm("---\nstatus: approved\nmaturity: mvp\nversioning: calver\n---\n"),
            "must carry its pattern",
        );
    }

    #[test]
    fn floor_auto_on_spike() {
        let text = "---\nstatus: approved\nmaturity: spike\n\
                    release:\n  model: auto\n  layout: single\nhealth_badges: [license]\n---\n";
        assert_error_contains(&norm(text), "release.model 'auto' is not allowed");
    }

    #[test]
    fn floor_slsa_l3_production_only() {
        assert_error_contains(
            &norm("---\nstatus: approved\nmaturity: mvp\nprovenance_level: slsa-l3\n---\n"),
            "slsa-l3' is production-only",
        );
    }

    #[test]
    fn floor_registry_requires_valid_license() {
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n\
                    license: Proprietary-Acme\nhealth_badges: [ci, registry, license]\n---\n";
        let n = norm(text);
        // Both the SPDX-validity error and the registry-needs-license floor fire.
        assert_error_contains(&n, "not a valid SPDX expression");
        assert!(n
            .problems
            .errors
            .iter()
            .any(|e| e.contains("floor: a target has a registry")));
    }

    #[test]
    fn floor_badge_without_producer() {
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [python]\n\
                    health_badges: [ci, coverage]\n---\n";
        assert_error_contains(&norm(text), "health_badge 'coverage' has no producer");
    }

    #[test]
    fn floor_schema_version_too_new() {
        assert_error_contains(
            &norm("---\nschema_version: 99\nstatus: approved\nmaturity: mvp\n---\n"),
            "exceeds what this tool knows",
        );
    }

    #[test]
    fn floor_fragment_dir_escape() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    changelog:\n  mode: fragment\n  source: manual\n  fragment_dir: /etc\n---\n";
        assert_error_contains(&norm(text), "must be a relative path inside the repo");
    }

    #[test]
    fn floor_fragment_dir_escape_relative_root() {
        // Regression: with a *relative* repo root (the CLI's `--repo-root .`),
        // a `../`-escaping fragment_dir must still be rejected. The earlier
        // join-then-contain check accepted it because a `.` root normalizes to
        // an empty path that prefixes everything.
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    changelog:\n  mode: fragment\n  source: manual\n  fragment_dir: ../etc\n---\n";
        let n = normalize_str(text, Path::new("."), &FakeFs::empty());
        assert_error_contains(&n, "must be a relative path inside the repo");
    }

    #[test]
    fn path_inside_repo_verdicts() {
        // Inside — plain and `.`/`..`-collapsing relative paths that stay in.
        assert!(path_inside_repo("changelog/fragments"));
        assert!(path_inside_repo("./changelog/fragments"));
        assert!(path_inside_repo("a/../fragments"));
        assert!(path_inside_repo("")); // the repo root itself
                                       // Escapes — absolute, leading `..`, and mid-path `..` that pops out.
        assert!(!path_inside_repo("/etc"));
        assert!(!path_inside_repo("../etc"));
        assert!(!path_inside_repo("a/../../etc"));
    }

    #[test]
    fn unknown_fields_preserved_and_warned() {
        let text =
            "---\nstatus: approved\nmaturity: mvp\nroadmap_url: https://example.com/x\n---\n";
        let n = norm(text);
        assert!(n.is_valid());
        assert_eq!(
            n.contract
                .extra_fields
                .get("roadmap_url")
                .and_then(|v| v.as_str()),
            Some("https://example.com/x")
        );
        assert!(n
            .problems
            .warnings
            .iter()
            .any(|w| w.contains("roadmap_url") && w.contains("forward-compat")));
    }

    #[test]
    fn duplicate_key_is_rejected() {
        assert_error_contains(
            &norm("---\nstatus: approved\nstatus: draft\nmaturity: mvp\n---\n"),
            "invalid YAML",
        );
    }

    #[test]
    fn missing_frontmatter_is_rejected() {
        assert_error_contains(&norm("no frontmatter here\n"), "frontmatter missing");
    }

    #[test]
    fn unclosed_frontmatter_is_rejected() {
        assert_error_contains(&norm("---\nstatus: approved\n"), "frontmatter not closed");
    }

    #[test]
    fn invalid_enum_records_error_and_continues() {
        // A bad status AND a bad maturity: both surface (multi-error collection).
        let n = norm("---\nstatus: bogus\nmaturity: alsobogus\n---\n");
        assert!(n.problems.errors.iter().any(|e| e.contains("status")));
        assert!(n.problems.errors.iter().any(|e| e.contains("maturity")));
    }

    #[test]
    fn fragment_dir_present_suppresses_advisory() {
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n\
                    changelog:\n  mode: fragment\n  source: manual\n---\n";
        // The default fragment dir exists → no advisory warning.
        let fs = FakeFs::with_dirs(["/repo/changelog/fragments"]);
        let n = norm_with(text, &fs);
        assert!(n.is_valid());
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("does not exist yet")),
            "advisory should be suppressed when the dir exists: {:?}",
            n.problems.warnings
        );
    }

    #[test]
    fn serializes_to_schema_v4_shape() {
        let json =
            serde_json::to_value(&norm("---\nstatus: approved\nmaturity: mvp\n---\n").contract)
                .unwrap();
        // Spot-check the §4 top-level keys that consumers read.
        for key in [
            "schema_version",
            "status",
            "maturity",
            "ecosystems",
            "targets",
            "distribution",
            "versioning",
            "versioning_pattern",
            "changelog",
            "conventional_commits",
            "release",
            "contribution_provenance",
            "provenance_level",
            "dependency_bot",
            "health_badges",
            "license",
            "docs_site",
            "extra_fields",
            "warnings",
        ] {
            assert!(json.get(key).is_some(), "missing §4 key {key}");
        }
        assert!(json["versioning_pattern"].is_null());
        // A registry-only contract carries an explicit `distribution: null` — the
        // additive field is present but shape-neutral for existing configs.
        assert!(json["distribution"].is_null());
    }

    // ── distribution (cargo-dist binary layer) ───────────────────────────────

    /// A registry-only contract is unchanged by the additive `distribution`
    /// field: it normalizes clean and `distribution` is `None`.
    #[test]
    fn registry_only_contract_has_no_distribution() {
        let c = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n").contract;
        assert_eq!(c.distribution, None);
        assert_eq!(c.targets.len(), 1);
        assert_eq!(c.targets[0].registry, Registry::CratesIo);
    }

    /// A cargo-dist repo: a `distribution` block (binaries + shell/Homebrew
    /// installers + a tap) coexisting with a crates.io registry target.
    #[test]
    fn cargo_dist_distribution_coexists_with_registry() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    targets:\n  - {ecosystem: rust, package: issuectl, registry: crates.io, adapter: cargo-publish}\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell, homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let c = n.contract;
        // The registry publish is still a Target.
        assert_eq!(c.targets.len(), 1);
        assert_eq!(c.targets[0].registry, Registry::CratesIo);
        assert_eq!(c.targets[0].adapter, Adapter::CargoPublish);
        // The binary layer is the Distribution block.
        let d = c.distribution.expect("distribution present");
        assert_eq!(d.adapter, DistributionAdapter::CargoDist);
        assert!(d.gh_releases); // default true
        assert_eq!(d.installers, vec![Installer::Shell, Installer::Homebrew]);
        assert_eq!(
            d.homebrew_tap.as_deref(),
            Some("jarimustonen/homebrew-issuectl")
        );
    }

    /// Round-trip: the serialized JSON shape a downstream `/oss-*` member reads.
    #[test]
    fn distribution_json_round_trip_shape() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  gh_releases: true\n  \
                    installers: [shell, homebrew]\n  homebrew_tap: jarimustonen/homebrew-issuectl\n---\n";
        let json = serde_json::to_value(&norm(text).contract).unwrap();
        let d = &json["distribution"];
        assert_eq!(d["adapter"], "cargo-dist");
        assert_eq!(d["gh_releases"], true);
        assert_eq!(d["installers"], serde_json::json!(["shell", "homebrew"]));
        assert_eq!(d["homebrew_tap"], "jarimustonen/homebrew-issuectl");
    }

    /// Forward-compat: an unknown key inside the `distribution` block is preserved
    /// under `distribution.extra_fields` (not dropped) and survives a
    /// parse→serialize round-trip, mirroring the top-level `extra_fields` capture.
    /// A warning reports it once; the known distribution keys are unaffected.
    #[test]
    fn distribution_unknown_subkey_preserved_and_warned() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  gh_releases: true\n  \
                    future_signing: {enabled: true, kms_key: alias/oss}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n
            .contract
            .clone()
            .distribution
            .expect("distribution present");
        // The unknown sub-key is captured, with its nested value intact.
        assert_eq!(
            d.extra_fields
                .get("future_signing")
                .and_then(|v| v.get("kms_key"))
                .and_then(|v| v.as_str()),
            Some("alias/oss")
        );
        // Known keys are untouched by the capture.
        assert_eq!(d.adapter, DistributionAdapter::CargoDist);
        assert!(d.gh_releases);
        // It round-trips through the serialized JSON downstream members read.
        let json = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(
            json["distribution"]["extra_fields"]["future_signing"]["enabled"],
            serde_json::json!(true)
        );
        // Reported once, scoped to the block, naming the key.
        assert!(
            n.problems.warnings.iter().any(|w| {
                w.contains("unknown distribution field(s) preserved")
                    && w.contains("future_signing")
            }),
            "expected a scoped forward-compat warning: {:?}",
            n.problems.warnings
        );
    }

    // ── installer ↔ platform cross-check (warning, not a floor) ──────────────

    /// `installers: [msi]` with no Windows triple in `platforms` warns — the MSI
    /// installer points at a binary the release never builds. Still valid (warning,
    /// not error): the contract is internally consistent, just wasteful.
    #[test]
    fn msi_installer_without_windows_platform_warns() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [msi]\n  \
                    platforms: [x86_64-apple-darwin, x86_64-unknown-linux-musl]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("includes 'msi'") && w.contains("no Windows")),
            "expected an msi/Windows cross-check warning: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [msi]` WITH a Windows triple present → no cross-check warning.
    #[test]
    fn msi_installer_with_windows_platform_no_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [msi]\n  \
                    platforms: [x86_64-pc-windows-msvc]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems.warnings.iter().any(|w| w.contains("'msi'")),
            "unexpected msi cross-check warning: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [homebrew]` with NEITHER a macOS nor a Linux triple warns —
    /// the generated formula has nothing to install. (A Windows-only platform set
    /// is the only way to strand a `homebrew` installer, since Homebrew serves
    /// both macOS and Linux.)
    #[test]
    fn homebrew_installer_without_darwin_or_linux_warns() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [x86_64-pc-windows-msvc]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("includes 'homebrew'") && w.contains("nothing to install")),
            "expected a homebrew/(macOS|Linux) cross-check warning: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [homebrew]` is satisfied by a LINUX triple alone (Linuxbrew) —
    /// no darwin triple required. The chosen interpretation: homebrew needs macOS
    /// OR Linux, so a Linux-only platform set is coherent, not a warning.
    #[test]
    fn homebrew_installer_with_linux_only_no_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [x86_64-unknown-linux-musl]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems.warnings.iter().any(|w| w.contains("'homebrew'")),
            "unexpected homebrew cross-check warning for a Linux-only set: {:?}",
            n.problems.warnings
        );
    }

    /// npm and shell installers are OS-agnostic: even a platform set that would
    /// strand an msi (no Windows) never warns for them.
    #[test]
    fn npm_and_shell_installers_never_cross_check_warn() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust, node]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell, npm]\n  \
                    platforms: [x86_64-apple-darwin]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("nothing to install")),
            "OS-agnostic installers must not cross-check warn: {:?}",
            n.problems.warnings
        );
    }

    /// A coherent installer/platform set (msi + Windows, homebrew + darwin) emits
    /// no cross-check warning.
    #[test]
    fn coherent_installer_platform_set_no_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew, msi]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [aarch64-apple-darwin, x86_64-pc-windows-msvc]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("nothing to install")),
            "coherent set must not warn: {:?}",
            n.problems.warnings
        );
    }

    /// ossctl's own contract shape — installers `[shell, powershell]` with a
    /// platform set spanning Windows + macOS + Linux — produces no cross-check
    /// warning (both installers are agnostic here, and every OS is covered anyway).
    #[test]
    fn ossctl_own_contract_shape_no_cross_check_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell, powershell]\n  \
                    platforms: [aarch64-apple-darwin, x86_64-apple-darwin, \
                    x86_64-unknown-linux-musl, aarch64-unknown-linux-musl, \
                    x86_64-pc-windows-msvc]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("nothing to install")),
            "ossctl's own shape must not cross-check warn: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [msi]` with `platforms` OMITTED warns: the default set
    /// (macOS + Linux) carries no Windows triple, so the MSI installs nothing.
    /// This is the common footgun — the author added msi but never listed a
    /// Windows target.
    #[test]
    fn msi_installer_with_defaulted_platforms_warns() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [msi]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("includes 'msi'") && w.contains("no Windows")),
            "expected an msi/Windows warning against the defaulted platform set: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [msi]` is satisfied by a `*-windows-gnu` triple just as by
    /// `*-windows-msvc` — both target the Windows OS. No warning.
    #[test]
    fn msi_installer_with_windows_gnu_no_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [msi]\n  \
                    platforms: [x86_64-pc-windows-gnu]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems.warnings.iter().any(|w| w.contains("'msi'")),
            "windows-gnu must satisfy msi: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [homebrew]` with an ANDROID-only platform set warns: Android
    /// triples (`aarch64-linux-android`) carry `linux` in the *vendor* slot but an
    /// `android` OS component — Homebrew/Linuxbrew does not serve Android, so the
    /// formula has nothing to install. Regression guard for the positional
    /// `triple_os` OS-component match (vs a naive any-component `== "linux"`).
    #[test]
    fn homebrew_installer_with_android_only_warns() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [aarch64-linux-android]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("includes 'homebrew'") && w.contains("nothing to install")),
            "Android-only must strand a homebrew installer: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [homebrew]` with an APPLE-iOS-only set warns: `*-apple-ios`
    /// carries an `ios` OS component, not `darwin`, so it is not a macOS target and
    /// Homebrew serves neither iOS nor (here) Linux.
    #[test]
    fn homebrew_installer_with_apple_ios_only_warns() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [aarch64-apple-ios]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("includes 'homebrew'") && w.contains("nothing to install")),
            "apple-ios must not satisfy homebrew's macOS need: {:?}",
            n.problems.warnings
        );
    }

    /// `installers: [homebrew]` with a macOS-only set (no Linux) is coherent — the
    /// isolated darwin case, distinct from the Linux-only test above.
    #[test]
    fn homebrew_installer_with_macos_only_no_warning() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [aarch64-apple-darwin]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems.warnings.iter().any(|w| w.contains("'homebrew'")),
            "macOS-only must satisfy homebrew: {:?}",
            n.problems.warnings
        );
    }

    /// Two stranded installers → two independent warnings. A wasm-only platform
    /// set has no OS component any installer supports, so both `msi` and `homebrew`
    /// warn (exactly once each — the installer list is de-duped and canonically
    /// ordered).
    #[test]
    fn both_installers_stranded_warn_once_each() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew, msi]\n  \
                    homebrew_tap: jarimustonen/homebrew-issuectl\n  \
                    platforms: [wasm32-unknown-unknown]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let msi = n
            .problems
            .warnings
            .iter()
            .filter(|w| w.contains("includes 'msi'"))
            .count();
        let brew = n
            .problems
            .warnings
            .iter()
            .filter(|w| w.contains("includes 'homebrew'"))
            .count();
        assert_eq!((msi, brew), (1, 1), "warnings: {:?}", n.problems.warnings);
    }

    /// A malformed triple that happens to contain an OS keyword must NOT drive the
    /// cross-check: the block has a parse error (uppercase triple), so the advisory
    /// is gated off entirely. Otherwise the misspelled `x86_64-PC-WINDOWS-MSVC`
    /// would silently "satisfy" msi and the warning would flip once the author
    /// fixed the typo.
    #[test]
    fn malformed_platform_triple_gates_off_cross_check() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [msi]\n  \
                    platforms: [x86_64-PC-WINDOWS-MSVC]\n---\n";
        let n = norm(text);
        // The uppercase triple is a hard error → the document is invalid …
        assert!(!n.is_valid(), "expected a malformed-triple error");
        // … and the cross-check emitted no (misleading) installer/platform warning.
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("nothing to install")),
            "cross-check must be gated off while platforms has errors: {:?}",
            n.problems.warnings
        );
    }

    /// A distribution block setting EVERY known key carries an empty
    /// `extra_fields` map and emits no forward-compat warning — the additive field
    /// is shape-neutral for existing contracts. Exercising all of
    /// `KNOWN_DISTRIBUTION_KEYS` guards against the allowlist drifting out of sync
    /// with the struct (a new known key wrongly captured as "unknown").
    #[test]
    fn distribution_all_known_keys_has_empty_extra_fields() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  gh_releases: true\n  \
                    installers: [shell, homebrew]\n  homebrew_tap: owner/tap\n  \
                    platforms: [x86_64-unknown-linux-musl]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n.contract.distribution.expect("distribution present");
        assert!(d.extra_fields.is_empty());
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("unknown distribution field(s) preserved")),
            "no forward-compat warning for an all-known-keys block: {:?}",
            n.problems.warnings
        );
    }

    /// Top-level and nested `extra_fields` capture are independent: a contract
    /// with BOTH an unknown top-level key AND an unknown distribution sub-key
    /// populates both maps and warns once for each, with the correct
    /// `schema_version` in each message.
    #[test]
    fn distribution_and_top_level_extra_fields_coexist() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    roadmap_url: https://example.com/x\n\
                    distribution:\n  adapter: cargo-dist\n  future_x: 1\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let c = n.contract.clone();
        assert!(c.extra_fields.contains_key("roadmap_url"));
        let d = c.distribution.expect("distribution present");
        assert_eq!(d.extra_fields.get("future_x"), Some(&serde_json::json!(1)));
        // Two independent forward-compat warnings, each naming schema_version 1.
        let fc: Vec<&String> = n
            .problems
            .warnings
            .iter()
            .filter(|w| w.contains("forward-compat") && w.contains("schema_version 1"))
            .collect();
        assert_eq!(fc.len(), 2, "expected two versioned warnings: {fc:?}");
    }

    // ── extra_fields capture hardening ───────────────────────────────────────

    /// A non-string top-level mapping key (`42:`, legal YAML) is a STRUCTURAL
    /// error, not silently coerced/dropped: distinct non-string keys collapse onto
    /// the same JSON key (`42` and `"42"`; every list key onto `<list>`), so
    /// preserving them losslessly is impossible — the normalizer rejects instead,
    /// keeping the never-drop invariant vacuously.
    #[test]
    fn non_string_top_level_key_rejected() {
        let n = norm("---\nstatus: approved\nmaturity: mvp\n42: answer\n---\n");
        assert_error_contains(&n, "must be a string");
        assert!(
            n.problems.errors.iter().any(|e| e.contains("42")),
            "error should name the offending key: {:?}",
            n.problems.errors
        );
    }

    /// The nested `distribution` scan rejects the same way, with the block scope in
    /// the message.
    #[test]
    fn non_string_distribution_key_rejected() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  true: enabled\n---\n";
        let n = norm(text);
        assert_error_contains(&n, "must be a string");
        assert!(
            n.problems
                .errors
                .iter()
                .any(|e| e.contains("distribution field key")),
            "error should be scoped to the distribution block: {:?}",
            n.problems.errors
        );
    }

    /// A known key placed normally is parsed as its field and NOT double-captured
    /// into `extra_fields` — the dedupe guarantee (a key is never both a known
    /// field and an extra field).
    #[test]
    fn known_key_not_double_captured() {
        let n = norm("---\nstatus: approved\nmaturity: production\necosystems: [rust]\n---\n");
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(!n.contract.extra_fields.contains_key("ecosystems"));
        assert!(!n.contract.extra_fields.contains_key("status"));
        assert!(n.contract.extra_fields.is_empty());
    }

    /// The reserved `extra_fields` metadata key is not re-captured into a nested
    /// `extra_fields.extra_fields`; its mapping contents are MERGED back, so a
    /// hand-authored (or defensively re-fed canonical) block round-trips losslessly
    /// rather than being silently dropped. The derived `warnings` key is ignored
    /// (regenerated), not preserved — it is not user contract data.
    #[test]
    fn reserved_extra_fields_block_merged_warnings_ignored() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    extra_fields:\n  foo: 1\nwarnings:\n  - a prior note\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        // `foo` is preserved (merged), not nested and not dropped.
        assert_eq!(
            n.contract.extra_fields.get("foo"),
            Some(&serde_json::json!(1))
        );
        assert!(!n.contract.extra_fields.contains_key("extra_fields"));
        // The stale input `warnings` list is not resurrected into the output.
        assert!(
            !n.contract
                .warnings
                .iter()
                .any(|w| w.contains("a prior note")),
            "input warnings must be regenerated, not preserved: {:?}",
            n.contract.warnings
        );
    }

    /// The nested analogue: `distribution.extra_fields` is merged back, not nested
    /// and not dropped.
    #[test]
    fn distribution_reserved_extra_fields_block_merged() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  extra_fields:\n    foo: 1\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n.contract.distribution.expect("distribution present");
        assert_eq!(d.extra_fields.get("foo"), Some(&serde_json::json!(1)));
        assert!(!d.extra_fields.contains_key("extra_fields"));
    }

    /// Idempotence: normalizing, serializing the canonical `extra_fields` map, and
    /// re-feeding it as an `extra_fields` block yields the identical map — the
    /// round-trip the reserve+merge design guarantees (no nesting, no loss).
    #[test]
    fn extra_fields_round_trip_is_idempotent() {
        let first = norm("---\nstatus: approved\nmaturity: mvp\nroadmap_url: https://x/y\n---\n");
        assert!(first.is_valid(), "errors: {:?}", first.problems.errors);
        assert_eq!(first.contract.extra_fields.len(), 1);
        // Feed the captured extra_fields back under the reserved key.
        let inner = serde_yaml::to_string(&first.contract.extra_fields).unwrap();
        let indented = inner
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let text =
            format!("---\nstatus: approved\nmaturity: mvp\nextra_fields:\n{indented}\n---\n");
        let second = norm(&text);
        assert!(second.is_valid(), "errors: {:?}", second.problems.errors);
        assert_eq!(second.contract.extra_fields, first.contract.extra_fields);
    }

    /// A key present BOTH inside the reserved `extra_fields` block AND as a sibling
    /// unknown top-level key is an ambiguity error — never a silent overwrite of
    /// either value (dedupe: a key resolves to exactly one source).
    #[test]
    fn extra_fields_block_sibling_collision_is_error() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    extra_fields:\n  dup: 1\ndup: 2\n---\n";
        let n = norm(text);
        assert_error_contains(&n, "appears both");
    }

    /// A reserved `extra_fields` value that is not a mapping is a structural error
    /// (it can only carry preserved key/value pairs).
    #[test]
    fn reserved_extra_fields_non_mapping_is_error() {
        let n = norm("---\nstatus: approved\nmaturity: mvp\nextra_fields: nonsense\n---\n");
        assert_error_contains(&n, "must be a mapping");
    }

    /// A contract setting EVERY parsed top-level known key carries an empty
    /// `extra_fields` and emits no forward-compat warning — the top-level analogue
    /// of `distribution_all_known_keys_has_empty_extra_fields`, guarding
    /// [`KNOWN_KEYS`] against drifting out of sync with the [`Contract`] struct (a
    /// new field whose key is missing here would be wrongly captured as unknown).
    #[test]
    fn top_level_all_known_keys_has_empty_extra_fields() {
        let text = "---\nschema_version: 1\nstatus: approved\nmaturity: production\n\
                    ecosystems: [rust]\n\
                    targets:\n  - {ecosystem: rust, package: x, registry: crates.io, adapter: cargo-publish}\n\
                    distribution:\n  adapter: cargo-dist\n\
                    versioning: semver\n\
                    changelog:\n  mode: curated\n  source: manual\n\
                    conventional_commits: false\n\
                    release:\n  model: gated\n  layout: single\n\
                    contribution_provenance: none\n\
                    provenance_level: none\n\
                    dependency_bot: dependabot\n\
                    health_badges: [ci, registry, license]\n\
                    license: MIT\n\
                    docs_site: none\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.contract.extra_fields.is_empty(),
            "unexpected extra_fields (KNOWN_KEYS drift?): {:?}",
            n.contract.extra_fields
        );
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("forward-compat")),
            "no forward-compat warning for an all-known-keys contract: {:?}",
            n.problems.warnings
        );
    }

    /// Installers de-dup into canonical order regardless of source order.
    #[test]
    fn distribution_installers_dedup_canonical_order() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew, shell, homebrew]\n  \
                    homebrew_tap: owner/tap\n---\n";
        let d = norm(text).contract.distribution.unwrap();
        assert_eq!(d.installers, vec![Installer::Shell, Installer::Homebrew]);
    }

    /// A `homebrew` installer without a tap is a floor error.
    #[test]
    fn distribution_homebrew_installer_requires_tap() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell, homebrew]\n---\n";
        assert_error_contains(
            &norm(text),
            "includes 'homebrew' but no distribution.homebrew_tap",
        );
    }

    /// A malformed tap slug (not `owner/repo`) is rejected AND, because the
    /// invalid value substitutes `None`, the homebrew-needs-tap floor still fires
    /// — a present-but-invalid tap must not slip a `homebrew` installer through.
    #[test]
    fn distribution_bad_tap_slug_rejected() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: not-a-slug\n---\n";
        let n = norm(text);
        assert_error_contains(&n, "must be an 'owner/repo' slug");
        assert!(
            n.problems
                .errors
                .iter()
                .any(|e| e.contains("includes 'homebrew' but no distribution.homebrew_tap")),
            "the tap floor must still fire on an invalid (→None) tap: {:?}",
            n.problems.errors
        );
        // The malformed slug never leaks into the built block.
        assert_eq!(n.contract.distribution.unwrap().homebrew_tap, None);
    }

    /// An unknown installer flavor surfaces an error listing the valid set.
    #[test]
    fn distribution_bad_installer_rejected() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [snap]\n---\n";
        assert_error_contains(&norm(text), "distribution.installers");
    }

    /// `adapter` is required when a distribution block is present — a bare
    /// `distribution: {}` must not silently claim cargo-dist ownership.
    #[test]
    fn distribution_adapter_is_required() {
        assert_error_contains(
            &norm("---\nstatus: approved\nmaturity: mvp\ndistribution: {}\n---\n"),
            "distribution.adapter is required",
        );
    }

    /// A distribution block ships public binaries — forbidden at maturity 'spike'
    /// (mirrors the `release.model: auto`-on-spike floor).
    #[test]
    fn distribution_forbidden_on_spike() {
        let text = "---\nstatus: approved\nmaturity: spike\n\
                    distribution:\n  adapter: cargo-dist\n---\n";
        assert_error_contains(&norm(text), "not allowed on maturity 'spike'");
    }

    /// A `homebrew_tap` set with neither a `homebrew` installer nor a
    /// `homebrew`-registry target is dead config — a warning, not a floor (the
    /// contract is still valid). This is the genuinely-orphaned tap: no consumer
    /// exists, so the tap is truly never updated.
    #[test]
    fn distribution_tap_without_installer_warns() {
        let text = "---\nstatus: approved\nmaturity: mvp\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  \
                    homebrew_tap: owner/tap\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("no formula is generated, so the tap will never be updated")),
            "expected dead-tap warning: {:?}",
            n.problems.warnings
        );
    }

    /// A `homebrew_tap` set with NO `homebrew` installer but WITH a
    /// `homebrew`-registry target (the release engine's homebrew-tap adapter, which
    /// pushes the formula in its `dist` phase) is NOT dead config — the tap IS
    /// updated by the engine, so the dead-config warning must NOT fire. This is
    /// ossctl's own (correct) contract shape.
    #[test]
    fn distribution_tap_with_homebrew_target_no_warning() {
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n\
                    targets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: homebrew-tap}\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  \
                    homebrew_tap: owner/tap\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        // Stronger than a negative substring match: the whole contract is clean,
        // so it must produce NO warnings at all — this also catches any reworded
        // dead-tap advisory that a substring check would miss.
        assert!(
            n.problems.warnings.is_empty(),
            "homebrew-target contract must not warn: {:?}",
            n.problems.warnings
        );
    }

    /// A goreleaser distribution with no installers and no tap is valid — the
    /// block is minimal and forward-compatible.
    #[test]
    fn distribution_goreleaser_minimal_is_valid() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [go]\n\
                    distribution:\n  adapter: goreleaser\n  gh_releases: true\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n.contract.distribution.unwrap();
        assert_eq!(d.adapter, DistributionAdapter::Goreleaser);
        assert!(d.installers.is_empty());
        assert_eq!(d.homebrew_tap, None);
    }

    /// A non-mapping `distribution` value is a structural error.
    #[test]
    fn distribution_non_mapping_rejected() {
        assert_error_contains(
            &norm("---\nstatus: approved\nmaturity: mvp\ndistribution: [nope]\n---\n"),
            "distribution must be a mapping",
        );
    }

    // ── distribution.platforms (cross-platform target set) ───────────────────

    /// Helper: does a platform list contain any Linux triple? The cross-platform
    /// install requirement is "at least one Linux triple", inspected via the OS
    /// component of the triple (exactly how `audit` will read this field).
    fn has_linux(platforms: &[String]) -> bool {
        platforms.iter().any(|t| t.contains("-linux"))
    }

    /// Omitted `platforms` → the cross-platform default (macOS + Linux). The
    /// KEYSTONE assertion: the DEFAULT covers Linux, so every distribution that
    /// omits the field does (an explicit set is the author's own choice, which the
    /// cross-platform `audit` — not this normalizer — checks).
    #[test]
    fn distribution_platforms_default_is_cross_platform() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n---\n";
        let d = norm(text)
            .contract
            .distribution
            .expect("distribution present");
        assert_eq!(
            d.platforms,
            vec![
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-musl",
                "x86_64-unknown-linux-musl",
            ]
        );
        assert!(
            has_linux(&d.platforms),
            "the default set MUST contain a Linux triple: {:?}",
            d.platforms
        );
    }

    /// An explicit `platforms` list round-trips through normalization and the
    /// serialized JSON downstream members read, order + values preserved.
    #[test]
    fn distribution_platforms_explicit_round_trips() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  \
                    platforms: [x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc]\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n.contract.clone().distribution.unwrap();
        assert_eq!(
            d.platforms,
            vec!["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]
        );
        let json = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(
            json["distribution"]["platforms"],
            serde_json::json!(["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"])
        );
    }

    /// An explicit empty `platforms: []` is a hard error — NOT silently defaulted.
    /// Only an omitted/null field yields the cross-platform default; an empty list
    /// is a mistake (a distribution with no platforms builds nothing) and, if
    /// silently defaulted, would surprise the author and erase the intent the
    /// cross-platform audit needs to see.
    #[test]
    fn distribution_platforms_empty_is_rejected() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  platforms: []\n---\n";
        assert_error_contains(&norm(text), "empty list — omit the key");
    }

    /// Duplicate triples de-duplicate, preserving first-seen order.
    #[test]
    fn distribution_platforms_dedup_preserves_order() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  \
                    platforms: [aarch64-apple-darwin, x86_64-apple-darwin, aarch64-apple-darwin]\n---\n";
        let d = norm(text).contract.distribution.unwrap();
        assert_eq!(
            d.platforms,
            vec!["aarch64-apple-darwin", "x86_64-apple-darwin"]
        );
    }

    /// A malformed triple is rejected with a message naming the field.
    #[test]
    fn distribution_platforms_bad_triple_rejected() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  platforms: [not_a_triple]\n---\n";
        assert_error_contains(&norm(text), "is not a well-formed target-triple");
    }

    /// A non-string entry (a nested list) is rejected structurally.
    #[test]
    fn distribution_platforms_non_string_entry_rejected() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  platforms: [[nope]]\n---\n";
        assert_error_contains(&norm(text), "each entry must be a target-triple string");
    }

    /// A `platforms` value that is not a list is a structural error.
    #[test]
    fn distribution_platforms_non_list_rejected() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  platforms: x86_64-apple-darwin\n---\n";
        assert_error_contains(&norm(text), "must be a list of target-triple strings");
    }

    /// Regression: a registry-only contract (no distribution block at all) is
    /// wholly unaffected by the additive `platforms` field — no distribution, so
    /// no `platforms` in the emitted shape.
    #[test]
    fn registry_only_contract_unaffected_by_platforms() {
        let json = serde_json::to_value(
            &norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n").contract,
        )
        .unwrap();
        assert!(json["distribution"].is_null());
    }

    #[test]
    fn looks_like_target_triple_verdicts() {
        // Standard triples across arch/vendor/os/env shapes.
        assert!(looks_like_target_triple("aarch64-apple-darwin"));
        assert!(looks_like_target_triple("x86_64-apple-darwin"));
        assert!(looks_like_target_triple("x86_64-unknown-linux-musl"));
        assert!(looks_like_target_triple("x86_64-unknown-linux-gnu"));
        assert!(looks_like_target_triple("x86_64-pc-windows-msvc"));
        assert!(looks_like_target_triple("armv7-unknown-linux-gnueabihf"));
        assert!(looks_like_target_triple("wasm32-wasi"));
        // Real dotted arch names must pass (regression: the `.` was rejected).
        assert!(looks_like_target_triple("thumbv8m.main-none-eabi"));
        assert!(looks_like_target_triple("thumbv8m.base-none-eabi"));
        // Rejects: too few/many components, empty parts, case, punctuation.
        assert!(!looks_like_target_triple("linux"));
        assert!(!looks_like_target_triple("a-b-c-d-e"));
        assert!(!looks_like_target_triple("x86_64--linux"));
        assert!(!looks_like_target_triple("-apple-darwin"));
        assert!(!looks_like_target_triple("X86_64-apple-darwin"));
        assert!(!looks_like_target_triple("x86_64-apple-darwin;rm"));
        assert!(!looks_like_target_triple("x86_64 apple darwin"));
        assert!(!looks_like_target_triple(""));
        // Structural-only: nonsense that happens to be well-formed IS accepted —
        // the toolchain, not the contract, is the authority on buildability.
        assert!(looks_like_target_triple("aa-bb"));
    }

    #[test]
    fn is_tap_slug_verdicts() {
        // Valid GitHub-style slugs.
        assert!(is_tap_slug("owner/repo"));
        assert!(is_tap_slug("jarimustonen/homebrew-issuectl"));
        assert!(is_tap_slug("Owner_1/repo.rb"));
        // Structural rejects.
        assert!(!is_tap_slug("no-slash"));
        assert!(!is_tap_slug("/repo"));
        assert!(!is_tap_slug("owner/"));
        assert!(!is_tap_slug("owner/repo/extra"));
        assert!(!is_tap_slug("owner / repo"));
        // Strict-charset rejects: path traversal, punctuation, injection chars.
        assert!(!is_tap_slug("owner/.."));
        assert!(!is_tap_slug("../repo"));
        assert!(!is_tap_slug("owner/repo;rm -rf"));
        assert!(!is_tap_slug("owner/@repo"));
        assert!(!is_tap_slug("ownér/repo"));
    }
}

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
use crate::facts::{detect_distribution_surface, CargoPublishPolicy};
use crate::ports::Fs;
use crate::release::distribution::{
    find_undeclared_distribution, undeclared_distribution_warnings,
};

/// The contract file the normalizer reads, relative to the repo root.
pub const CONTRACT_FILENAME: &str = "OSS-RELEASE.md";

/// The optional cargo-dist configuration checked for a Homebrew-release drift.
const DIST_WORKSPACE_FILENAME: &str = "dist-workspace.toml";

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
    // Both distribution input keys are known: `distribution` (a single mapping,
    // v1 back-compat) and `distributions` (a sequence, the monorepo shape). See
    // [`parse_distributions`]; declaring both is an error, not an unknown-field.
    "distribution",
    "distributions",
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
    // schema_version — validate the DECLARED version (a too-new config is a hard
    // stop, a sub-1 or non-integer is an error), but do NOT echo it: the canonical
    // output is ALWAYS the current shape, so the emitted `schema_version` is
    // KNOWN_SCHEMA_VERSION regardless of what the (older, still-readable) document
    // declared. Echoing the declared version would stamp a canonical v2 body with a
    // v1 number — a mislabeled, self-inconsistent shape a strict consumer cannot
    // trust. The tool reads a v1 `distribution:` mapping and emits the v2
    // `distributions: [...]` shape under `schema_version: 2`.
    match map.get("schema_version") {
        None => {}
        Some(v) => match v.as_i64() {
            Some(n) if n > i64::from(KNOWN_SCHEMA_VERSION) => p.err(format!(
                "schema_version {n} exceeds what this tool knows ({KNOWN_SCHEMA_VERSION}); \
                 upgrade the OSS-release skills before reading this config (refusing rather \
                 than guessing)."
            )),
            Some(n) if n < 1 => p.err(format!("schema_version {n} is invalid (must be >= 1)")),
            Some(_) => {}
            None => p.err(format!(
                "schema_version must be an integer, got {}",
                yaml_display(v)
            )),
        },
    }
    let schema_version = KNOWN_SCHEMA_VERSION;

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

    // release (model + layout + optional bump_hook).
    let (model, layout, bump_hook) = match map.get("release") {
        None | Some(Value::Null) => (ReleaseModel::Gated, ReleaseLayout::Single, None),
        Some(Value::Mapping(m)) => (
            enum_field!(m, "model", ReleaseModel, ReleaseModel::Gated, p),
            enum_field!(m, "layout", ReleaseLayout, ReleaseLayout::Single, p),
            parse_bump_hook(m, p),
        ),
        Some(_) => {
            p.err("release must be a mapping (model / layout / bump_hook)".to_string());
            (ReleaseModel::Gated, ReleaseLayout::Single, None)
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

    // distributions — the binary-distribution blocks (cargo-dist/goreleaser); a
    // registry-only repo has none (→ empty list), leaving its contract shape
    // unchanged. The homebrew cross-field truth table (tap ↔ installer-producer ↔
    // target-producer) is enforced afterwards by [`check_homebrew_configuration`],
    // once both `targets` and `distributions` are resolved.
    let distributions = parse_distributions(map, &targets, schema_version, p);

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
            "floor: changelog.fragment_dir {} must be a relative path inside the repo (an \
             absolute or '../'-escaping path is refused)",
            quote_for_diagnostic(&changelog.fragment_dir)
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
                        "license {} is not a valid SPDX expression (unknown id or malformed \
                         AND/OR/WITH grammar)",
                        quote_for_diagnostic(s)
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
             {} is not a valid SPDX expression",
            quote_for_diagnostic(&license)
        ));
    }
    check_badge_producers(&health_badges, maturity, &targets, p);
    // Homebrew cross-field consistency: missing-tap (either producer), the
    // double-publish collision, and the dead-tap advisory — the full truth table.
    check_homebrew_configuration(&targets, &distributions, p);
    check_publisher_conflicts(&targets, p);
    check_cargo_publish_evidence(&ecosystems, &targets, repo_root, fs, p);
    check_dist_workspace_homebrew(&targets, &distributions, repo_root, fs, p);
    let distribution_surface = detect_distribution_surface(repo_root, fs);
    for warning in undeclared_distribution_warnings(&find_undeclared_distribution(
        &targets,
        &distribution_surface,
        distributions
            .iter()
            .any(|d| d.homebrew_tap.is_some() && !d.installers.contains(&Installer::Homebrew)),
    )) {
        p.warn(warning);
    }
    // Publish-none must mean NOTHING is published, by anyone. A distribution block is
    // a second, independent publish surface (GH-Release binaries, an installer, a tap
    // formula) produced by a tag-triggered workflow — so `targets: []` next to a
    // declared distribution is a self-contradiction with real consequences: the engine
    // reads the empty target set as a TAG-ONLY cut, pushes the tag, and reports
    // "published nothing" while that very tag triggers cargo-dist/goreleaser to publish
    // binaries the run never planned, journalled, or verified. This floor is what makes
    // `targets.is_empty()` a sound publish-none discriminator for the coordinator.
    if targets.is_empty() && !distributions.is_empty() {
        p.err(
            "floor: a distribution block declares a binary publish surface (GitHub Release \
             artifacts / installers / a tap formula) but targets is empty — the two contradict \
             each other: the release engine would treat the empty target set as publish-none and \
             cut a TAG-ONLY release, while the pushed tag triggers the distribution's workflow to \
             publish anyway, unplanned and unverified. Declare the distribution's target (e.g. \
             {ecosystem, package, registry: gh-releases, adapter: cargo-dist}) or drop the \
             distribution block for a genuine publish-none contract"
                .to_string(),
        );
    }
    // A distribution block ships public binaries (GH-Release artifacts, a curl-pipe
    // installer, a Homebrew tap PR) — that is publishing, and a spike is not being
    // published. Mirrors the `release.model: auto` floor: raise maturity or drop the
    // block. (No blocks → no constraint; registry-only spikes are unaffected.)
    if !distributions.is_empty() && maturity == Maturity::Spike {
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
            "changelog.mode 'fragment' but the fragment dir {} does not exist yet under {} — \
             /oss-changelog creates it; /oss-readiness reports it as a gap until then",
            quote_for_diagnostic(&changelog.fragment_dir),
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
        distributions,
        versioning,
        versioning_pattern,
        changelog,
        conventional_commits,
        release: Release {
            model,
            layout,
            bump_hook,
        },
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

/// Warn when cargo-dist configures Homebrew but the release contract cannot plan it.
///
/// `OSS-RELEASE.md` remains authoritative: cargo-dist's config is only a
/// best-effort drift signal. A missing, unreadable, invalid, or irrelevant file
/// therefore produces no diagnostic and never changes validation's success.
fn check_dist_workspace_homebrew(
    targets: &[Target],
    distributions: &[Distribution],
    repo_root: &Path,
    fs: &dyn Fs,
    p: &mut Problems,
) {
    let path = repo_root.join(DIST_WORKSPACE_FILENAME);
    let Ok(bytes) = fs.read(&path) else {
        return;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return;
    };
    let Ok(config) = text.parse::<toml::Value>() else {
        return;
    };
    let Some(dist) = config.get("dist").and_then(toml::Value::as_table) else {
        return;
    };

    let configured_tap = dist
        .get("tap")
        .and_then(toml::Value::as_str)
        .filter(|tap| !tap.trim().is_empty());
    let has_tap = configured_tap.is_some();
    let has_homebrew_publish_job = dist
        .get("publish-jobs")
        .and_then(toml::Value::as_array)
        .is_some_and(|jobs| {
            jobs.iter()
                .filter_map(toml::Value::as_str)
                .any(|job| job == "homebrew")
        });

    let contract_tap = distributions.iter().find_map(|d| d.homebrew_tap.as_deref());
    let has_delegated_homebrew_target = targets.iter().any(|target| {
        target.registry == Registry::Homebrew && target.adapter == Adapter::CargoDist
    });
    if has_homebrew_publish_job && !has_delegated_homebrew_target {
        p.err(
            "floor: dist-workspace.toml publish-jobs includes 'homebrew', but the contract has \
             no delegated Homebrew target — cargo-dist would write a formula that the verify \
             barrier never observes. Add a target with registry 'homebrew' and adapter \
             'cargo-dist', or remove cargo-dist's Homebrew publish job"
                .to_string(),
        );
    }
    if has_homebrew_publish_job
        && targets.iter().any(|target| {
            target.registry == Registry::Homebrew && target.adapter == Adapter::HomebrewTap
        })
    {
        p.err(
            "floor: dist-workspace.toml publish-jobs includes 'homebrew', but the contract's \
             Homebrew target uses adapter 'homebrew-tap' — cargo-dist CI and ossctl would both \
             write the same tap. Change that target to adapter 'cargo-dist' so the engine \
             delegates the write and verifies the formula, or remove cargo-dist's Homebrew \
             publish job"
                .to_string(),
        );
    }
    if (has_tap || has_homebrew_publish_job) && contract_tap.is_none() {
        p.warn(
            "dist-workspace.toml configures Homebrew, but the contract omits \
             distribution.homebrew_tap; the Homebrew leg will not be planned. Add \
             distribution.homebrew_tap: owner/repo to OSS-RELEASE.md"
                .to_string(),
        );
    }

    // A CI-delegated target has no publish receipt carrying the actual tap, so its
    // post-cut observer must use the contract destination. A disagreement with
    // cargo-dist's configured tap could observe an unrelated stale formula and
    // report a false green. Reject that reachable double-source mismatch early.
    let has_ci_delegated_homebrew = targets.iter().any(|target| {
        target.registry == Registry::Homebrew && target.adapter == Adapter::CargoDist
    });
    if let (true, Some(configured), Some(declared)) =
        (has_ci_delegated_homebrew, configured_tap, contract_tap)
    {
        if configured != declared {
            p.err(format!(
                "floor: dist-workspace.toml configures Homebrew tap {}, but the CI-delegated \
                 target's distribution.homebrew_tap is {} — cargo-dist would write one tap while \
                 ossctl verifies another",
                quote_for_diagnostic(configured),
                quote_for_diagnostic(declared)
            ));
        }
    }
}

/// Parse the optional `release.bump_hook` command string.
///
/// Absent/`null` → `None` (the default, no hook). A present value must be a
/// **non-empty** string (the engine runs it verbatim in the clean checkout during
/// the bump phase); an empty string or a non-string is a fatal error, substituting
/// `None` so the built contract never carries a malformed hook (the "placeholders
/// keep the strong type" error-path rule the rest of the normalizer follows).
fn parse_bump_hook(m: &Mapping, p: &mut Problems) -> Option<String> {
    match m.get("bump_hook") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_str() {
            Some(s) if !s.trim().is_empty() => Some(s.to_string()),
            Some(_) => {
                p.err(
                    "release.bump_hook must be a non-empty command string (or omit it for no hook)"
                        .to_string(),
                );
                None
            }
            None => {
                p.err("release.bump_hook must be a command string".to_string());
                None
            }
        },
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
            "versioning {} invalid — must be semver | calver:<pattern> | zerover",
            quote_for_diagnostic(s)
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

/// Floor the registry/ecosystem/adapter combinations a target may declare — the
/// checks that stop a well-formed-but-inert (or dangerously mis-routed) target
/// before a cut, not during one.
///
/// Each field is an `Option` because a parse error already reported its own
/// problem and substituted nothing; a combination is only judged once the fields
/// it involves are well-formed.
fn check_target_adapter_compat(
    idx: usize,
    ecosystem: Option<Ecosystem>,
    registry: Option<Registry>,
    adapter: Option<Adapter>,
    p: &mut Problems,
) {
    // Floor: registry/adapter compatibility. A `homebrew`-registry target is
    // either engine-owned (`homebrew-tap` pushes a personal-tap formula;
    // `homebrew-core` opens the central-formula PR) or CI-delegated
    // (`cargo-dist`'s publish-homebrew-formula job writes the tap). Any other
    // adapter has no formula path, so the target would silently do nothing at
    // cut time. Reject it here rather than at release time. Only checked once
    // both fields are well-formed (a parse error already reported its problem).
    if let (Some(Registry::Homebrew), Some(a)) = (registry, adapter) {
        if !matches!(
            a,
            Adapter::HomebrewTap | Adapter::HomebrewCore | Adapter::CargoDist
        ) {
            p.err(format!(
                "floor: targets[{idx}] has registry 'homebrew' but adapter {} — a \
                 homebrew-registry target requires adapter 'homebrew-tap' (personal tap), \
                 'homebrew-core' (central formula), or 'cargo-dist' (CI-delegated tap)",
                quote_for_diagnostic(a.as_str())
            ));
        }
    }

    // Floor: `cargo-publish-ci` (the CI-delegated crates.io publish) is
    // meaningful only for a rust crate going to crates.io. Its whole contract
    // with the engine is "skip the local publish, then observe crates.io" — on
    // any other registry there is nothing the observer knows how to look at, so
    // the target would tag, skip, and then fail the mandatory verify barrier
    // AFTER the irreversible tag push. Reject it at normalization instead, where
    // nothing has happened yet. (The cargo adapter refuses a non-crates.io
    // registry too, but only once a cut is already running.)
    if let (Some(r), Some(Adapter::CargoPublishCi)) = (registry, adapter) {
        if r != Registry::CratesIo {
            p.err(format!(
                "floor: targets[{idx}] has adapter 'cargo-publish-ci' but registry {} — the \
                 CI-delegated cargo publish targets crates.io only (use 'cargo-publish' for \
                 an engine-run publish, or the registry's own adapter)",
                quote_for_diagnostic(r.as_str())
            ));
        }
        if let Some(e) = ecosystem {
            if e != Ecosystem::Rust {
                p.err(format!(
                    "floor: targets[{idx}] has adapter 'cargo-publish-ci' but ecosystem {} — \
                     `cargo publish` releases a rust crate",
                    quote_for_diagnostic(e.as_str())
                ));
            }
        }
    }
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
                        "targets[{idx}].ecosystem {} is not in ecosystems {:?}",
                        quote_for_diagnostic(s),
                        ecosystems.iter().map(|e| e.as_str()).collect::<Vec<_>>()
                    ));
                }
                Some(e)
            } else {
                p.err(format!(
                    "targets[{idx}].ecosystem {} invalid — one of {:?}",
                    quote_for_diagnostic(s),
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
                        "targets[{idx}].registry {} invalid — one of {:?}",
                        quote_for_diagnostic(s),
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

        check_target_adapter_compat(idx, ecosystem, registry, adapter, p);

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
    "package",
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

/// Parse the optional distribution layer, accepting BOTH input spellings:
/// `distribution:` (a single mapping — v1 back-compat, the overwhelmingly common
/// case) and `distributions:` (a sequence of mappings — a monorepo shipping
/// several independently-distributed binaries). A registry-only repo declares
/// neither (or a bare/null key) and gets an empty list, leaving its contract
/// shape unchanged. Declaring BOTH keys at once is ambiguous and is an error.
///
/// Each element is parsed by [`parse_one_distribution`]; the collection-level
/// floor (a monorepo's `package` must be present and unique) lives here.
fn parse_distributions(
    map: &Mapping,
    targets: &[Target],
    schema_version: u32,
    p: &mut Problems,
) -> Vec<Distribution> {
    let single = map.get("distribution");
    let many = map.get("distributions");
    // Distinguish "absent" from "present-but-null": a bare `distribution:` /
    // `distributions:` (null value) reads as absent, exactly like the sibling keys.
    let single_present = matches!(single, Some(v) if !v.is_null());
    let many_present = matches!(many, Some(v) if !v.is_null());
    if single_present && many_present {
        p.err(
            "declare either `distribution` (one block) or `distributions` (a list), not both — \
             they are the singular and plural spellings of the same field"
                .to_string(),
        );
        // Fall through parsing the plural so the rest of the pass still surfaces
        // problems; the document is never emitted while `errors` is non-empty.
    }

    let distributions = match (single, many) {
        // `distributions:` — a sequence of mappings (the monorepo shape). Wins
        // over a stray singular key (already flagged above).
        (_, Some(Value::Sequence(seq))) => {
            let mut out = Vec::with_capacity(seq.len());
            for (idx, item) in seq.iter().enumerate() {
                match item {
                    Value::Mapping(m) => {
                        out.push(parse_one_distribution(m, schema_version, p));
                    }
                    _ => p.err(format!(
                        "distributions[{idx}] must be a mapping with {{package, adapter, \
                         gh_releases?, installers?, homebrew_tap?, platforms?}}"
                    )),
                }
            }
            out
        }
        (_, Some(v)) if !v.is_null() => {
            p.err(format!(
                "distributions must be a list of distribution mappings, got {}",
                yaml_display(v)
            ));
            Vec::new()
        }
        // `distribution:` — a single mapping (v1 back-compat) → a one-element list.
        (Some(Value::Mapping(m)), _) => {
            vec![parse_one_distribution(m, schema_version, p)]
        }
        (Some(v), _) if !v.is_null() => {
            p.err(
                "distribution must be a mapping with {adapter?, gh_releases?, installers?, \
                 homebrew_tap?, platforms?} (or use `distributions:` for a list)"
                    .to_string(),
            );
            Vec::new()
        }
        // Neither key (or both null) → a registry-only repo.
        _ => Vec::new(),
    };

    // Collection floor: a monorepo (≥2 distributions) must tag each entry with a
    // non-null, UNIQUE `package` — otherwise its distributions are
    // indistinguishable and the association is meaningless. A single distribution
    // may leave `package` null (the bare `distribution:` back-compat case).
    if distributions.len() >= 2 {
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (idx, d) in distributions.iter().enumerate() {
            match d.package.as_deref() {
                None => p.err(format!(
                    "floor: distributions[{idx}] has no `package` — with two or more \
                     distributions each must name the package it builds (the monorepo \
                     association key), so they can be told apart"
                )),
                Some(pkg) if !seen.insert(pkg) => p.err(format!(
                    "floor: distributions[{idx}].package {} is used by more than one \
                     distribution — each distribution must name a distinct package",
                    quote_for_diagnostic(pkg)
                )),
                Some(_) => {}
            }
        }

        // Typo guard (advisory): once a monorepo names packages, a distribution
        // whose `package` matches NO `targets[].package` is very likely a typo — a
        // distribution should build a package the contract also tracks as a target.
        // A warning, not a floor: a binary-only package legitimately need not appear
        // in the registry `targets`, and `targets` may be empty (a version-tracked,
        // unpublished repo) — so it fires only when there ARE named target packages
        // to compare against.
        let target_pkgs: std::collections::BTreeSet<&str> = targets
            .iter()
            .filter_map(|t| t.package.as_deref())
            .collect();
        if !target_pkgs.is_empty() {
            for (idx, d) in distributions.iter().enumerate() {
                if let Some(pkg) = d.package.as_deref() {
                    if !target_pkgs.contains(pkg) {
                        p.warn(format!(
                            "distributions[{idx}].package {} matches no targets[].package \
                             ({target_pkgs:?}) — likely a typo; a distribution should build a \
                             package the contract also lists as a target",
                            quote_for_diagnostic(pkg)
                        ));
                    }
                }
            }
        }
    }

    distributions
}

/// Parse ONE distribution mapping (an element of `distributions`, or the sole
/// `distribution:` block) into a [`Distribution`]. On the error path it records
/// problems and returns a placeholder — the document is never emitted while
/// `problems.errors` is non-empty.
#[allow(clippy::too_many_lines)]
fn parse_one_distribution(m: &Mapping, schema_version: u32, p: &mut Problems) -> Distribution {
    // package — the monorepo association key. Optional (null for the sole/bare
    // distribution); the collection-level floor in `parse_distributions` requires
    // it once there are two or more distributions.
    let package = match m.get("package") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_str() {
            // Store the TRIMMED value: surrounding whitespace would otherwise make
            // `" alpha"` and `"alpha"` distinct to the uniqueness floor and to the
            // per-package association/audit keying, silently breaking both.
            Some(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => {
                p.err(
                    "distribution.package must be a non-empty string (the package this \
                     distribution builds)"
                        .to_string(),
                );
                None
            }
        },
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
                    "distribution.homebrew_tap {} invalid — must be an 'owner/repo' slug",
                    quote_for_diagnostic(s)
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
    // This is a PER-BLOCK check — cargo-dist pushes the formula to the tap
    // configured in this same distribution, so the tap must live here, not in a
    // sibling distribution. (The target-side missing-tap floor, the double-publish
    // collision, and the dead-tap advisory are cross-field and aggregate over all
    // distributions + targets — they live in [`check_homebrew_configuration`].)
    if wants_homebrew && homebrew_tap.is_none() {
        p.err(
            "floor: distribution.installers includes 'homebrew' but no distribution.homebrew_tap \
             is set — the generated formula has nowhere to be pushed"
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
                        "distribution.platforms: {} is not a well-formed target-triple \
                         (e.g. x86_64-unknown-linux-musl, aarch64-apple-darwin) — structural \
                         check only; the toolchain is the final authority on what builds",
                        quote_for_diagnostic(s)
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

    Distribution {
        package,
        adapter,
        gh_releases,
        installers,
        homebrew_tap,
        platforms,
        extra_fields,
    }
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

/// Floor a package declared with **two different crates.io publishers** — one
/// engine-run (`cargo-publish`) and one CI-delegated (`cargo-publish-ci`).
///
/// The two identities are mutually exclusive statements about who runs `cargo
/// publish` for that crate, and the per-target journal id keeps them distinct, so
/// both would plan: the engine would publish the crate in publish-all AND the tag
/// push would trigger CI to publish it again. crates.io rejects the duplicate, so
/// the damage is a red CI job rather than a corrupted release — but the contract is
/// self-contradictory and the cut would report green over a failed workflow. Only
/// reachable since `cargo-publish-ci` exists, hence floored with it.
fn check_publisher_conflicts(targets: &[Target], p: &mut Problems) {
    for target in targets
        .iter()
        .filter(|t| t.adapter == Adapter::CargoPublishCi)
    {
        let Some(package) = target.package.as_deref() else {
            continue;
        };
        if targets.iter().any(|other| {
            other.adapter == Adapter::CargoPublish
                && other.registry == target.registry
                && other.package.as_deref() == Some(package)
        }) {
            p.err(format!(
                "floor: package {} is declared for {} twice, once with adapter 'cargo-publish' \
                 (the engine publishes it) and once with 'cargo-publish-ci' (CI publishes it) — \
                 the engine would publish it AND the tag would trigger CI to publish it again. \
                 Keep exactly one publisher for a package",
                quote_for_diagnostic(package),
                quote_for_diagnostic(target.registry.as_str())
            ));
        }
    }
}

/// Cross-read the repo's Cargo manifests as evidence for (or against) what the
/// contract says about publishing to crates.io.
///
/// The contract is the authority on *intent*; `Cargo.toml`'s `publish` key is the
/// authority on what Cargo will actually permit. Where the two disagree the
/// normalized contract would assert a publish surface that cannot exist (or omit
/// one nothing prevents), which is exactly the silent-wrong-result the machine
/// contract must not carry, so each direction is diagnosed here:
///
/// - **Contradiction (hard error).** A declared crates.io target whose crate
///   [forbids](CargoPublishPolicy::Forbidden) publishing can never publish. Refusing
///   at normalization keeps that failure *before* the irreversible tag: the
///   engine-run form would die in `cargo publish`, and the CI-delegated form
///   (`cargo-publish-ci`) is worse — publish-all skips it, so the first symptom
///   would be a red workflow after the tag, with verify then reporting the crate
///   Missing at its destination.
/// - **Unguarded publish-none (warning).** A contract with **no publish targets at
///   all** is the authored publish-none shape (an explicit `targets: []`; the
///   normalizer never expands an empty list back into a phantom target). That is a
///   valid, honored state — but if nothing in the tree forbids publishing, the
///   declaration rests on the contract alone and a stray `cargo publish` would still
///   succeed. Non-fatal: the contract is the truth, this only names the missing
///   belt-and-braces.
///
/// The warning deliberately keys on `targets` being **entirely** empty, not on the
/// absence of a *crates.io* target: a repo that ships binaries through a
/// `gh-releases`/`homebrew` target publishes plenty and is not publish-none, so
/// telling it to set `publish = false` would be both wrong and (for a cargo-dist
/// repo) actively bad advice.
///
/// Evidence-gated in every direction. No readable `Cargo.toml`, a target naming a
/// package no manifest declares, or a manifest whose `publish` key the reader could
/// not resolve ([`Unknown`](CargoPublishPolicy::Unknown) — inheritance with no
/// `[workspace.package]`, an inline table) all yield **no** diagnostic: absence of
/// evidence is never evidence, in either direction. Only an explicit `Forbidden`
/// errors and only an explicit `Allowed` warns.
///
/// Scoped to rust/crates.io on purpose: it is the one ecosystem whose manifest
/// carries a machine-readable publish veto. A non-crates.io registry target is
/// governed by its own allow-list semantics, not by this key.
fn check_cargo_publish_evidence(
    ecosystems: &[Ecosystem],
    targets: &[Target],
    repo_root: &Path,
    fs: &dyn Fs,
    p: &mut Problems,
) {
    if !ecosystems.contains(&Ecosystem::Rust) {
        return;
    }
    let evidence = crate::facts::cargo_publish_evidence(repo_root, fs);
    if evidence.is_empty() {
        return;
    }

    // Publish-none: NO target of any kind. Distinct from CI-DELEGATION (a
    // `cargo-publish-ci` target still publishes, just from CI) and from a
    // binary-only repo (a gh-releases/homebrew target publishes too) — neither
    // reaches this branch.
    if targets.is_empty() {
        let unguarded: Vec<String> = evidence
            .iter()
            .filter(|m| m.policy == CargoPublishPolicy::Allowed)
            .map(|m| quote_for_diagnostic(&m.manifest))
            .collect();
        if !unguarded.is_empty() {
            p.warn(format!(
                "the contract declares no publish targets (publish-none), but {} {} not forbid \
                 publishing — the intent holds in the contract, yet nothing in the tree stops an \
                 accidental 'cargo publish'. Set publish = false to make it enforceable",
                unguarded.join(", "),
                if unguarded.len() == 1 { "does" } else { "do" }
            ));
        }
        return;
    }

    for target in targets.iter().filter(|t| {
        t.ecosystem == Ecosystem::Rust
            && t.registry == Registry::CratesIo
            && matches!(t.adapter, Adapter::CargoPublish | Adapter::CargoPublishCi)
    }) {
        let forbidden =
            |m: &&crate::facts::CargoPublishFlag| m.policy == CargoPublishPolicy::Forbidden;
        // An unresolved (`null`) package cannot be matched to one manifest, so it is
        // contradicted only when EVERY manifest in the tree forbids the publish (a
        // single `Unknown` or `Allowed` is enough to withhold the verdict). The
        // message then names them all — an arbitrary "first" would send the author to
        // one of several equally-blocking manifests.
        let blocking: Vec<&crate::facts::CargoPublishFlag> = match target.package.as_deref() {
            Some(package) => evidence
                .iter()
                .filter(|m| m.package.as_deref() == Some(package))
                .filter(forbidden)
                .collect(),
            None if evidence.iter().all(|m| forbidden(&m)) => evidence.iter().collect(),
            None => Vec::new(),
        };
        if blocking.is_empty() {
            continue;
        }
        p.err(format!(
            "floor: targets declares a crates.io publish for {} with adapter {}, but {} \
             forbids publishing ('publish = false', 'publish = []', or an allow-list without \
             'crates-io') — the publish can never succeed. Drop the target (an explicit \
             'targets: []' is the publish-none contract) or allow the publish in the manifest. \
             Run 'ossctl facts --json' from the repository root and inspect \
             data.cargo_publish to see the manifest evidence ossctl read.",
            quote_for_diagnostic(target.package.as_deref().unwrap_or("this repo's crate")),
            target.adapter.as_str(),
            blocking
                .iter()
                .map(|m| quote_for_diagnostic(&m.manifest))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
}

/// Cross-field Homebrew consistency — the full truth table over three aggregate
/// signals: a **tap** configured on any distribution, an installer-side formula
/// **producer** (a `homebrew` installer on any distribution — cargo-dist), and a
/// target-side formula **producer** (a `homebrew`-registry target whose adapter is
/// `homebrew-tap`, i.e. the release engine pushes a formula to the personal tap).
/// A `cargo-dist` Homebrew target is deliberately absent from this signal: its
/// tag-triggered CI writes the formula, so it is declared and verified but never an
/// engine-side writer.
///
/// A `homebrew-core` target is deliberately NOT a tap-producer: it bumps the
/// central formula via a PR and needs no personal tap, so it neither requires a
/// `homebrew_tap` nor collides with the installer's tap push.
///
/// The floors (all hard errors, per the AI-first fail-fast contract):
/// - **missing-tap (target side):** a `homebrew-tap` target with no `homebrew_tap`
///   anywhere — the engine's `dist` phase has nowhere to push the formula. (The
///   installer side is floored per-block in [`parse_one_distribution`].)
/// - **double-publish:** a `homebrew` installer AND a `homebrew-tap` target both
///   generate + push a formula to the personal tap — a guaranteed collision.
///
/// A configured tap with no producer is a **floor**, not an advisory: a successful
/// release would otherwise omit the Homebrew channel. The engine cannot safely
/// synthesize a target because a distribution block may not identify which package
/// supplies the formula (especially in a multi-crate workspace), so the contract
/// must name the target explicitly.
///
/// **Why aggregate, not per-package (deliberate).** The three signals are OR-ed
/// across all distributions/targets rather than grouped by the monorepo `package`
/// key. This matches the release engine's actual homebrew model: a cut carries a
/// SINGLE tap (`ReleasePlan::homebrew_tap` is the first-found tap, see
/// `release::plan`), and the CLI's `ensure_single_distribution` rejects a
/// multi-distribution monorepo BEFORE it can be planned — a per-package multi-tap
/// monorepo is an explicit deferred follow-up, not a shape the engine can cut. For
/// everything the engine supports (≤1 distribution), aggregate == per-package, and
/// crucially the aggregate view is what lets a bare `package: null` distribution's
/// tap serve a named-package `homebrew-tap` target (ossctl's OWN contract shape) —
/// a strict `target.package == distribution.package` grouping would wrongly reject
/// it. Revisit this only alongside the engine's per-package-tap follow-up.
fn check_homebrew_configuration(
    targets: &[Target],
    distributions: &[Distribution],
    p: &mut Problems,
) {
    let has_tap = distributions.iter().any(|d| d.homebrew_tap.is_some());
    let installer_producer = distributions
        .iter()
        .any(|d| d.installers.contains(&Installer::Homebrew));
    // Narrowed to `homebrew-tap` (not merely `registry == homebrew`): only the tap
    // adapter pushes a formula to the personal tap. `validate_targets` already
    // floors any other adapter on a `homebrew` registry, so a `homebrew-core`
    // target is the only other well-formed case, and it is not a tap-producer.
    let tap_target_producer = targets
        .iter()
        .any(|t| t.registry == Registry::Homebrew && t.adapter == Adapter::HomebrewTap);
    // cargo-dist does not write from the engine, but its CI job and the mandatory
    // observer both need the contract's tap destination. A target without it would
    // only fail later in verify as unobservable, so reject it at normalization.
    let tap_target_needing_tap = targets.iter().any(|t| {
        t.registry == Registry::Homebrew
            && matches!(t.adapter, Adapter::HomebrewTap | Adapter::CargoDist)
    });

    // Floor: a personal-tap target needs a declared tap destination, whether its
    // formula writer is the engine or cargo-dist CI.
    if tap_target_needing_tap && !has_tap {
        p.err(
            "floor: a 'homebrew'-registry target with adapter 'homebrew-tap' or 'cargo-dist' \
             needs distribution.homebrew_tap — the formula has nowhere to be published or \
             observed (set distribution.homebrew_tap to the 'owner/repo' tap)"
                .to_string(),
        );
    }

    let ci_delegated_tap_target = targets
        .iter()
        .any(|t| t.registry == Registry::Homebrew && t.adapter == Adapter::CargoDist);
    // cargo-dist publishes a tap formula only through its Homebrew installer job.
    // Reject an otherwise-declared target that CI can never realize rather than
    // letting mandatory verification time out after the tag has been pushed.
    if ci_delegated_tap_target && !installer_producer {
        p.err(
            "floor: a 'homebrew'-registry target with adapter 'cargo-dist' requires \
             distribution.installers to include 'homebrew' — cargo-dist only writes the formula \
             through its Homebrew installer job"
                .to_string(),
        );
    }

    // Floor: the double-publish collision — two mechanisms push a formula to the
    // personal tap (cargo-dist's installer AND the engine's homebrew-tap adapter).
    // A cargo-dist Homebrew target names that same CI writer, so it does not collide.
    if installer_producer && tap_target_producer {
        p.err(
            "floor: both a 'homebrew' installer (distribution.installers) and a 'homebrew'-registry \
             target with adapter 'homebrew-tap' generate + push a formula to the tap — they would \
             collide; keep exactly one homebrew formula producer, not both"
                .to_string(),
        );
    }

    // A tap without an engine-owned target is diagnosed by the facts-to-contract
    // advisory after this structural pass. It remains a warning so validation and
    // planning can explain the exact missing release surface before `cut` refuses.
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
        Value::String(s) => quote_for_diagnostic(s),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
        Value::Sequence(_) => "<list>".to_string(),
        Value::Mapping(_) => "<map>".to_string(),
        Value::Tagged(t) => yaml_display(&t.value),
    }
}

/// Quote a user-controlled string for safe embedding in a warning/error message.
///
/// Diagnostics interleave user-controlled text (unknown field keys, rejected enum
/// values, package/tap/path strings) into a single line that lands in the §10
/// error envelope and the JSONL log. Wrapping such a value in bare single quotes
/// (`'{s}'`) lets a value carrying a quote, newline, or control character forge a
/// second diagnostic line or corrupt the log — a log-injection vector. JSON string
/// encoding escapes `"`, `\`, newlines, and C0 control characters (and leaves
/// ordinary text readable), so `foo` renders as `"foo"` and a hostile
/// `a"\ninjected` renders as `"a\"\ninjected"` on one intact line. Infallible:
/// serializing a string to JSON never fails.
fn quote_for_diagnostic(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
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

    /// A path rooted at the actual reserved YAML key, with the distribution
    /// context expressed separately rather than inventing a `distribution
    /// extra_fields` key in the diagnostic.
    fn reserved_extra_fields_path(self, key: &str) -> String {
        let path = format!(
            "reserved 'extra_fields' field {}",
            quote_for_diagnostic(key)
        );
        match self {
            Self::TopLevel => path,
            Self::Distribution => format!("{path} in distribution"),
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
                    let path = format!("{label}extra field {}", quote_for_diagnostic(key));
                    match yaml_to_json(v, &path) {
                        Ok(value) => {
                            extra_fields.insert(key.clone(), value);
                        }
                        Err(error) => p.err(error),
                    }
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
        // sorted. Each key is a user-controlled map key, so JSON-encode it (rather
        // than bare single-quoting) to keep a hostile key from forging a diagnostic
        // line — see [`quote_for_diagnostic`].
        let keys = extra_fields
            .keys()
            .map(|k| quote_for_diagnostic(k))
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
                        let path = scope.reserved_extra_fields_path(key);
                        match yaml_to_json(val, &path) {
                            Ok(value) => {
                                out.insert(key.clone(), value);
                            }
                            Err(error) => p.err(error),
                        }
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
///
/// JSON objects only admit string keys. Rather than coercing an arbitrary YAML
/// mapping key (which can collapse distinct values such as `42` and `"42"`),
/// reject it with the preserved field path. That fail-closed behavior upholds the
/// never-drop guarantee for mapping keys without changing the canonical JSON
/// output shape.
fn yaml_to_json(v: &Value, path: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    match v {
        Value::Null => Ok(J::Null),
        Value::Bool(b) => Ok(J::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(J::from(i))
            } else if let Some(u) = n.as_u64() {
                Ok(J::from(u))
            } else if let Some(f) = n.as_f64() {
                Ok(serde_json::Number::from_f64(f).map_or(J::Null, J::Number))
            } else {
                Ok(J::Null)
            }
        }
        Value::String(s) => Ok(J::String(s.clone())),
        Value::Sequence(seq) => seq
            .iter()
            .enumerate()
            .map(|(index, value)| yaml_to_json(value, &format!("{path}[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(J::Array),
        Value::Mapping(m) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in m {
                let Value::String(key) = k else {
                    return Err(format!(
                        "preserved content at {path} has non-string mapping key {}; \
                         canonical JSON object keys must be strings to preserve every value. \
                         Quote the key in YAML or replace it with a string key",
                        yaml_display(k)
                    ));
                };
                let child_path = format!("{path}[{}]", quote_for_diagnostic(key));
                obj.insert(key.clone(), yaml_to_json(val, &child_path)?);
            }
            Ok(J::Object(obj))
        }
        Value::Tagged(t) => yaml_to_json(&t.value, path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A fake `Fs` for filesystem-dependent normalization checks.
    struct FakeFs {
        dirs: HashSet<PathBuf>,
        files: HashMap<PathBuf, Vec<u8>>,
    }

    impl FakeFs {
        fn empty() -> Self {
            Self {
                dirs: HashSet::new(),
                files: HashMap::new(),
            }
        }

        fn with_dirs<const N: usize>(dirs: [&str; N]) -> Self {
            Self {
                dirs: dirs.iter().map(PathBuf::from).collect(),
                files: HashMap::new(),
            }
        }

        fn with_file(path: &str, content: &str) -> Self {
            Self {
                dirs: HashSet::new(),
                files: HashMap::from([(PathBuf::from(path), content.as_bytes().to_vec())]),
            }
        }

        fn with_files<const N: usize>(files: [(&str, &str); N]) -> Self {
            Self {
                dirs: HashSet::new(),
                files: files
                    .iter()
                    .map(|(p, c)| (PathBuf::from(p), c.as_bytes().to_vec()))
                    .collect(),
            }
        }
    }

    impl Fs for FakeFs {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn exists(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }
        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.contains_key(path)
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
        // Pinned to the literal (not KNOWN_SCHEMA_VERSION) so a future bump is an
        // explicit, visible test change rather than silently tracking the constant.
        assert_eq!(c.schema_version, 2);
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
        assert_eq!(c.release.bump_hook, None); // optional, absent by default
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
    fn parses_a_declared_bump_hook() {
        // `release.bump_hook` (facet 3) — the command the engine runs during the bump
        // phase to regenerate version-embedding artifacts (e.g. insta snapshots).
        let c = norm(
            "---\nstatus: approved\nmaturity: mvp\n\
             release:\n  model: gated\n  bump_hook: \"cargo insta test --accept\"\n---\n",
        )
        .contract;
        assert_eq!(
            c.release.bump_hook.as_deref(),
            Some("cargo insta test --accept")
        );
        // Additive: it round-trips through the canonical JSON when present.
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(
            json["release"]["bump_hook"],
            serde_json::json!("cargo insta test --accept")
        );
    }

    #[test]
    fn an_absent_bump_hook_is_omitted_from_canonical_json() {
        // The additive superset guarantee: a contract with no hook serializes exactly
        // as before — no `bump_hook` key in the release block.
        let c = norm(MINIMAL).contract;
        let json = serde_json::to_value(&c).unwrap();
        assert!(
            json["release"].get("bump_hook").is_none(),
            "an absent hook must not appear in canonical JSON, got {:?}",
            json["release"]
        );
    }

    #[test]
    fn an_empty_bump_hook_is_rejected() {
        // A present-but-empty command is a configuration error (fail closed), not a
        // silently-ignored no-op.
        assert_error_contains(
            &norm(
                "---\nstatus: approved\nmaturity: mvp\n\
                 release:\n  model: gated\n  bump_hook: \"   \"\n---\n",
            ),
            "release.bump_hook must be a non-empty",
        );
    }

    #[test]
    fn a_non_string_bump_hook_is_rejected() {
        assert_error_contains(
            &norm(
                "---\nstatus: approved\nmaturity: mvp\n\
                 release:\n  model: gated\n  bump_hook: [not, a, string]\n---\n",
            ),
            "release.bump_hook must be a command string",
        );
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

    /// Cross-field: an explicit empty `targets: []` skips the registry-license
    /// floor (no target → no registry that requires an SPDX license), while a
    /// genuinely invalid license is still caught by its OWN check. Locks in that
    /// the `!targets.is_empty()` gate on the floor keeps honoring an empty set.
    #[test]
    fn explicit_empty_targets_skips_registry_license_floor() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n\
             license: not-a-real-spdx-id\n---\n",
        );
        // The bad license is still invalid on its own …
        assert!(!n.is_valid());
        // … but the registry-requires-license FLOOR must NOT fire — there is no
        // registry target to trigger it.
        assert!(
            !n.problems
                .errors
                .iter()
                .any(|e| e.contains("floor: a target has a registry")),
            "registry-license floor fired despite empty targets: {:?}",
            n.problems.errors
        );
    }

    /// Cross-field: forcing a `registry` health badge while declaring `targets: []`
    /// is a floor error — the badge has no producer (no registry to publish to).
    /// The empty set is honored, and the badge/target consistency floor still
    /// guards against a badge with nothing behind it.
    #[test]
    fn registry_badge_with_explicit_empty_targets_fails() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n\
             health_badges: [registry, license]\n---\n",
        );
        assert_error_contains(&n, "health_badge 'registry' has no producer");
    }

    /// The expansion-skip is independent of the `ecosystems` list: an explicit
    /// `targets: []` with NO ecosystems is still an honored empty set (and, like
    /// the minimal contract, defaults its badges to [ci, license] — no registry
    /// badge without a target).
    #[test]
    fn explicit_empty_targets_with_no_ecosystems() {
        let n = norm("---\nstatus: approved\nmaturity: mvp\ntargets: []\n---\n");
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(n.contract.targets.is_empty());
        assert_eq!(
            n.contract.health_badges,
            vec![HealthBadge::Ci, HealthBadge::License]
        );
    }

    // ── Cargo `publish` cross-read (publish-none supporting evidence) ────────

    const PUBLISH_NONE: &str =
        "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n---\n";
    const RUST_DEFAULT: &str = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n";

    /// The intactl/intakectl case end to end: a private service whose `Cargo.toml`
    /// sets `publish = false` and whose contract declares `targets: []`. The manifest
    /// CONFIRMS the declaration, so the contract normalizes clean and silent — no
    /// phantom crates.io target, no error, and no warning.
    #[test]
    fn publish_none_confirmed_by_publish_false_normalizes_silently() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"intakectl\"\nversion = \"0.1.0\"\npublish = false\n",
        );
        let n = norm_with(PUBLISH_NONE, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(n.contract.targets.is_empty());
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "publish = false is the supporting evidence — nothing to warn about: {:?}",
            n.problems.warnings
        );
    }

    /// Publish-none declared in the contract but NOT backed by the manifest: the
    /// contract is still honored (valid, empty target set), and a warning names the
    /// manifest that leaves an accidental `cargo publish` possible.
    #[test]
    fn publish_none_without_publish_false_warns_with_the_manifest_path() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"intakectl\"\nversion = \"0.1.0\"\n",
        );
        let n = norm_with(PUBLISH_NONE, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(n.contract.targets.is_empty());
        let warning = n
            .problems
            .warnings
            .iter()
            .find(|w| w.contains("publish-none"))
            .unwrap_or_else(|| panic!("no publish-none warning in {:?}", n.problems.warnings));
        assert!(warning.contains("Cargo.toml"), "warning was {warning:?}");
    }

    /// The contradiction, via the DEFAULT expansion: a repo that never publishes but
    /// omits `targets` gets the ecosystem-default crates.io target — which its
    /// `publish = false` manifest can never satisfy. That is the phantom target the
    /// issue is about, and it is now a hard error pointing at `targets: []`.
    #[test]
    fn expanded_crates_io_target_contradicted_by_publish_false_is_an_error() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"intakectl\"\nversion = \"0.1.0\"\npublish = false\n",
        );
        let n = norm_with(RUST_DEFAULT, &fs);
        assert_error_contains(&n, "forbids publishing");
        assert!(
            n.problems.errors.iter().any(|e| e.contains("targets: []")),
            "the error must name the publish-none escape: {:?}",
            n.problems.errors
        );
        assert!(
            n.problems
                .errors
                .iter()
                .any(|e| e.contains("ossctl facts --json") && e.contains("data.cargo_publish")),
            "the error must point to the inspectable evidence: {:?}",
            n.problems.errors
        );
    }

    /// A NAMED target is contradicted by its own member manifest, and only by that
    /// one: the workspace's other, publishable crate does not rescue it.
    #[test]
    fn named_crates_io_target_contradicted_by_its_member_manifest_is_an_error() {
        let fs = FakeFs::with_files([
            (
                "/repo/Cargo.toml",
                "[workspace]\nmembers = [\"a\", \"b\"]\n",
            ),
            (
                "/repo/a/Cargo.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\n",
            ),
            (
                "/repo/b/Cargo.toml",
                "[package]\nname = \"b\"\nversion = \"1.0.0\"\npublish = false\n",
            ),
        ]);
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n\
                    targets:\n  - ecosystem: rust\n    package: b\n    registry: crates.io\n---\n";
        let n = norm_with(text, &fs);
        assert_error_contains(&n, "b/Cargo.toml");

        // The publishable sibling declares cleanly.
        let text_a = text.replace("package: b", "package: a");
        let n_a = norm_with(&text_a, &fs);
        assert!(n_a.is_valid(), "errors: {:?}", n_a.problems.errors);
    }

    /// The CI-DELEGATED publish is not publish-none: `cargo-publish-ci` still puts the
    /// crate on crates.io (CI runs `cargo publish` on the tag), so `publish = false`
    /// contradicts it exactly as it contradicts the engine-run form — and it must never
    /// be mistaken for the no-target case, whose warning would be nonsense here.
    #[test]
    fn ci_delegated_crates_io_target_is_contradicted_by_publish_false_too() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"1.0.0\"\npublish = false\n",
        );
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
                    - ecosystem: rust\n    package: tool\n    registry: crates.io\n    \
                    adapter: cargo-publish-ci\n---\n";
        let n = norm_with(text, &fs);
        assert_error_contains(&n, "forbids publishing");
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "a delegated publish is not publish-none: {:?}",
            n.problems.warnings
        );
    }

    /// Publish-none must mean nothing is published by ANYONE: a distribution block
    /// alongside an empty target set is floored, because the engine would cut it as
    /// tag-only while the pushed tag triggers cargo-dist to publish binaries the run
    /// never planned or verified. This floor is what makes an empty target set a
    /// sound publish-none signal for the coordinator.
    #[test]
    fn publish_none_with_a_distribution_block_is_a_floor_error() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets: []\n\
             distribution:\n  adapter: cargo-dist\n  gh_releases: true\n---\n",
        );
        assert_error_contains(&n, "but targets is empty");
    }

    /// The same floor catches the shape that has no ecosystems either — an empty
    /// target set from ANY route is incompatible with a declared binary surface.
    #[test]
    fn a_distribution_block_without_any_target_is_a_floor_error() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\n\
             distribution:\n  adapter: cargo-dist\n  gh_releases: true\n---\n",
        );
        assert_error_contains(&n, "but targets is empty");
    }

    /// A repo that publishes BINARIES but not crates (a `gh-releases`/`cargo-dist`
    /// target) is not publish-none: it must not be told to set `publish = false`,
    /// which would be wrong advice and can break cargo-dist packaging.
    #[test]
    fn a_binary_only_rust_repo_gets_no_publish_none_warning() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"1.0.0\"\n",
        );
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
                    - ecosystem: rust\n    package: tool\n    registry: gh-releases\n    \
                    adapter: cargo-dist\n---\ndistribution:\n";
        let n = norm_with(text, &fs);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "a binary-publishing repo was called publish-none: {:?}",
            n.problems.warnings
        );
    }

    /// A manifest whose `publish` the reader could not resolve is NOT evidence.
    /// `publish.workspace = true` with no `[workspace.package]` to inherit from is
    /// `Unknown`: it neither contradicts a declared target nor counts as "unguarded".
    #[test]
    fn an_unresolvable_publish_key_produces_no_diagnostic_in_either_direction() {
        let fs = FakeFs::with_files([
            ("/repo/Cargo.toml", "[workspace]\nmembers = [\"a\"]\n"),
            (
                "/repo/a/Cargo.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\npublish.workspace = true\n",
            ),
        ]);
        // Declared target: not contradicted (Unknown is not Forbidden).
        let declared = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
                        - ecosystem: rust\n    package: a\n    registry: crates.io\n---\n";
        let n = norm_with(declared, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        // Publish-none: not warned about (Unknown is not Allowed).
        let n_none = norm_with(PUBLISH_NONE, &fs);
        assert!(
            !n_none
                .problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "an unresolved publish key was reported as unguarded: {:?}",
            n_none.problems.warnings
        );
    }

    /// Inherited `publish = false` (the modern single-place layout) IS resolved, so
    /// a publish-none repo using it is confirmed rather than falsely warned.
    #[test]
    fn inherited_publish_false_is_supporting_evidence_for_publish_none() {
        let fs = FakeFs::with_files([
            (
                "/repo/Cargo.toml",
                "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\npublish = false\n",
            ),
            (
                "/repo/a/Cargo.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\npublish.workspace = true\n",
            ),
        ]);
        let n = norm_with(PUBLISH_NONE, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "inherited publish = false was not read as evidence: {:?}",
            n.problems.warnings
        );
    }

    /// A member's `publish = false` behind a publishable HYBRID root is still seen:
    /// the evidence read covers the root package AND the members.
    #[test]
    fn a_blocked_member_under_a_hybrid_root_still_contradicts_its_target() {
        let fs = FakeFs::with_files([
            (
                "/repo/Cargo.toml",
                "[package]\nname = \"root\"\nversion = \"1.0.0\"\n\n\
                 [workspace]\nmembers = [\"cli\"]\n",
            ),
            (
                "/repo/cli/Cargo.toml",
                "[package]\nname = \"cli\"\nversion = \"1.0.0\"\npublish = false\n",
            ),
        ]);
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
                    - ecosystem: rust\n    package: cli\n    registry: crates.io\n---\n";
        assert_error_contains(&norm_with(text, &fs), "cli/Cargo.toml");
    }

    /// Evidence-gated: no readable `Cargo.toml` means no evidence, so neither
    /// diagnostic fires. Absence of evidence is never evidence of absence — and this
    /// is what keeps every fixture-driven contract (and any non-checkout consumer)
    /// unaffected.
    #[test]
    fn cargo_publish_cross_read_is_silent_without_a_manifest() {
        let n = norm(PUBLISH_NONE);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems.warnings.is_empty(),
            "warnings without evidence: {:?}",
            n.problems.warnings
        );
        assert!(norm(RUST_DEFAULT).is_valid());
    }

    /// A target naming a package no manifest declares is unmatched, not contradicted:
    /// the cross-read stays silent rather than guessing which manifest governs it.
    #[test]
    fn cargo_publish_cross_read_ignores_an_unmatched_package() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"other\"\nversion = \"1.0.0\"\npublish = false\n",
        );
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
                    - ecosystem: rust\n    package: unrelated\n    registry: crates.io\n---\n";
        let n = norm_with(text, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
    }

    /// A non-rust contract is out of scope even with a `publish = false` manifest
    /// lying around: the key governs crates.io, and nothing else.
    #[test]
    fn cargo_publish_cross_read_does_not_touch_a_non_rust_contract() {
        let fs = FakeFs::with_file(
            "/repo/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"1.0.0\"\npublish = false\n",
        );
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [node]\n---\n";
        let n = norm_with(text, &fs);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("publish-none")),
            "warnings: {:?}",
            n.problems.warnings
        );
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
            "distributions",
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
            "warnings",
        ] {
            assert!(json.get(key).is_some(), "missing §4 key {key}");
        }
        assert!(json["versioning_pattern"].is_null());
        // A registry-only contract carries an explicit empty `distributions: []` —
        // the collection is always a JSON array (v2 canonical shape).
        assert_eq!(json["distributions"], serde_json::json!([]));
        // An EMPTY `extra_fields` is OMITTED from canonical JSON (Option A,
        // `skip_serializing_if`): a contract with no unknown keys carries no
        // `extra_fields` key at all. It reappears only when populated — see
        // [`empty_extra_fields_absent_populated_present`].
        assert!(
            json.get("extra_fields").is_none(),
            "empty extra_fields must be absent, got {:?}",
            json.get("extra_fields")
        );
    }

    /// Option A (omit-when-empty), asserted SYMMETRICALLY on both the top-level
    /// [`Contract::extra_fields`] and the nested [`Distribution::extra_fields`]:
    /// an empty map is ABSENT from canonical JSON, a populated map is PRESENT and
    /// byte-for-shape unchanged from before the `skip_serializing_if`.
    #[test]
    fn empty_extra_fields_absent_populated_present() {
        // Empty (both levels): a contract with a distribution but no unknown keys.
        let empty = serde_json::to_value(
            norm(
                "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                 distribution:\n  adapter: cargo-dist\n---\n",
            )
            .contract,
        )
        .unwrap();
        assert!(
            empty.get("extra_fields").is_none(),
            "empty top-level extra_fields must be absent"
        );
        assert!(
            empty["distributions"][0].get("extra_fields").is_none(),
            "empty nested extra_fields must be absent"
        );

        // Populated (both levels): an unknown top-level key and an unknown
        // distribution key are preserved and PRESENT.
        let populated = serde_json::to_value(
            norm(
                "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                 roadmap_url: https://example.com/roadmap\n\
                 distribution:\n  adapter: cargo-dist\n  future_x: 1\n---\n",
            )
            .contract,
        )
        .unwrap();
        assert_eq!(
            populated["extra_fields"]["roadmap_url"],
            "https://example.com/roadmap"
        );
        assert_eq!(populated["distributions"][0]["extra_fields"]["future_x"], 1);
    }

    // ── distribution (cargo-dist binary layer) ───────────────────────────────

    /// A registry-only contract has no distribution: it normalizes clean and
    /// `distributions` is empty.
    #[test]
    fn registry_only_contract_has_no_distribution() {
        let c = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n").contract;
        assert!(c.distributions.is_empty());
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
        let d = c
            .distributions
            .into_iter()
            .next()
            .expect("distribution present");
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
        let d = &json["distributions"][0];
        assert_eq!(d["adapter"], "cargo-dist");
        assert_eq!(d["gh_releases"], true);
        assert_eq!(d["installers"], serde_json::json!(["shell", "homebrew"]));
        assert_eq!(d["homebrew_tap"], "jarimustonen/homebrew-issuectl");
        // A bare (singular) block carries a `null` association key.
        assert!(d["package"].is_null());
    }

    // ── cargo-dist Homebrew drift advisory ───────────────────────────────────

    /// A Homebrew configuration in cargo-dist without the authoritative contract
    /// tap warns: release planning deliberately reads only the contract.
    #[test]
    fn dist_workspace_tap_without_contract_tap_warns() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/homebrew-tool\"\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n",
            &fs,
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("distribution.homebrew_tap")
                    && w.contains("will not be planned")),
            "expected cargo-dist drift warning: {:?}",
            n.problems.warnings
        );
    }

    /// A cargo-dist Homebrew publish job is a real destination and must be modelled
    /// as a delegated target so the mandatory verify barrier observes it.
    #[test]
    fn dist_workspace_homebrew_publish_job_without_target_is_a_floor() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\npublish-jobs = [\"homebrew\"]\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n",
            &fs,
        );
        assert_error_contains(&n, "no delegated Homebrew target");
    }

    /// A distribution without its own tap still warns when cargo-dist configures one.
    #[test]
    fn dist_workspace_tap_with_distribution_lacking_tap_warns() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/homebrew-tool\"\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ndistribution:\n  \
             adapter: cargo-dist\n  installers: [shell]\n---\n",
            &fs,
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|w| w.contains("distribution.homebrew_tap")),
            "expected cargo-dist drift warning: {:?}",
            n.problems.warnings
        );
    }

    #[test]
    fn dist_workspace_homebrew_publish_job_refuses_an_engine_owned_tap_target() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/homebrew-tool\"\npublish-jobs = [\"homebrew\"]\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
             - {ecosystem: rust, package: tool, registry: gh-releases, adapter: cargo-dist}\n  \
             - {ecosystem: rust, package: tool, registry: homebrew, adapter: homebrew-tap}\n\
             distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  \
             homebrew_tap: owner/homebrew-tool\n---\n",
            &fs,
        );
        assert_error_contains(&n, "cargo-dist CI and ossctl would both write the same tap");
    }

    #[test]
    fn dist_workspace_homebrew_publish_job_accepts_a_delegated_tap_target() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/homebrew-tool\"\npublish-jobs = [\"homebrew\"]\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
             - {ecosystem: rust, package: tool, registry: gh-releases, adapter: cargo-dist}\n  \
             - {ecosystem: rust, package: tool, registry: homebrew, adapter: cargo-dist}\n\
             distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
             homebrew_tap: owner/homebrew-tool\n---\n",
            &fs,
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
    }

    /// When both configs declare the tap but cargo-dist does not publish it, the
    /// engine-owned target remains valid (ossctl's own release shape).
    #[test]
    fn dist_workspace_tap_matching_contract_is_silent() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/homebrew-tool\"\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\ntargets:\n  \
             - {ecosystem: rust, package: tool, registry: crates.io, adapter: cargo-publish}\n  \
             - {ecosystem: rust, package: tool, registry: gh-releases, adapter: cargo-dist}\n  \
             - {ecosystem: rust, package: tool, registry: homebrew, adapter: homebrew-tap}\n\
             distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  homebrew_tap: owner/homebrew-tool\n---\n",
            &fs,
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems.warnings.is_empty(),
            "warnings: {:?}",
            n.problems.warnings
        );
    }

    #[test]
    fn ci_delegated_homebrew_rejects_a_dist_workspace_tap_mismatch() {
        let fs = FakeFs::with_file(
            "/repo/dist-workspace.toml",
            "[dist]\ntap = \"owner/actual-tap\"\npublish-jobs = [\"homebrew\"]\n",
        );
        let n = norm_with(
            "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
             - {ecosystem: rust, package: tool, registry: homebrew, adapter: cargo-dist}\n\
             distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
             homebrew_tap: owner/declared-tap\n---\n",
            &fs,
        );
        assert_error_contains(
            &n,
            "cargo-dist would write one tap while ossctl verifies another",
        );
    }

    /// An absent cargo-dist config does not add a warning.
    #[test]
    fn absent_dist_workspace_is_silent() {
        let n = norm("---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n");
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems.warnings.is_empty(),
            "warnings: {:?}",
            n.problems.warnings
        );
    }

    /// A malformed cargo-dist file still proves that distribution infrastructure
    /// exists, so it is advisory rather than fatal but warns when GH Releases is
    /// absent from the contract.
    #[test]
    fn unparseable_dist_workspace_warns_about_missing_gh_releases() {
        let fs = FakeFs::with_file("/repo/dist-workspace.toml", "[dist\ntap = \"owner/tap\"");
        let n = norm_with(
            "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n---\n",
            &fs,
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems
                .warnings
                .iter()
                .any(|warning| warning.contains("no 'gh-releases' target")),
            "warnings: {:?}",
            n.problems.warnings
        );
    }

    // ── homebrew cross-field consistency floors (truth table) ────────────────

    /// Build a production contract exercising the three homebrew signals: a
    /// configured `tap`, an `installer` producer (a `homebrew` installer), and a
    /// `tap_target` producer (a `homebrew`-registry target with adapter
    /// `homebrew-tap`). A crates.io target is always present so the contract has a
    /// licensed publishable target.
    fn hb_case(tap: bool, installer: bool, tap_target: bool) -> String {
        let mut fm = String::from(
            "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
             - {ecosystem: rust, package: ossctl, registry: crates.io, adapter: cargo-publish}\n",
        );
        if tap_target {
            fm.push_str(
                "  - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: homebrew-tap}\n",
            );
        }
        // A distribution block exists whenever we need to express an installer
        // producer or a configured tap; otherwise the contract is registry-only.
        if installer || tap {
            fm.push_str("distribution:\n  adapter: cargo-dist\n");
            if installer {
                fm.push_str("  installers: [homebrew]\n");
            } else {
                fm.push_str("  installers: [shell]\n");
            }
            if tap {
                fm.push_str("  homebrew_tap: owner/tap\n");
            }
        }
        fm.push_str("---\n");
        fm
    }

    /// The full 8-row truth table (tap × installer-producer × tap-target-producer).
    /// Each row asserts the accept/reject verdict; the floor/advisory messages are
    /// pinned in the focused tests below.
    #[test]
    fn homebrew_truth_table_all_eight_rows() {
        // (tap, installer, tap_target, expect_valid)
        let rows = [
            (false, false, false, true), // 1: nothing homebrew → clean
            (false, false, true, false), // 2: tap-target, no tap → missing-tap floor
            (false, true, false, false), // 3: installer, no tap → per-block floor
            (false, true, true, false),  // 4: both producers, no tap → floors
            (true, false, false, true),  // 5: tap, no producer → cut-time refusal
            (true, false, true, true),   // 6: tap + tap-target → well-formed (ossctl's case)
            (true, true, false, true),   // 7: tap + installer → well-formed (cargo-dist)
            (true, true, true, false),   // 8: tap + both producers → double-publish floor
        ];
        for (tap, installer, tap_target, expect_valid) in rows {
            let n = norm(&hb_case(tap, installer, tap_target));
            assert_eq!(
                n.is_valid(),
                expect_valid,
                "row (tap={tap}, installer={installer}, tap_target={tap_target}) expected \
                 valid={expect_valid}; errors were {:?}",
                n.problems.errors
            );
        }
    }

    /// Row 2: a `homebrew-tap` target with no tap anywhere is a hard error (the
    /// target-side counterpart of the per-block installer-without-tap floor).
    #[test]
    fn homebrew_tap_target_without_tap_is_a_floor() {
        assert_error_contains(
            &norm(&hb_case(false, false, true)),
            "needs distribution.homebrew_tap",
        );
    }

    #[test]
    fn ci_delegated_homebrew_target_requires_tap_for_ci_and_verify() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: cargo-dist}\n---\n";
        assert_error_contains(&norm(text), "needs distribution.homebrew_tap");
    }

    #[test]
    fn ci_delegated_homebrew_target_requires_cargo_dist_homebrew_installer() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: cargo-dist}\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  \
                    homebrew_tap: owner/tap\n---\n";
        assert_error_contains(
            &norm(text),
            "requires distribution.installers to include 'homebrew'",
        );
    }

    /// Row 8: an installer producer AND a `homebrew-tap` target both push a formula
    /// to the tap — the double-publish collision is a hard error.
    #[test]
    fn homebrew_double_publish_is_a_floor() {
        assert_error_contains(
            &norm(&hb_case(true, true, true)),
            "they would collide; keep exactly one homebrew formula producer",
        );
    }

    /// Regression for `intake-feature-ossctl-73e870268475`: the field-reported
    /// shape had two crates.io targets and a distribution tap, but no Homebrew
    /// target. Validation warns and the cut preflight refuses before it can omit
    /// the tap leg.
    #[test]
    fn distribution_tap_without_formula_producer_warns() {
        let text = concat!(
            "---\n",
            "status: approved\n",
            "maturity: production\n",
            "ecosystems: [rust]\n",
            "targets:\n",
            "  - {ecosystem: rust, package: project-canon-core, registry: crates.io, adapter: cargo-publish}\n",
            "  - {ecosystem: rust, package: project-canon-cli, registry: crates.io, adapter: cargo-publish}\n",
            "distribution:\n",
            "  adapter: cargo-dist\n",
            "  installers: [shell, powershell]\n",
            "  homebrew_tap: owner/homebrew-project-canon\n",
            "---\n",
        );
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(n
            .problems
            .warnings
            .iter()
            .any(|warning| warning.contains("tap leg would be silently skipped")));
    }

    /// Row 6: a `homebrew-tap` target with a configured tap and no installer
    /// producer is the well-formed case (ossctl's own shape) — clean.
    #[test]
    fn homebrew_tap_target_with_tap_is_clean() {
        let n = norm(&hb_case(true, false, true));
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("the tap will never be updated")),
            "unexpected dead-tap advisory: {:?}",
            n.problems.warnings
        );
    }

    /// The repository's own contract already carries the explicit target, so this
    /// safety floor must leave its four-target release path unchanged.
    #[test]
    fn ossctl_contract_keeps_its_four_explicit_targets() {
        let n = norm(include_str!("../../../../OSS-RELEASE.md"));
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert_eq!(n.contract.targets.len(), 4);
        assert!(n.contract.targets.iter().any(|target| {
            target.registry == Registry::Homebrew && target.adapter == Adapter::HomebrewTap
        }));
    }

    /// registry/adapter compatibility: a `homebrew`-registry target with a
    /// non-homebrew adapter (here the ecosystem default via an explicit `manual`)
    /// is a hard error — it has no homebrew formula path.
    #[test]
    fn homebrew_registry_requires_homebrew_adapter() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: manual}\n\
                    distribution:\n  adapter: cargo-dist\n  homebrew_tap: owner/tap\n---\n";
        assert_error_contains(
            &norm(text),
            "requires adapter 'homebrew-tap' (personal tap), 'homebrew-core'",
        );
    }

    /// A CI-delegated cargo-dist target declares a real Homebrew surface without
    /// making the engine a second tap writer.
    #[test]
    fn ci_delegated_homebrew_target_normalizes_and_round_trips() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: cargo-dist}\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [homebrew]\n  \
                    homebrew_tap: owner/tap\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems.warnings.is_empty(),
            "warnings: {:?}",
            n.problems.warnings
        );
        let target = n
            .contract
            .targets
            .iter()
            .find(|target| target.registry == Registry::Homebrew)
            .expect("normalized Homebrew target");
        assert_eq!(target.adapter, Adapter::CargoDist);
        let canonical = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(canonical["targets"][1]["adapter"], "cargo-dist");
        assert_eq!(canonical["targets"][1]["registry"], "homebrew");
    }

    /// A CI-delegated crates.io target (`cargo-publish-ci`) is a first-class,
    /// round-trippable contract shape: the repo whose crates.io publish runs in a
    /// tag-triggered workflow can declare its real publish surface, and the engine
    /// reads the delegation off the adapter identity.
    #[test]
    fn ci_delegated_crates_io_target_normalizes_and_round_trips() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: glasspad, registry: crates.io, adapter: cargo-publish-ci}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            n.problems.warnings.is_empty(),
            "warnings: {:?}",
            n.problems.warnings
        );
        assert_eq!(n.contract.targets[0].adapter, Adapter::CargoPublishCi);
        let canonical = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(canonical["targets"][0]["adapter"], "cargo-publish-ci");
        assert_eq!(canonical["targets"][0]["registry"], "crates.io");
    }

    /// A mixed contract — one engine-published crate, one CI-published crate — is
    /// valid: delegation is a per-target property, not a repo-wide mode.
    #[test]
    fn a_mixed_local_and_ci_publish_contract_is_valid() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: lib, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: cli, registry: crates.io, adapter: cargo-publish-ci}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert_eq!(n.contract.targets[0].adapter, Adapter::CargoPublish);
        assert_eq!(n.contract.targets[1].adapter, Adapter::CargoPublishCi);
    }

    /// One package, two publishers is a floor: the engine would publish it in
    /// publish-all and the tag push would trigger CI to publish it again.
    #[test]
    fn a_package_cannot_be_declared_with_both_publishers() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: tool, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: tool, registry: crates.io, adapter: cargo-publish-ci}\n---\n";
        assert_error_contains(&norm(text), "Keep exactly one publisher for a package");
    }

    /// `cargo-publish-ci` on a non-crates.io registry is a floor: the delegated
    /// publish has no destination the verify barrier knows how to observe, so it
    /// would tag first and only then fail — refuse it while nothing has happened.
    #[test]
    fn ci_delegated_cargo_publish_requires_crates_io() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: tool, registry: gh-releases, adapter: cargo-publish-ci}\n---\n";
        assert_error_contains(
            &norm(text),
            "the CI-delegated cargo publish targets crates.io only",
        );
    }

    /// …and it is a rust adapter: `cargo publish` releases a crate.
    #[test]
    fn ci_delegated_cargo_publish_requires_the_rust_ecosystem() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [node]\ntargets:\n  \
                    - {ecosystem: node, package: tool, registry: crates.io, adapter: cargo-publish-ci}\n---\n";
        assert_error_contains(&norm(text), "`cargo publish` releases a rust crate");
    }

    /// A `homebrew-core` target is a valid homebrew adapter and needs NO personal
    /// tap (it bumps the central formula) — it is neither a missing-tap floor nor a
    /// dead-tap advisory, and does not collide with a `homebrew` installer.
    #[test]
    fn homebrew_core_target_needs_no_tap() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: homebrew-core}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
    }

    /// registry/adapter compat, OMITTED adapter: a `homebrew`-registry target with
    /// no adapter resolves to the ecosystem default (`cargo-publish` for rust),
    /// which is non-homebrew — so it hits the same floor. The normalizer never
    /// registry-defaults a homebrew target to `homebrew-tap` (that would silently
    /// choose personal-tap publication over a homebrew-core PR); the author must
    /// spell the adapter. This locks the omitted-adapter path, not just explicit
    /// `manual`.
    #[test]
    fn homebrew_registry_omitted_adapter_is_a_floor() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew}\n---\n";
        assert_error_contains(
            &norm(text),
            "requires adapter 'homebrew-tap' (personal tap), 'homebrew-core'",
        );
    }

    /// The homebrew cross-field check reads the plural `distributions:` (Vec) path,
    /// not only the singular back-compat mapping: a one-entry `distributions:` list
    /// carrying the tap satisfies a `homebrew-tap` target (row 6 via the Vec shape).
    #[test]
    fn homebrew_tap_target_satisfied_via_plural_distributions() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ntargets:\n  \
                    - {ecosystem: rust, package: ossctl, registry: crates.io, adapter: cargo-publish}\n  \
                    - {ecosystem: rust, package: ossctl, registry: homebrew, adapter: homebrew-tap}\n\
                    distributions:\n  \
                    - {package: ossctl, adapter: cargo-dist, installers: [shell], homebrew_tap: owner/tap}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(
            !n.problems
                .warnings
                .iter()
                .any(|w| w.contains("the tap will never be updated")),
            "unexpected dead-tap advisory: {:?}",
            n.problems.warnings
        );
    }

    // ── monorepo: Vec<Distribution> + per-package association ─────────────────

    /// Back-compat: a bare singular `distribution:` mapping deserializes as a
    /// one-element `distributions` list with a `null` package — the v1 author
    /// changes nothing.
    #[test]
    fn singular_distribution_parses_as_one_element_list() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n---\n";
        let c = norm(text).contract;
        assert_eq!(c.distributions.len(), 1);
        assert_eq!(c.distributions[0].package, None);
        assert_eq!(c.distributions[0].adapter, DistributionAdapter::CargoDist);
    }

    /// A monorepo: a plural `distributions:` sequence, each entry tagged with the
    /// package it builds, parses with the per-package association preserved in
    /// order (each distribution keeps its own installers/tap).
    #[test]
    fn plural_distributions_parse_with_per_package_association() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    targets:\n  - {ecosystem: rust, package: alpha, registry: crates.io}\n  \
                    - {ecosystem: rust, package: beta, registry: crates.io}\n\
                    distributions:\n  - {package: alpha, adapter: cargo-dist, installers: [shell]}\n  \
                    - {package: beta, adapter: cargo-dist, installers: [homebrew], homebrew_tap: owner/tap}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let d = n.contract.distributions;
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].package.as_deref(), Some("alpha"));
        assert_eq!(d[0].installers, vec![Installer::Shell]);
        assert_eq!(d[1].package.as_deref(), Some("beta"));
        assert_eq!(d[1].homebrew_tap.as_deref(), Some("owner/tap"));
    }

    /// Canonical JSON round-trips for BOTH shapes: the emitted `distributions`
    /// array re-feeds as YAML frontmatter and normalizes to the same list — the
    /// single (bare `distribution:`) and the monorepo (`distributions:`) cases.
    #[test]
    fn distributions_canonical_json_round_trip() {
        for text in [
            "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
             distribution:\n  adapter: cargo-dist\n  installers: [shell]\n---\n",
            "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
             targets:\n  - {ecosystem: rust, package: a, registry: crates.io}\n  \
             - {ecosystem: rust, package: b, registry: crates.io}\n\
             distributions:\n  - {package: a, adapter: cargo-dist}\n  \
             - {package: b, adapter: goreleaser}\n---\n",
        ] {
            let first = norm(text).contract;
            assert!(!first.distributions.is_empty());
            // Re-feed the canonical JSON as the frontmatter of a fresh document.
            let json = serde_json::to_value(&first).unwrap();
            let refed = format!("---\n{}---\n", serde_yaml::to_string(&json).unwrap());
            let second = norm(&refed).contract;
            assert_eq!(
                first.distributions, second.distributions,
                "round-trip drift for: {text}"
            );
        }
    }

    /// Declaring BOTH `distribution:` and `distributions:` is ambiguous → error.
    #[test]
    fn both_distribution_keys_is_an_error() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n\
                    distributions:\n  - {package: a, adapter: cargo-dist}\n---\n";
        assert_error_contains(&norm(text), "not both");
    }

    /// A monorepo (≥2 distributions) with an entry missing `package` → floor error
    /// (the entries would be indistinguishable).
    #[test]
    fn multi_distribution_missing_package_is_a_floor_error() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    targets:\n  - {ecosystem: rust, package: a, registry: crates.io}\n\
                    distributions:\n  - {package: a, adapter: cargo-dist}\n  \
                    - {adapter: cargo-dist}\n---\n";
        assert_error_contains(&norm(text), "must name the package it builds");
    }

    /// A monorepo with a duplicate `package` across distributions → floor error.
    #[test]
    fn multi_distribution_duplicate_package_is_a_floor_error() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distributions:\n  - {package: dup, adapter: cargo-dist}\n  \
                    - {package: dup, adapter: goreleaser}\n---\n";
        assert_error_contains(&norm(text), "distinct package");
    }

    /// A v1 document (explicit `schema_version: 1`, singular `distribution:`)
    /// normalizes to the v2 canonical shape AND is re-labeled `schema_version: 2` —
    /// never a v2 body stamped with a v1 number. The tool reads v1, emits v2.
    #[test]
    fn v1_document_is_relabeled_to_current_schema_version_on_emit() {
        let text = "---\nschema_version: 1\nstatus: approved\nmaturity: production\n\
                    ecosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        // Emitted version is the current one, not the declared 1.
        assert_eq!(n.contract.schema_version, 2);
        let json = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(json["schema_version"], 2);
        // …and the shape is the v2 `distributions` array (the singular key parsed).
        assert_eq!(json["distributions"].as_array().map(Vec::len), Some(1));
    }

    /// A whitespace-padded `package` is trimmed before storing — so `"  alpha "`
    /// and `"alpha"` are the SAME package to the uniqueness floor and association,
    /// not two distinct ones that would slip past the dup-check.
    #[test]
    fn distribution_package_is_trimmed() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    distributions:\n  - {package: '  alpha ', adapter: cargo-dist}\n  \
                    - {package: alpha, adapter: goreleaser}\n---\n";
        // The two trimmed packages collide → the duplicate-package floor fires.
        assert_error_contains(&norm(text), "distinct package");
    }

    /// A single distribution MAY carry a `package` (no floor below the ≥2
    /// threshold) — the association key is optional, not forbidden, for one block.
    #[test]
    fn single_distribution_may_carry_a_package() {
        let text = "---\nstatus: approved\nmaturity: production\necosystems: [rust]\n\
                    targets:\n  - {ecosystem: rust, package: solo, registry: crates.io}\n\
                    distributions:\n  - {package: solo, adapter: cargo-dist}\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert_eq!(n.contract.distributions[0].package.as_deref(), Some("solo"));
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
            .distributions
            .into_iter()
            .next()
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
            json["distributions"][0]["extra_fields"]["future_signing"]["enabled"],
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
        let d = n
            .contract
            .distributions
            .into_iter()
            .next()
            .expect("distribution present");
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
        let d = c
            .distributions
            .into_iter()
            .next()
            .expect("distribution present");
        assert_eq!(d.extra_fields.get("future_x"), Some(&serde_json::json!(1)));
        // Two independent forward-compat warnings, each naming schema_version 2
        // (the contract omits schema_version → defaults to KNOWN_SCHEMA_VERSION).
        let fc: Vec<&String> = n
            .problems
            .warnings
            .iter()
            .filter(|w| w.contains("forward-compat") && w.contains("schema_version 2"))
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

    /// Nested non-string YAML keys cannot be represented as canonical JSON object
    /// keys without loss, so a numeric/string collision fails closed instead of
    /// silently overwriting either preserved value.
    #[test]
    fn extra_fields_nested_numeric_string_key_collision_is_rejected() {
        let n =
            norm("---\nstatus: approved\nmaturity: mvp\nfuture_x:\n  42: a\n  \"42\": b\n---\n");
        assert_error_contains(&n, "non-string mapping key 42");
        assert!(
            n.problems
                .errors
                .iter()
                .any(|error| error.contains("extra field \"future_x\"")),
            "error should name the preserved field path: {:?}",
            n.problems.errors
        );
        assert!(
            !n.contract.extra_fields.contains_key("future_x"),
            "invalid preserved content must not be partially captured"
        );
    }

    /// A sequence is legal YAML as a mapping key, but has no lossless JSON-object
    /// key representation. It must therefore fail closed like scalar non-string
    /// keys, with the preserved field path in the diagnostic.
    #[test]
    fn extra_fields_nested_list_key_is_rejected() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\nfuture_x:\n  ? [one, two]\n  : list-key\n---\n",
        );
        assert_error_contains(&n, "non-string mapping key <list>");
        assert!(
            n.problems
                .errors
                .iter()
                .any(|error| error.contains("extra field \"future_x\"")),
            "error should name the preserved field path: {:?}",
            n.problems.errors
        );
    }

    /// A non-string key nested several levels down still names its complete path,
    /// allowing an AI caller to repair the exact mapping rather than hunting
    /// through an arbitrary preserved value.
    #[test]
    fn extra_fields_deeply_nested_non_string_key_reports_path() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\nfuture_x:\n  outer:\n    inner:\n      42: answer\n---\n",
        );
        assert!(
            n.problems.errors.iter().any(|error| {
                error.contains("extra field \"future_x\"[\"outer\"][\"inner\"]")
                    && error.contains("non-string mapping key 42")
            }),
            "error should name the complete nested path: {:?}",
            n.problems.errors
        );
    }

    /// All-string mapping keys retain their existing JSON representation, including
    /// keys nested inside sequences, so the fail-closed guard is shape-neutral for
    /// valid forward-compatible content.
    #[test]
    fn extra_fields_nested_string_keys_round_trip_unchanged() {
        let n = norm(
            "---\nstatus: approved\nmaturity: mvp\nfuture_x:\n  outer:\n    answer: 42\n    items:\n      - name: first\n        enabled: true\n---\n",
        );
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert_eq!(
            n.contract.extra_fields.get("future_x"),
            Some(&serde_json::json!({
                "outer": {
                    "answer": 42,
                    "items": [{"name": "first", "enabled": true}],
                }
            }))
        );
    }

    /// The three capture contexts all propagate nested conversion failures with
    /// their own useful root path, including a sequence index where applicable.
    #[test]
    fn nested_non_string_key_paths_cover_all_capture_contexts() {
        let reserved = norm(
            "---\nstatus: approved\nmaturity: mvp\nextra_fields:\n  future_x:\n    42: answer\n---\n",
        );
        assert!(
            reserved.problems.errors.iter().any(|error| {
                error.contains("reserved 'extra_fields' field \"future_x\"")
                    && error.contains("non-string mapping key 42")
            }),
            "reserved-block path missing: {:?}",
            reserved.problems.errors
        );

        let distribution = norm(
            "---\nstatus: approved\nmaturity: production\necosystems: [rust]\ndistribution:\n  adapter: cargo-dist\n  future_x:\n    42: answer\n---\n",
        );
        assert!(
            distribution.problems.errors.iter().any(|error| {
                error.contains("distribution extra field \"future_x\"")
                    && error.contains("non-string mapping key 42")
            }),
            "distribution path missing: {:?}",
            distribution.problems.errors
        );

        let sequence = norm(
            "---\nstatus: approved\nmaturity: mvp\nfuture_x:\n  - ok: first\n  - 42: answer\n---\n",
        );
        assert!(
            sequence.problems.errors.iter().any(|error| {
                error.contains("extra field \"future_x\"[1]")
                    && error.contains("non-string mapping key 42")
            }),
            "sequence index path missing: {:?}",
            sequence.problems.errors
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
        let d = n
            .contract
            .distributions
            .into_iter()
            .next()
            .expect("distribution present");
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
        let d = norm(text)
            .contract
            .distributions
            .into_iter()
            .next()
            .unwrap();
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
        assert_eq!(
            n.contract
                .distributions
                .into_iter()
                .next()
                .unwrap()
                .homebrew_tap,
            None
        );
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

    /// A `homebrew_tap` without an engine-owned target is a warning at validation
    /// time and a hard refusal at cut time, so the author can see the remediation.
    #[test]
    fn distribution_tap_without_target_warns() {
        // `ecosystems` is declared so `targets` expands: a distribution block next to
        // an EMPTY target set is its own floor (publish-none cannot ship binaries).
        let text = "---\nstatus: approved\nmaturity: mvp\necosystems: [rust]\n\
                    distribution:\n  adapter: cargo-dist\n  installers: [shell]\n  \
                    homebrew_tap: owner/tap\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        assert!(n
            .problems
            .warnings
            .iter()
            .any(|warning| warning.contains("no 'homebrew' target")));
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
        let d = n.contract.distributions.into_iter().next().unwrap();
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
            .distributions
            .into_iter()
            .next()
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
        let d = n.contract.clone().distributions.into_iter().next().unwrap();
        assert_eq!(
            d.platforms,
            vec!["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]
        );
        let json = serde_json::to_value(&n.contract).unwrap();
        assert_eq!(
            json["distributions"][0]["platforms"],
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
        let d = norm(text)
            .contract
            .distributions
            .into_iter()
            .next()
            .unwrap();
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
        assert_eq!(json["distributions"], serde_json::json!([]));
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

    /// `quote_for_diagnostic` JSON-encodes: quotes/backslashes/newlines/control
    /// chars are escaped, ordinary text stays readable.
    #[test]
    fn quote_for_diagnostic_escapes_hostile_input() {
        assert_eq!(quote_for_diagnostic("foo"), "\"foo\"");
        assert_eq!(quote_for_diagnostic("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_for_diagnostic("a\nb"), "\"a\\nb\"");
        assert_eq!(quote_for_diagnostic("a\tb"), "\"a\\tb\"");
        // A bare C0 control char (0x01) escapes to , never a raw byte.
        assert_eq!(quote_for_diagnostic("\u{1}"), "\"\\u0001\"");
    }

    /// Log-injection hardening: a user-controlled unknown-field KEY carrying a
    /// quote, newline, and control char cannot forge a diagnostic line or emit a
    /// raw control char — it is JSON-encoded onto a single intact line.
    #[test]
    fn unknown_field_key_is_escaped_in_warning() {
        // The key is `evil"key` + newline + a forged-looking line + a control char.
        // Quoted in YAML so the literal quote/newline/control byte are the KEY text.
        let text =
            "---\nstatus: approved\nmaturity: mvp\n\"evil\\\"key\\nforged: line\\u0001\": 1\n---\n";
        let n = norm(text);
        assert!(n.is_valid(), "errors: {:?}", n.problems.errors);
        let warning = n
            .problems
            .warnings
            .iter()
            .find(|w| w.contains("unknown field(s) preserved"))
            .expect("expected an unknown-field warning");
        // The raw quote/newline/control char never appear unescaped in the message:
        // no forged second line, no bare control byte.
        assert!(
            !warning.contains('\n'),
            "warning must stay on one line: {warning:?}"
        );
        assert!(
            !warning.contains('\u{1}'),
            "warning must not carry a raw control char: {warning:?}"
        );
        assert!(
            !warning.contains("evil\"key"),
            "the raw unescaped key must not appear: {warning:?}"
        );
        // The escaped JSON form is present (quote → \", newline → \n, ctrl → ).
        assert!(
            warning.contains("\\\"") && warning.contains("\\n") && warning.contains("\\u0001"),
            "the key must be JSON-escaped: {warning:?}"
        );
    }

    /// The same hardening on a user-controlled VALUE routed through `yaml_display`
    /// (an invalid enum): a newline in the rejected value cannot forge an error
    /// line.
    #[test]
    fn invalid_enum_value_is_escaped_in_error() {
        let text = "---\nstatus: approved\nmaturity: \"mvp\\nforged: line\"\n---\n";
        let n = norm(text);
        assert_error_contains(&n, "maturity");
        let err = n
            .problems
            .errors
            .iter()
            .find(|e| e.contains("maturity") && e.contains("invalid"))
            .expect("expected a maturity-invalid error");
        assert!(!err.contains('\n'), "error must stay on one line: {err:?}");
        assert!(
            err.contains("\\n"),
            "the rejected value's newline must be escaped: {err:?}"
        );
    }
}

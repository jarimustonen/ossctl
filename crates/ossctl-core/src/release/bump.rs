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

// ── Cut-time edit transforms (pure) ──────────────────────────────────────────
//
// The engine-owned bump phase applies a deterministic edit set inside the clean
// checkout (`release-rust-workspace-multicrate` facet 2). These are the *pure* text
// transforms behind those edits — no filesystem, no process — so each is exhaustively
// unit-tested and the effectful executor ([`crate::release::bump_exec`]) is thin glue.
// Every transform **fails closed**: it returns a [`BumpEditError`] rather than write an
// ambiguous or silently-wrong result onto the irreversible cut path.

/// Why a cut-time bump edit could not be applied to a file's text. Each variant is a
/// fail-closed refusal — the executor aborts the cut rather than commit a wrong edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BumpEditError {
    /// The `[workspace.package]` section, or its `version = "…"` line, was not found —
    /// the workspace root manifest is not the shape the bump expects.
    WorkspaceVersionNotFound,
    /// Neither the root `[workspace.package]` nor `[package]` table carried the
    /// expected `version = "…"` line, so the engine cannot identify a version source.
    RootManifestVersionNotFound,
    /// A pin rewrite found no line declaring `dependency` with the exact `from`
    /// requirement — the sealed pin does not match the tree, so the executor refuses
    /// rather than guess (fail closed on **zero** matches).
    PinNotFound {
        /// The dependency whose `=<from>` pin was expected.
        dependency: String,
        /// The exact requirement string that was expected (`=<from_version>`).
        from: String,
    },
    /// A pin rewrite matched `dependency`'s `from` requirement in **more than one**
    /// place, so replacing is ambiguous — the executor refuses rather than rewrite the
    /// wrong one (fail closed on **multiple** matches).
    PinAmbiguous {
        /// The dependency whose pin matched more than once.
        dependency: String,
        /// The requirement string that matched multiply.
        from: String,
        /// How many declarations matched.
        count: usize,
    },
    /// The CHANGELOG had no `## [Unreleased]` section to finalize, but the contract's
    /// changelog mode said the engine should finalize one — fail closed rather than
    /// tag a release whose notes were never promoted.
    ChangelogUnreleasedNotFound,
}

impl std::fmt::Display for BumpEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkspaceVersionNotFound => write!(
                f,
                "could not find a `[workspace.package]` `version = \"…\"` line matching the \
                 sealed bump in the workspace root manifest"
            ),
            Self::RootManifestVersionNotFound => write!(
                f,
                "could not find a root `[package]` `version = \"…\"` line matching the sealed \
                 bump after no `[workspace.package]` version source was found"
            ),
            Self::PinNotFound { dependency, from } => write!(
                f,
                "no `{dependency} = \"{from}\"` intra-workspace pin found to rewrite (the sealed \
                 plan's pin does not match the tree)"
            ),
            Self::PinAmbiguous {
                dependency,
                from,
                count,
            } => write!(
                f,
                "the `{dependency} = \"{from}\"` pin matched {count} declarations — refusing to \
                 rewrite an ambiguous pin"
            ),
            Self::ChangelogUnreleasedNotFound => write!(
                f,
                "the contract asks the engine to finalize the CHANGELOG, but no `## [Unreleased]` \
                 section was found to promote"
            ),
        }
    }
}

impl std::error::Error for BumpEditError {}

/// The `[workspace.package]` `version = "…"` value, or `None` when the section or its
/// `version` line is absent.
#[must_use]
pub fn workspace_version(manifest: &str) -> Option<String> {
    section_version(manifest, "workspace.package")
}

/// The root `[package]` `version = "…"` value, or `None` when the table or its version
/// line is absent.
#[must_use]
pub fn package_version(manifest: &str) -> Option<String> {
    section_version(manifest, "package")
}

/// The release version source in a root Cargo manifest. A workspace package version is
/// authoritative when present; otherwise a plain single-crate `[package]` version is
/// used. This is deliberately a shape check, not a best-effort search across tables.
#[must_use]
pub fn root_manifest_version(manifest: &str) -> Option<String> {
    workspace_version(manifest).or_else(|| package_version(manifest))
}

fn section_version(manifest: &str, section: &str) -> Option<String> {
    let mut in_section = false;
    for line in manifest.lines() {
        let trimmed = strip_comment(line).trim();
        if let Some(header) = section_header(trimmed) {
            in_section = header == section;
        } else if in_section && line_starts_with_key(trimmed, "version") {
            if let Some(v) = scan_key_string(trimmed, "version") {
                return Some(v);
            }
        }
    }
    None
}

/// Rewrite the `[workspace.package]` `version = "<from>"` line to `to`, returning the
/// new manifest text.
///
/// Scoped to the `[workspace.package]` section (the single source of truth for the
/// release version) so a `version` key in any other table — `[package]`,
/// `[dependencies.foo]`, `[workspace.dependencies]` — is never touched. Preserves the
/// line's exact indentation and quote style; only the value between the quotes changes.
///
/// **Verified against `from`** (llm-review defense-in-depth): the line is rewritten only
/// when its current value is exactly `from` (the sealed pre-bump version). This makes the
/// edit fail closed on a tree that does not match the plan, and — since the whole-key scan
/// is line-oriented — it also sidesteps a `version = "…"` occurrence *inside a quoted
/// string value* (e.g. a `description` that mentions a version) unless that string
/// happens to equal `from`, in which case a following real `version` line still matches.
///
/// # Errors
/// [`BumpEditError::WorkspaceVersionNotFound`] when the section, or a `version = "<from>"`
/// line within it, is absent (fail closed rather than write a manifest with no bump).
pub fn set_workspace_version(
    manifest: &str,
    from: &str,
    to: &str,
) -> Result<String, BumpEditError> {
    set_section_version(manifest, "workspace.package", from, to)
        .ok_or(BumpEditError::WorkspaceVersionNotFound)
}

/// Rewrite the root `[package]` `version = "<from>"` line to `to`, preserving the
/// line's formatting and failing closed when the sealed source version is absent.
pub fn set_package_version(manifest: &str, from: &str, to: &str) -> Result<String, BumpEditError> {
    set_section_version(manifest, "package", from, to)
        .ok_or(BumpEditError::RootManifestVersionNotFound)
}

fn set_section_version(manifest: &str, section: &str, from: &str, to: &str) -> Option<String> {
    let mut out = String::with_capacity(manifest.len() + to.len());
    let mut in_section = false;
    let mut replaced = false;
    let ends_with_newline = manifest.ends_with('\n');
    let mut lines = manifest.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = strip_comment(line).trim();
        if let Some(header) = section_header(trimmed) {
            in_section = header == section;
        } else if in_section && !replaced && line_starts_with_key(trimmed, "version") {
            if let Some(rewritten) = replace_exact_string_value(line, "version", from, to) {
                out.push_str(&rewritten);
                push_line_ending(&mut out, lines.peek().is_some(), ends_with_newline);
                replaced = true;
                continue;
            }
        }
        out.push_str(line);
        push_line_ending(&mut out, lines.peek().is_some(), ends_with_newline);
    }
    replaced.then_some(out)
}

/// Rewrite a single intra-workspace `=`-pin (`dependency = "…, version = \"<from>\""`)
/// from `from` to `to`, returning the new manifest text.
///
/// Precise and fail-closed (`release-rust-workspace-multicrate` facet 3): it counts the
/// declarations of `dependency` whose version requirement is **exactly** `from` and
/// rewrites iff there is exactly one — refusing on zero
/// ([`BumpEditError::PinNotFound`]) or several ([`BumpEditError::PinAmbiguous`]). It
/// matches both the inline-table form (`dep = { path = "…", version = "=X" }`) and the
/// dependency sub-table form (`[dependencies.dep]` … `version = "=X"`), the two shapes
/// [`crate::facts`] records a requirement for.
///
/// # Errors
/// [`BumpEditError::PinNotFound`] / [`BumpEditError::PinAmbiguous`] as above.
pub fn rewrite_pin(
    manifest: &str,
    dependency: &str,
    from: &str,
    to: &str,
) -> Result<String, BumpEditError> {
    // First pass: count matches so we can fail closed on 0 or >1 without a partial edit.
    let matches = count_pin_matches(manifest, dependency, from);
    if matches == 0 {
        return Err(BumpEditError::PinNotFound {
            dependency: dependency.to_string(),
            from: from.to_string(),
        });
    }
    if matches > 1 {
        return Err(BumpEditError::PinAmbiguous {
            dependency: dependency.to_string(),
            from: from.to_string(),
            count: matches,
        });
    }
    // Exactly one match: rewrite it.
    let mut out = String::with_capacity(manifest.len() + to.len());
    let mut in_dep_subtable = false;
    let ends_with_newline = manifest.ends_with('\n');
    let mut done = false;
    let mut lines = manifest.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = strip_comment(line).trim();
        let mut rewritten: Option<String> = None;
        if let Some(header) = section_header(trimmed) {
            in_dep_subtable = dep_subtable_matches(header, dependency);
        } else if !done {
            if in_dep_subtable {
                // A `version = "<from>"` line inside `[dependencies.<dep>]`.
                rewritten = replace_exact_string_value(line, "version", from, to);
            } else if line_declares_dep_inline(trimmed, dependency) {
                // An inline `dep = { …, version = "<from>" }` line.
                rewritten = replace_exact_string_value(line, "version", from, to);
            }
        }
        match rewritten {
            Some(r) => {
                out.push_str(&r);
                done = true;
            }
            None => out.push_str(line),
        }
        push_line_ending(&mut out, lines.peek().is_some(), ends_with_newline);
    }
    Ok(out)
}

/// Count the declarations of `dependency` whose version requirement is exactly `from`
/// — the fail-closed gate [`rewrite_pin`] keys on.
fn count_pin_matches(manifest: &str, dependency: &str, from: &str) -> usize {
    let mut count = 0;
    let mut in_dep_subtable = false;
    for line in manifest.lines() {
        let trimmed = strip_comment(line).trim();
        if let Some(header) = section_header(trimmed) {
            in_dep_subtable = dep_subtable_matches(header, dependency);
        } else if in_dep_subtable {
            if key_has_exact_string(trimmed, "version", from) {
                count += 1;
            }
        } else if line_declares_dep_inline(trimmed, dependency)
            && inline_has_exact_version(trimmed, from)
        {
            count += 1;
        }
    }
    count
}

/// Finalize a Keep-a-Changelog CHANGELOG: promote the `## [Unreleased]` section's
/// content under a new dated `## [<version>] - <date>` header, leaving a fresh empty
/// `## [Unreleased]` above it for the next cycle. Returns the new text.
///
/// Deliberately conservative: it inserts one dated header immediately after the
/// `## [Unreleased]` line and does not otherwise reflow the file, so it composes with a
/// human-curated body. `date` is `YYYY-MM-DD`.
///
/// # Errors
/// [`BumpEditError::ChangelogUnreleasedNotFound`] when there is no `## [Unreleased]`
/// header to promote (fail closed — the contract asked for a finalize there is nothing
/// to finalize).
pub fn finalize_changelog(text: &str, version: &str, date: &str) -> Result<String, BumpEditError> {
    let ends_with_newline = text.ends_with('\n');
    let mut out = String::with_capacity(text.len() + version.len() + date.len() + 16);
    let mut inserted = false;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        out.push_str(line);
        push_line_ending(&mut out, lines.peek().is_some(), ends_with_newline);
        if !inserted && is_unreleased_header(line) {
            // A blank line, then the dated release header — Keep a Changelog style.
            out.push('\n');
            out.push_str("## [");
            out.push_str(version);
            out.push_str("] - ");
            out.push_str(date);
            // Guarantee a newline after the inserted header even at EOF, so the
            // promoted content is not glued onto it.
            out.push('\n');
            inserted = true;
        }
    }
    if inserted {
        Ok(out)
    } else {
        Err(BumpEditError::ChangelogUnreleasedNotFound)
    }
}

/// Whether `line` is a `## [Unreleased]` header (Keep a Changelog), tolerant of
/// surrounding whitespace and `Unreleased` letter-case.
fn is_unreleased_header(line: &str) -> bool {
    let t = line.trim();
    let Some(rest) = t.strip_prefix("##") else {
        return false;
    };
    let rest = rest.trim();
    rest.eq_ignore_ascii_case("[unreleased]")
}

/// The bracketed section name of a TOML header line (`[a.b.c]` → `Some("a.b.c")`), or
/// `None` when the line is not a bare section header.
fn section_header(trimmed: &str) -> Option<&str> {
    // Only a plain `[header]`; an array-of-tables `[[x]]` is not a bump target.
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.starts_with('[') || inner.contains('[') {
        return None;
    }
    Some(inner.trim())
}

/// Whether a TOML section `header` is the dependency sub-table for `dependency`
/// (`[dependencies.<dep>]`, `[build-dependencies.<dep>]`, or a target-specific
/// `[target.<cfg>.dependencies.<dep>]`), excluding every `dev-dependencies` form.
fn dep_subtable_matches(header: &str, dependency: &str) -> bool {
    if header.contains("dev-dependencies") {
        return false;
    }
    for infix in ["dependencies.", "build-dependencies."] {
        if let Some(idx) = header.rfind(infix) {
            let name = header[idx + infix.len()..].trim().trim_matches(['"', '\'']);
            return name == dependency;
        }
    }
    false
}

/// Whether `trimmed` (a `[dependencies]`-table line) declares `dependency` as an inline
/// table (`dep = { … }` or `"dep" = { … }`) — the form whose `version` an inline pin
/// rewrite edits. A dotted `dep.version = …` line is not matched here (its own key is
/// `dep.version`, handled by the sub-table/dotted paths).
fn line_declares_dep_inline(trimmed: &str, dependency: &str) -> bool {
    let Some(eq) = trimmed.find('=') else {
        return false;
    };
    let key = trimmed[..eq].trim().trim_matches(['"', '\'']);
    if key != dependency {
        return false;
    }
    trimmed[eq + 1..].trim_start().starts_with('{')
}

/// Whether an inline dependency table `trimmed` carries `version = "<from>"` exactly.
fn inline_has_exact_version(trimmed: &str, from: &str) -> bool {
    inline_version_value(trimmed).is_some_and(|v| v == from)
}

/// The `version = "…"` value inside an inline dependency table, matching `version` as a
/// whole key (mirrors the facts parser's discipline).
fn inline_version_value(inline: &str) -> Option<String> {
    scan_key_string(inline, "version")
}

/// Whether a trimmed line starts with `key =`, excluding a matching string embedded in
/// another key's value. Section version reads and writes use this stricter rule; inline
/// dependency-table scans intentionally use the more flexible token search below.
fn line_starts_with_key(line: &str, key: &str) -> bool {
    line.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Whether a `key = "value"` line (whole-key `key`) has value exactly `expected`.
fn key_has_exact_string(trimmed: &str, key: &str, expected: &str) -> bool {
    scan_key_string(trimmed, key).is_some_and(|v| v == expected)
}

/// Replace a whole-key `key = "<old>"` with `key = "<new>"` on `line`, but only when
/// the current value is exactly `old`; returns the rewritten line or `None`.
fn replace_exact_string_value(line: &str, key: &str, old: &str, new: &str) -> Option<String> {
    let current = scan_key_string(strip_comment(line).trim(), key)?;
    if current != old {
        return None;
    }
    replace_string_value(line, key, new)
}

/// Replace the value of a whole-key `key = "…"` on `line` with `new` (keeping quote
/// style and everything else on the line), or `None` when the line has no such key.
///
/// Operates on the raw `line` (so indentation and a trailing inline `# comment` are
/// preserved), locating the quoted value via the same whole-key scan used to read it.
fn replace_string_value(line: &str, key: &str, new: &str) -> Option<String> {
    let (val_start, quote) = locate_key_string(line, key)?;
    // `val_start` points at the opening quote; find the closing quote.
    let after_open = val_start + 1;
    let rel_close = line[after_open..].find(quote)?;
    let close = after_open + rel_close;
    let mut out = String::with_capacity(line.len() + new.len());
    out.push_str(&line[..after_open]);
    out.push_str(new);
    out.push_str(&line[close..]);
    Some(out)
}

/// The value of a whole-key `key = "…"` in `s` (matching the facts parser's whole-token
/// discipline), or `None`.
fn scan_key_string(s: &str, key: &str) -> Option<String> {
    let (open, quote) = locate_key_string(s, key)?;
    let after_open = open + 1;
    let rel_close = s[after_open..].find(quote)?;
    Some(s[after_open..after_open + rel_close].to_string())
}

/// Locate a whole-key `key = "…"` in `s`, returning the byte offset of the opening
/// quote and the quote char. "Whole key" = the char before `key` is not an identifier
/// char, and `key` is immediately followed (past spaces) by `=` then a quote.
fn locate_key_string(s: &str, key: &str) -> Option<(usize, char)> {
    let mut search = 0;
    while let Some(rel) = s[search..].find(key) {
        let pos = search + rel;
        let prev_is_ident = s[..pos]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
        let after = &s[pos + key.len()..];
        let after_trimmed = after.trim_start();
        if !prev_is_ident {
            if let Some(rest) = after_trimmed.strip_prefix('=') {
                let rest_trimmed = rest.trim_start();
                if let Some(q) = rest_trimmed.chars().next() {
                    if q == '"' || q == '\'' {
                        // Offset of the quote in the original string.
                        let consumed = s.len() - rest_trimmed.len();
                        return Some((consumed, q));
                    }
                }
            }
        }
        search = pos + key.len();
    }
    None
}

/// Strip a trailing `# comment` from a TOML line, respecting quoted `#`s crudely: it
/// cuts at the first `#` not inside a quote. Sufficient for manifest lines the bump
/// touches (version/pin values never contain `#`).
fn strip_comment(line: &str) -> &str {
    let mut in_str: Option<char> = None;
    for (i, c) in line.char_indices() {
        match in_str {
            Some(q) => {
                if c == q {
                    in_str = None;
                }
            }
            None => match c {
                '"' | '\'' => in_str = Some(c),
                '#' => return &line[..i],
                _ => {}
            },
        }
    }
    line
}

/// Append the correct line ending: a `\n` between lines, and preserve whether the file
/// ended with a trailing newline (so a rewrite is byte-faithful).
fn push_line_ending(out: &mut String, more_lines: bool, ends_with_newline: bool) {
    if more_lines || ends_with_newline {
        out.push('\n');
    }
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

    // ── set_workspace_version ────────────────────────────────────────────────

    #[test]
    fn sets_the_workspace_package_version_only() {
        let manifest = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.4.0\"\nedition = \"2021\"\n";
        let out = set_workspace_version(manifest, "0.4.0", "0.5.0").unwrap();
        assert!(out.contains("version = \"0.5.0\""));
        assert!(!out.contains("0.4.0"));
        // Everything else preserved.
        assert!(out.contains("edition = \"2021\""));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn does_not_touch_a_version_in_another_section() {
        let manifest =
            "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"0.4.0\"\n";
        let out = set_workspace_version(manifest, "0.4.0", "0.5.0").unwrap();
        assert!(out.contains("[package]\nversion = \"9.9.9\""));
        assert!(out.contains("[workspace.package]\nversion = \"0.5.0\""));
    }

    #[test]
    fn does_not_match_a_version_inside_a_description_string() {
        let manifest = "[workspace.package]\ndescription = 'requires version = \"0.4.0\"'\nversion = \"0.4.0\"\n";
        let out = set_workspace_version(manifest, "0.4.0", "0.5.0").unwrap();
        assert!(
            out.contains("requires version = \"0.4.0\""),
            "description untouched: {out}"
        );
        assert!(
            out.contains("version = \"0.5.0\""),
            "real version bumped: {out}"
        );
    }

    #[test]
    fn fails_closed_when_the_current_version_does_not_match_from() {
        let manifest = "[workspace.package]\nversion = \"1.2.3\"\n";
        assert_eq!(
            set_workspace_version(manifest, "0.4.0", "0.5.0"),
            Err(BumpEditError::WorkspaceVersionNotFound)
        );
    }

    #[test]
    fn package_version_is_available_for_a_plain_single_crate_manifest() {
        let manifest = "[package]\nname = \"acme\"\nversion = \"1.0.0\"\n";
        assert_eq!(package_version(manifest).as_deref(), Some("1.0.0"));
        assert_eq!(root_manifest_version(manifest).as_deref(), Some("1.0.0"));
        assert_eq!(
            set_package_version(manifest, "1.0.0", "2.0.0").unwrap(),
            "[package]\nname = \"acme\"\nversion = \"2.0.0\"\n"
        );
        let with_description =
            "[package]\ndescription = 'requires version = \"1.0.0\"'\nversion = \"1.0.0\"\n";
        let out = set_package_version(with_description, "1.0.0", "2.0.0").unwrap();
        assert!(out.contains("requires version = \"1.0.0\""));
        assert_eq!(package_version(&out).as_deref(), Some("2.0.0"));
    }

    #[test]
    fn root_manifest_version_prefers_workspace_inheritance() {
        let manifest =
            "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"1.0.0\"\n";
        assert_eq!(root_manifest_version(manifest).as_deref(), Some("1.0.0"));
    }

    #[test]
    fn package_rewrite_fails_closed_when_neither_root_version_shape_matches() {
        let manifest = "[package]\nname = \"acme\"\n";
        assert_eq!(
            set_package_version(manifest, "1.0.0", "2.0.0"),
            Err(BumpEditError::RootManifestVersionNotFound)
        );
    }

    // ── rewrite_pin ──────────────────────────────────────────────────────────

    #[test]
    fn rewrites_an_inline_table_pin() {
        let manifest = "[dependencies]\nossctl-core = { path = \"../ossctl-core\", version = \"=0.4.0\" }\nserde = \"1\"\n";
        let out = rewrite_pin(manifest, "ossctl-core", "=0.4.0", "=0.5.0").unwrap();
        assert!(out.contains("version = \"=0.5.0\""));
        assert!(out.contains("path = \"../ossctl-core\""));
        assert!(out.contains("serde = \"1\""));
    }

    #[test]
    fn rewrites_a_subtable_pin() {
        let manifest =
            "[dependencies.ossctl-core]\npath = \"../ossctl-core\"\nversion = \"=0.4.0\"\n";
        let out = rewrite_pin(manifest, "ossctl-core", "=0.4.0", "=0.5.0").unwrap();
        assert!(out.contains("version = \"=0.5.0\""));
    }

    #[test]
    fn pin_rewrite_fails_closed_when_absent() {
        let manifest =
            "[dependencies]\nossctl-core = { path = \"../ossctl-core\", version = \"^0.4\" }\n";
        assert_eq!(
            rewrite_pin(manifest, "ossctl-core", "=0.4.0", "=0.5.0"),
            Err(BumpEditError::PinNotFound {
                dependency: "ossctl-core".into(),
                from: "=0.4.0".into(),
            })
        );
    }

    #[test]
    fn pin_rewrite_leaves_a_caret_dep_untouched_even_with_same_crate() {
        // A different crate sharing the exact from-string must not be rewritten.
        let manifest = "[dependencies]\nossctl-core = { path = \"../c\", version = \"=0.4.0\" }\nother = \"=0.4.0\"\n";
        let out = rewrite_pin(manifest, "ossctl-core", "=0.4.0", "=0.5.0").unwrap();
        assert!(out.contains("ossctl-core = { path = \"../c\", version = \"=0.5.0\" }"));
        // `other = "=0.4.0"` is a plain registry dep, not our pin — untouched.
        assert!(out.contains("other = \"=0.4.0\""));
    }

    #[test]
    fn pin_rewrite_fails_closed_on_multiple_matches() {
        let manifest = "[dependencies]\ncore = { path = \"a\", version = \"=0.4.0\" }\n[build-dependencies]\ncore = { path = \"a\", version = \"=0.4.0\" }\n";
        let err = rewrite_pin(manifest, "core", "=0.4.0", "=0.5.0").unwrap_err();
        assert!(matches!(err, BumpEditError::PinAmbiguous { count: 2, .. }));
    }

    // ── finalize_changelog ───────────────────────────────────────────────────

    #[test]
    fn finalizes_the_unreleased_section() {
        let text = "# Changelog\n\n## [Unreleased]\n### Added\n- a thing\n";
        let out = finalize_changelog(text, "0.5.0", "2026-08-13").unwrap();
        assert!(out.contains("## [Unreleased]\n\n## [0.5.0] - 2026-08-13"));
        assert!(out.contains("- a thing"));
    }

    #[test]
    fn changelog_finalize_fails_closed_without_unreleased() {
        let text = "# Changelog\n\n## [0.4.0] - 2026-01-01\n";
        assert_eq!(
            finalize_changelog(text, "0.5.0", "2026-08-13"),
            Err(BumpEditError::ChangelogUnreleasedNotFound)
        );
    }
}

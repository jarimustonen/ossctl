//! The deterministic cargo-dist config generator (issue
//! `release-engine-dist-config-generator`).
//!
//! Renders a downstream project's `dist-workspace.toml` from the contract's
//! [`Distribution`] block — the deterministic half of `shipshape dist generate`.
//! The `shipshape-cli` handler writes the rendered text to the repo root and then
//! invokes `dist generate` (the cargo-dist tool) to produce the tag-triggered
//! `.github/workflows/release.yml` from it; this module never shells out and
//! never touches the filesystem, so it is a pure, fully unit-testable function.
//!
//! ## What maps where (the contract → cargo-dist mapping)
//!
//! The mapping is the binding cross-platform default documented in the
//! `/shipshape-release` skill and `AGENTS.md` ("Cross-platform is a hard requirement —
//! macOS AND Linux"):
//!
//! - **`distribution.platforms` → `[dist] targets`.** Copied verbatim (Rust
//!   target-triple syntax). The normalizer guarantees the set is non-empty and
//!   defaults an omitted `platforms` to the cross-platform macOS + Linux-musl
//!   set ([`DEFAULT_CROSS_PLATFORM_TARGETS`](crate::contract::schema::DEFAULT_CROSS_PLATFORM_TARGETS)),
//!   so a repo that never thinks about platforms still ships Linux binaries.
//!   This generator NEVER narrows the set — a macOS-only matrix is a release gap.
//! - **`distribution.installers` → `[dist] installers`.** Mapped through, with
//!   two deliberate rules that mirror shipshape's own `dist-workspace.toml`:
//!   1. `shell` is always ensured, so the generated curl-installer covers the
//!      Unix side (macOS AND Linux) even when the contract omitted it; and
//!   2. `homebrew` is EXCLUDED from cargo-dist's installer set — shipshape publishes
//!      the Homebrew formula through its own tap adapter (post-tag, needing the
//!      tarball sha256 that only exists after the release), exactly as the
//!      reference config does ("Homebrew auto-publish is deliberately NOT enabled
//!      here"). The tap itself is threaded elsewhere (`distribution.homebrew_tap`
//!      in [`crate::release::plan`]).
//!
//! The rest of the `[dist]` table is the fixed reference shape: a pinned
//! [`PINNED_CARGO_DIST_VERSION`], `ci = "github"`, `hosting = "github"`,
//! `github-attestations = true`, and `pr-run-mode = "skip"` (tag-triggered only).
//! The personal `[dist.github-custom-runners]` override in shipshape's own config is
//! repo-local infra and is deliberately NOT emitted for downstream projects.
//!
//! Determinism: the output is a pure function of the [`Distribution`] — no clock,
//! no environment, no map iteration — so the same block always renders the same
//! bytes (proven in tests).

use std::fmt::Write as _;

use crate::contract::schema::{Distribution, Installer};

/// The cargo-dist version pinned into every generated `dist-workspace.toml`, so a
/// regenerated workflow and a locally-installed `dist` stay in lockstep. Kept in
/// step with shipshape's own reference `dist-workspace.toml` at the repo root.
pub const PINNED_CARGO_DIST_VERSION: &str = "0.28.2";

/// The result of rendering a `dist-workspace.toml` from a [`Distribution`].
///
/// Carries the rendered [`toml`](Self::toml) plus the resolved decisions
/// (`targets` / `installers` after the shell-ensure + homebrew-exclude rules) and
/// any non-fatal [`warnings`](Self::warnings), so the CLI handler can report what
/// it decided without re-deriving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDistConfig {
    /// The full `dist-workspace.toml` text, ready to write to the repo root.
    pub toml: String,
    /// The `[dist] targets` set (verbatim from `distribution.platforms`).
    pub targets: Vec<String>,
    /// The `[dist] installers` set actually emitted (shell ensured, homebrew
    /// excluded).
    pub installers: Vec<String>,
    /// The pinned cargo-dist version emitted ([`PINNED_CARGO_DIST_VERSION`]).
    pub cargo_dist_version: &'static str,
    /// Non-fatal notes about decisions the generator made (added `shell`,
    /// excluded `homebrew`, a Linux-less target set).
    pub warnings: Vec<String>,
}

/// Render a `dist-workspace.toml` from a normalized [`Distribution`] block.
///
/// Assumes the block has already been through the normalizer (so `platforms` is
/// non-empty and every triple is well-formed, and `installers` is canonically
/// ordered and de-duplicated). Reads only `platforms` and `installers`; the
/// `adapter` gate (cargo-dist vs goreleaser/manual) is the caller's, since this
/// renderer only knows how to emit cargo-dist config.
#[must_use]
pub fn generate(dist: &Distribution) -> GeneratedDistConfig {
    let mut warnings = Vec::new();

    // targets: verbatim from the contract's platform set. Never narrowed.
    let targets: Vec<String> = dist.platforms.clone();
    if !targets.iter().any(|t| t.contains("linux")) {
        warnings.push(
            "distribution.platforms lists no Linux target — the cross-platform install \
             requirement (macOS AND Linux) is not met; add an '…-unknown-linux-musl' triple"
                .to_string(),
        );
    }

    // installers: map through, excluding homebrew (owned by the tap adapter,
    // post-tag) and ensuring shell (the Unix curl-installer covering Mac+Linux).
    // The match is EXHAUSTIVE (no `_ =>` catch-all) on purpose: adding an
    // `Installer` variant must force a conscious decision here about whether
    // cargo-dist understands it, not silently pass an unknown name through.
    let mut installers: Vec<String> = Vec::new();
    let mut excluded_homebrew = false;
    for installer in &dist.installers {
        let name = match installer {
            Installer::Homebrew => {
                excluded_homebrew = true;
                continue;
            }
            Installer::Shell => Installer::Shell.as_str(),
            Installer::Powershell => Installer::Powershell.as_str(),
            Installer::Msi => Installer::Msi.as_str(),
            Installer::Npm => Installer::Npm.as_str(),
        };
        // The normalizer already de-duplicates `installers`; this guard is only
        // belt-and-suspenders so a hand-built `Distribution` cannot emit a
        // duplicate installer line.
        if !installers.iter().any(|s| s == name) {
            installers.push(name.to_string());
        }
    }
    if excluded_homebrew {
        warnings.push(
            "the 'homebrew' installer is published by shipshape's Homebrew tap adapter (post-tag, \
             once the release tarball sha256 exists), not cargo-dist — it is excluded from [dist] \
             installers, mirroring shipshape's own dist-workspace.toml"
                .to_string(),
        );
    }
    let shell = Installer::Shell.as_str().to_string();
    if !installers.contains(&shell) {
        // Prepend so the ensured shell keeps the canonical shell-first order.
        installers.insert(0, shell);
        warnings.push(
            "added the 'shell' installer so the generated curl-installer covers macOS and Linux \
             (the Unix cross-platform install path)"
                .to_string(),
        );
    }

    let toml = render_toml(&targets, &installers);
    GeneratedDistConfig {
        toml,
        targets,
        installers,
        cargo_dist_version: PINNED_CARGO_DIST_VERSION,
        warnings,
    }
}

/// Render the `dist-workspace.toml` text for a resolved `targets` + `installers`
/// set. Values are simple, closed-vocabulary tokens (target triples and installer
/// names — `[a-z0-9-]`), so in practice they need no escaping; every interpolated
/// string nonetheless goes through [`toml_basic_string`] so a future variant or a
/// hand-built `Distribution` can never emit syntactically-broken TOML.
fn render_toml(targets: &[String], installers: &[String]) -> String {
    let mut out = String::new();
    // Header: mark the file generated and name the round-trip so a human does not
    // hand-edit the workflow it feeds.
    out.push_str(
        "# Generated by `shipshape dist generate` from OSS-RELEASE.md `distribution`.\n\
         # Edit distribution.* in OSS-RELEASE.md, then re-run `shipshape dist generate`.\n\
         # The tag-triggered `.github/workflows/release.yml` is produced from the\n\
         # [dist] section below via `dist generate` — never hand-edit the workflow.\n\
         [workspace]\n\
         members = [\"cargo:.\"]\n\
         \n\
         [dist]\n",
    );
    let _ = writeln!(out, "cargo-dist-version = \"{PINNED_CARGO_DIST_VERSION}\"");
    out.push_str("ci = \"github\"\n");
    let _ = writeln!(out, "installers = {}", inline_string_array(installers));
    out.push_str("targets = [\n");
    for target in targets {
        let _ = writeln!(out, "    {},", toml_basic_string(target));
    }
    out.push_str("]\n");
    out.push_str("hosting = \"github\"\n");
    out.push_str("github-attestations = true\n");
    out.push_str("pr-run-mode = \"skip\"\n");
    out
}

/// Render a slice of tokens as an inline TOML string array (`["a", "b"]`).
fn inline_string_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| toml_basic_string(s)).collect();
    format!("[{}]", quoted.join(", "))
}

/// Quote `value` as a TOML basic string, escaping the characters TOML requires
/// (`"`, `\`, control chars). For the closed-vocabulary tokens this module emits
/// this is a no-op beyond the surrounding quotes, but it makes the renderer robust
/// against a value that ever carries a special character rather than silently
/// producing invalid TOML.
fn toml_basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;

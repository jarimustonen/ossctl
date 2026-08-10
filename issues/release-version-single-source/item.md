---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@cut-noop-self-visibility-check']
---

# release version entered in two places (--version flag + workspace manifest) can drift — make the manifest the single source of truth

## Description

The release version is supplied BOTH on the command line (`release plan/cut --version X.Y.Z`) AND read from the workspace Cargo.toml, so the two can disagree — the two-masters footgun that cost the issuectl cut a failed attempt (stint #16 landed a --version-vs-tree drift GUARD as a stopgap). Make the workspace manifest the SINGLE SOURCE OF TRUTH: derive the release version from the manifest so version becomes a projection of facts, not an input. Keep `--version` as an OPTIONAL confirmation that must equal the manifest version (superset of the just-landed drift guard — reconcile/subsume it, don't duplicate), so the documented cut recipe and downstream skill wiring keep working; a mismatch still errors. Update plan.rs / release.rs accordingly. Do NOT regress the drift guard's tests — fold them in. Full green gate incl. RUSTDOCFLAGS doc build.

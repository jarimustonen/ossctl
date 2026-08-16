---
created: 2026-08-16
updated: 2026-08-16
type: task
status: in-progress
priority: high
lane: repo-hygiene
lane_seq: 10
---

# Audit: no user-specific facts in a public artifact

## Description

## Description

This repository is **public** and distributed. Audit it for user-specific facts that must not
ship in a public artifact, and move any that exist into user configuration.

### The rule

Maintainer rule, 2026-08-16: *a public repo must not reference user-specific things at all;
user-specific things belong in user config.*

A publicly-distributed artifact MUST NOT contain personal account handles, private
repo/project names, personal filesystem-layout conventions, hostnames, internal URLs, or
org-internal identifiers — not in source, **not in built-in defaults**, not in generated
scaffold/template output, not in installed skill content, not in docs, not in tests or
fixtures.

**Key point, and the one that caused the original defect: overridability does not launder a
user-specific default.** "Every value is configurable" is not a defence — an unset default is
still whatever ships in the package. The correct built-in default is neutral/absent, with an
actionable error naming the config key or env var to set. Never a silent guess at someone's
environment.

### Why this issue exists

`project-canon` — the family's own conformance tool — shipped these to crates.io in `0.1.1`
and `0.2.0`:

```rust
gh_account: "example-org".to_string(),
repo_root:  "/path/to/sources".to_string(),
const DEFAULT_FAMILY_TOOLS: [&str; 7] = [
    "issuectl", "orchestratectl", "private-tool-a", "private-tool-b", "ossctl", "a-private-service", "glasspad",
];
```

Three of those seven are **private** repositories, so a public crate disclosed the names of
private projects. The defect survived a design pass that explicitly considered portability,
because the reasoning stopped at "it's overridable".

Every public tool in the family is plausibly exposed to the same class of defect, so each is
being audited rather than assumed clean.

## What to check

Grep the whole repo — source, defaults, templates, generated output, docs, README, tests,
golden fixtures, skill content — for:

- the maintainer's account handle / name / email
- names of **private** family repos: `private-tool-a`, `private-tool-b`, `a-private-service`, `private-tool-c`
  (public siblings are fine to reference where genuinely relevant, e.g. a real dependency)
- personal path conventions (`/path/to/sources`, `/Users/<name>`, personal machine hostnames)
- internal URLs, internal service names, org-internal identifiers
- any built-in default that encodes one person's environment rather than a neutral value

## Acceptance

- No user-specific value anywhere in the shipped artifact (per the list above).
- Any environment-specific value the tool genuinely needs is read from user config, with a
  neutral built-in default and an actionable error when a required value is unset.
- Fixtures, examples, and docs use obviously fictional values.
- The maintainer's own setup still works, expressed through user config outside the repo.
- **If the audit finds nothing, close the issue saying so** — a recorded clean result is the
  point; it is what makes the family-wide sweep meaningful.

## Comments

Filed 2026-08-16 from `project-canon` as part of a family-wide sweep after the leak above was
found. Companion work in `project-canon`: `portable-neutral-defaults` (the concrete cleanup)
and `canon-no-user-specifics` (promoting this rule to a canon section with a mechanical
`doctor` check, so it is enforced rather than remembered). Once that check ships, this audit
becomes automated — this issue is the one-time manual pass.

### 2026-08-16T18:32:12Z · @claude

Laned into a new repo-hygiene lane (stint #21): the audit's likely touch set is docs, AGENTS.md, ADRs, test fixtures and dist-workspace.toml — a different hot-file family from the release-engine lanes, so it runs in parallel without collision. A quick scan confirms the audit is not a no-op: seven files reference user-specific things across three classes (a documented deliberate exception in the project's own build config, documentation, and test/ADR occurrences that are more likely accidental than decided).

### 2026-08-16T18:38:58Z · @claude

Audit result (2026-08-16): completed the tracked-artifact sweep and neutralised occurrences outside active shared modules.

Fixed:
- Replaced non-essential maintainer handles, paths, names, and host labels in docs, ADR provenance, README/install guidance, changelog history, workflow comments, issue history, vendored issue-skill prompts, and package authors metadata. Historical examples now use neutral or fictional values such as `example-org` and `/path/to/sources`.
- Kept the distribution runner override operational but removed the host name. `dist-workspace.toml` now explicitly marks it as a deliberate repository-local exception that is never generated for downstream projects. The downstream-generation test now names only a generic runner.
- Confirmed no bundled `crates/ossctl-cli/skills/*` template or CLI fixture contains a searched user-specific value. No generated default or installed oss skill inherits one.

Retained, with justification:
- `LICENSE` and copied crate licenses retain the legal copyright attribution.
- `Cargo.toml` repository/homepage metadata and `OSS-RELEASE.md` homebrew tap retain this public project's canonical delivery identity. The latter is required for the project's release path.
- `AGENTS.md` retains the exact self-hosted runner hostname only as the already-documented repository-local build exception. The override remains clearly marked in both `AGENTS.md` and `dist-workspace.toml`.
- `TODO.md` was not changed, as explicitly constrained.

Not edited because this task was explicitly forbidden from touching active shared areas. These are test-only examples, not shipped defaults, and should be neutralised by their owning release/contract units: `crates/ossctl-core/src/contract/normalize.rs`, `crates/ossctl-core/src/release/plan/tests.rs`, and `crates/ossctl-core/src/release/coordinator/tests.rs`.

The final scan found no private sibling names, personal filesystem conventions, hostnames, or internal URLs outside the retained/blocked items above. Product defaults and generated oss skill content are clean.

### 2026-08-16T18:42:36Z · @claude

Verification: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` passed. `cargo test --workspace` ran 490/491 core tests successfully but failed the pre-existing active release/coordinator test `ossctl_like_contract_cuts_end_to_end_across_target_classes`: its expected post-tag SHA-256 does not match the fixture formula. This task did not modify that forbidden release area.




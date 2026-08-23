# 0005 — Shipshape product migration and compatibility boundary

**Status:** Accepted
**Date:** 2026-08-23
**Authors:** Maintainer (name owner); `rename-ossctl-shipshape` migration worktree.

> This ADR supersedes the product names in ADR-0001 without rewriting that historical
> decision. ADRs 0001–0004 remain accurate records of decisions made for `ossctl`.

## Context

The released product is named `ossctl`, with crates `ossctl` and `ossctl-core`, ten
bundled `/oss-*` skills, and durable release state under Git's common directory at
`ossctl/{plans,releases}`. The product is being renamed to **Shipshape**. A rename that
only changes display strings would strand installed skills and, more seriously, could
make an interrupted irreversible release impossible to resume.

The locked new identities are:

- product, executable, and CLI package: `shipshape`;
- core crate: `shipshape-core` (Rust import `shipshape_core`);
- bundled skill namespace: `/shipshape-*`.

The GitHub repository itself is not renamed by this change. Its existing public URL and
all historical release records remain valid.

## Decision

### 1. Canonical package and command names change without executable aliases

The workspace publishes `shipshape` and `shipshape-core`; the only canonical binary is
`shipshape`. New source installs use `cargo install shipshape`. Distribution archives,
installers, formulas, CLI help, user agent, and generated artifacts use Shipshape.

The already-published `ossctl` and `ossctl-core` crate lines remain available at their
published 0.10.x versions but are frozen. The sole exception is completing a release run
that was already in flight at migration time; resume may publish its sealed legacy
version to finish an otherwise torn irreversible release. Shipshape does not ship an
`ossctl` alias or an
`ossctl-core` facade. An alias would cause `cargo install shipshape` and cargo-dist to
install or package two product commands indefinitely, while a facade would imply a
second supported library API and release order. Existing installations keep working at
their old version; moving to the maintained line is an explicit uninstall/install step.
Invoking an old binary is therefore not silently redirected to a different version.

### 2. Skills move as one atomic catalog; old names are actionable refusals

All ten bundled entries and their installation paths are renamed from `oss-*` to
`shipshape-*`. No old-name aliases are bundled: aliases would leave two trigger surfaces
that can issue conflicting instructions. A request for a known old name fails with a
stable `skill_renamed` error that names the new skill and the command to install it.

Installation does not delete old runtime files. Deleting files outside the requested
installation destination would violate the installer's no-surprise boundary and could
remove locally edited content. The rollout procedure installs the complete new catalog
first, verifies it, then explicitly removes the ten old runtime entries.

### 3. Durable release storage keeps its legacy namespace permanently

The machine-facing path

```
$(git rev-parse --git-common-dir)/ossctl/{plans,releases}
```

is a versioned compatibility namespace, not product branding, and remains unchanged.
`shipshape config path|show` reports it as the legacy-compatible storage root. This lets
Shipshape and the last ossctl release find the same lock, plans, journals, receipts, and
run IDs; avoids a split lock that could authorize concurrent cuts; and requires no
non-atomic directory migration.

Likewise, the sealed-plan hash domain `ossctl.release-plan`, journal event values,
schema versions, target IDs stored in old runs, and marker-bounded changelog tokens
remain stable. Changelog tokens keep their `oss-changelog:*` spelling because existing
downstream contracts already carry them. The project contract filename
`OSS-RELEASE.md` is stable for the same fleet-wide reason. New formulas carry a
Shipshape ownership marker; formula readers accept both the Shipshape and legacy ossctl
markers.
Temporary-file prefixes and internal test-only environment variables may adopt the new
name because they are neither durable nor public configuration.

### 4. Wire and release compatibility is preserved

The canonical JSON envelope and DTO field names do not change, so
`schema_version` remains unchanged. Skill-name values and the program name do change;
value-keyed consumers must follow this announced product migration even though the JSON
shape does not require a schema bump. Journal schemas remain unchanged. The execution
phase model and seal pre-image semantics do not change, so `SEAL_VERSION` remains 10.
The old seal domain is intentionally retained to authenticate existing plans.

Resume loads the stored sealed plan, including its historical `ossctl` package targets,
and executes it against the plan's sealed checkout. A plan for the renamed tree contains
`shipshape` targets naturally. If the sealed checkout or plan is unavailable, the
existing `plan_store_*` / sealed-checkout refusal remains the actionable boundary; the
rename adds no guessed conversion of package names or journal facts.

### 5. Source repository and channels have distinct migration rules

The existing GitHub repository coordinate remains
`jarimustonen/ossctl` until an external repository migration is separately chosen.
That coordinate is not rewritten in source links, old release notes, or repository
metadata. New crates.io coordinates are `shipshape` and `shipshape-core`. New release
assets and the Homebrew formula are named `shipshape`; the contract points at the new
`homebrew-shipshape` tap, which must exist before the first Shipshape cut.

Distribution continues to provide a source install and prebuilt macOS arm64/x86_64 and
Linux musl arm64/x86_64 binaries. Windows remains deliberately unsupported.

## Post-merge rollout order

The conductor performs external changes only after this repository migration is merged
and green:

1. Create and permission `jarimustonen/homebrew-shipshape`; do not mutate the old tap.
   Seed `Formula/shipshape.rb` with this exact first line before the cut:
   `# Generated by shipshape; do not edit by hand (template-version: 2)`. Verify the
   remote file's first line byte-for-byte. This explicit bootstrap authorization makes
   the engine use its verified-asset
   direct-write path instead of opening a first-formula PR that cannot be observed on
   the default branch until merged.
2. Build Shipshape from merged `main`, seal a fresh plan, and cut the first Shipshape
   release. Publish `shipshape-core` before `shipshape`; let cargo-dist create the four
   platform artifacts; let the engine write and verify the new formula.
3. Verify crates.io, GitHub assets/install script, and the new tap through the release
   engine. Do not remove the old installation before this passes.
4. Apply the declared Homebase fleet unit to install `shipshape` while retaining the
   working `ossctl` rollback binary. Verify `shipshape version --json` and `shipshape
   doctor --json`. Do not use a source-tree install as persistent setup.
5. Run `shipshape skill install --agent all`, verify all ten `shipshape-*` entries, then
   remove the corresponding `oss-*` files from Claude, pi, and Codex homes. Preserve any
   unrelated or locally authored skills. Only after those checks remove any unmanaged
   stale `ossctl` binary.
6. Keep the old crates, releases, tap, changelog sections, and git-common-dir storage
   readable. They are historical and compatibility state, not cleanup candidates.

If any channel verification fails, retain the old command and skill catalog and resume
or abandon the Shipshape run using the journal. Never publish by guessing around a
compatibility refusal.

## Consequences

- Users make one explicit command/package transition rather than carrying aliases
  forever.
- Existing interrupted releases remain resumable with no storage copy, schema bump, or
  seal migration.
- The legacy string remains in a few deliberately machine-stable and historical places.
  A global “no `ossctl` text” check would be incorrect; tests instead pin the allowed
  compatibility surfaces and the canonical new public surface.
- External channel creation and machine convergence are sequenced after a verified
  release, so the old working installation remains the rollback path.

Refs-Issue: rename-ossctl-shipshape

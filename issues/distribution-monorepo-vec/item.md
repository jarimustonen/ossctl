---
created: 2026-08-04
updated: 2026-08-10
type: feature
status: done
priority: normal
related: ['@contract-cannot-model-cargo-dist-release']
commits:
- hash: '2706589'
  summary: Vec<Distribution> + per-package association, schema_version 1->2, back-compat deser, release-seam threading
- hash: 4dfcff6
  summary: apply /llm-review — schema_version relabel-on-emit, engine multi-dist guard, typo/trim/golden fixes
closed: 2026-08-10
---

# Monorepo distribution: Vec<Distribution> with per-package association

_Source: /llm-review of contract-cannot-model-cargo-dist-release_

## Description

Deferred spin-off from the /llm-review of `contract-cannot-model-cargo-dist-release`. The new `distribution` block is a single `Option<Distribution>` on `Contract`, which models a single-artifact repo. A monorepo can ship multiple independently-distributed binaries (each with its own gh-releases/installer/tap). Extend the model to `Vec<Distribution>` with a way to associate each distribution with the package/target it belongs to, without breaking the single-distribution common case.

## Decision (maintainer, 2026-08-10) — implement now

**Do it now** (overrides the earlier "defer until a monorepo consumer appears" recommendation).
Design-first: this is a structural schema change (`schema_version` bump) touching the canonical
serde model (`schema.rs`) + `normalize.rs`. Must preserve the single-`Distribution` common case
(back-compat deser: a bare `distribution:` block still parses as one entry) and add the
per-package/target association key. Sequence strictly AFTER `publish-target-none` (both touch the
same LANE B hot files). Land via a reviewed worktree (design-first + `/llm-review` before merge).

## Design note (implementation, 2026-08-10)

**Field shape.** `Contract.distribution: Option<Distribution>` → `Contract.distributions:
Vec<Distribution>`. Canonical output is ALWAYS a JSON array (`[]` for a registry-only repo,
replacing the old `distribution: null`). This renames the top-level canonical key
`distribution` → `distributions`.

**Association key.** `Distribution` gains `package: Option<String>` (first field). It names the
package/target this distribution belongs to (matches a `Target.package` / a manifest package).
`null` for the sole/unassociated distribution (back-compat); always present in canonical output
(like `Target.package`). Floor: when `distributions.len() >= 2`, every entry MUST carry a
non-null, unique `package` — otherwise a monorepo's two distributions are indistinguishable. A
single distribution may leave `package` null.

**Back-compat deser — accept BOTH input keys (chosen over "one key, mapping-or-sequence").**
- `distribution:` (a single mapping) → parsed to a one-element `Vec` (the overwhelmingly common
  case, incl. ossctl's own `OSS-RELEASE.md` — the author changes NOTHING).
- `distributions:` (a sequence of mappings) → each parsed; each may carry `package:`.
- Declaring BOTH keys is an error (ambiguous). Both keys are in `KNOWN_KEYS`.

Justification for two distinct keys over a polymorphic single key: (1) a plural key is
self-documenting — `distributions:` reads as "many" at a glance; (2) each input key stays
*monomorphic* (one YAML type), matching the normalizer's style and avoiding the shape-polymorphism
that makes `targets`' absent-vs-empty subtle; (3) zero-change back-compat for the singular author.

**schema_version bump (deliberate).** Renaming the canonical key `distribution` → `distributions`
and adding `package` to every distribution is a BREAKING wire change (not a pure addition), so
`KNOWN_SCHEMA_VERSION` bumps 1 → 2. The tool still READS v1 documents (a `distribution:` mapping),
translating them into the v2 canonical shape. `SEAL_VERSION` bumps 3 → 4 (the sealed pre-image
embeds the serialized `Contract`, whose shape changed) and the golden `plan_id` vector is updated
in lockstep.

**Release-seam threading (minimal, mechanical — len==1 is byte-identical to today).**
- `release::plan::build` — `homebrew_tap` = first distribution carrying a tap
  (`distributions.iter().find_map(|d| d.homebrew_tap.clone())`); identical to today for 0/1
  distributions. A per-target tap for a true multi-tap monorepo is a documented follow-up.
- `ossctl dist generate` — 0 → existing `no_distribution`; 1 → today's behavior; ≥2 → a new
  `multiple_distributions` user error (per-package dist scaffolding is a follow-up).
- `audit::cross_platform_gap` — iterate all distributions; gap ids stay bare
  (`distribution-linux` / `distribution-macos`) for the single case (byte-identical audit), and
  are suffixed with the package for a monorepo (`distribution-linux:<pkg>`).

## Review follow-through (4-model /llm-review, 2026-08-10)

Two criticals (4/4 consensus) + several good catches, all fixed or deferred (report:
`history/review-distribution-monorepo-vec.md`):

- **schema_version mislabel (FIXED).** The normalizer echoed the DECLARED version, so a
  `schema_version: 1` doc emitted the v2 `distributions` body still labeled 1. Now emits
  `schema_version = KNOWN_SCHEMA_VERSION` always — reads v1, emits v2. (The `contract show` fixture
  test asserted the old buggy echo; corrected: envelope stays 1, contract payload is 2.)
- **Monorepo tap drop (FIXED).** `plan.homebrew_tap`'s `find_map` would silently drop a second
  distribution's tap at publish. Added `ensure_single_distribution` — `release plan`/`cut`/`resume`
  reject `distributions.len() > 1` (`multiple_distributions`) until the engine is per-distribution
  aware. `dist generate` already rejected ≥2.
- **Typo guard (FIXED, advisory).** Warn when a monorepo distribution's `package` matches no
  `targets[].package`. **Package trimming (FIXED).** Second golden vector for a populated
  distribution (FIXED).
- **Deferred → `per-distribution-release`:** per-distribution taps in the engine, `dist generate
  --package`, cargo-dist per-distribution platforms. The contract MODEL is complete; the engine is
  single-distribution and fails loud on a monorepo.
- **Declined:** deprecation warning on the singular `distribution:` key (contradicts the
  zero-change back-compat requirement); bumping the wire-envelope `SCHEMA_VERSION` (versions the
  envelope, not the payload — the contract's own `schema_version: 2` is the shape signal).

---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: fixed
priority: normal
commits:
- hash: 2ce27d7
  summary: first-class distribution block + normalize/validate + SEAL_VERSION bump + tests
- hash: ed3854a
  summary: apply /llm-review panel findings (adapter-required, spike floor, tap-slug hardening, dead-config warn)
closed: 2026-08-04
---

# OSS-RELEASE contract can't model a cargo-dist release (binaries + installer + Homebrew tap) alongside registry publishes

_Source: OSS-RELEASE.md targets/adapter model_

## Description

Surfaced running `/oss-init` on the **issuectl** repo.

## Observed
issuectl's release engine is **cargo-dist** (tag-triggered `.github/workflows/release.yml`) which produces: multi-platform GitHub-Release binaries, a shell installer, and a Homebrew formula pushed to a tap (`example-org/homebrew-issuectl`) — PLUS a separate crates.io publish (`publish-crates.yml`). The contract's `targets: [{ecosystem, package, registry, adapter}]` model only represents **registry** publishes; `adapter` allows `cargo-dist` but there is no first-class field for the **binary-distribution + tap + installer** layer.

## Expected / suggestion
A way to express a cargo-dist-style release in the machine contract, e.g.:
- a `binary`/`gh-releases` distribution target type (distinct from a registry target), and/or
- a release-level `adapter: cargo-dist` with sub-config for `installer`, `homebrew_tap`, and `registry_publish` split.

## Impact
Had to record the whole binary-distribution reality as prose in the draft's `## Rationale` + `## Release notes` (with an explicit 'keep cargo-dist; do not let /oss-release-cut regenerate release.yml' note). Downstream `/oss-*` members reading the contract can't see the tap/installer, so they'd under-describe or risk clobbering the existing pipeline.

## Resolution — chosen shape + rationale

**Model chosen:** a **release-level `distribution` block** (`Option<Distribution>` on `Contract`), separate from `targets` — NOT extra `targets` rows and NOT a per-target adapter. `targets` stays registry-only; the two layers coexist, which is exactly the "cargo-dist release alongside a crates.io publish" the model couldn't express.

```yaml
# issuectl's contract, now first-class:
ecosystems: [rust]
targets:
  - {ecosystem: rust, package: issuectl, registry: crates.io, adapter: cargo-publish}   # registry publish
distribution:                                        # binary/installer/tap layer
  adapter: cargo-dist                                # required: cargo-dist | goreleaser | manual
  gh_releases: true                                  # multi-platform binaries on the GH Release (default true)
  installers: [shell, homebrew]                      # shell | powershell | homebrew | msi | npm
  homebrew_tap: example-org/homebrew-issuectl       # owner/repo; required when installers ∋ homebrew
```

Canonical JSON: `distribution` is `null` for a registry-only repo (present-but-null, per the §4 "every field materialized" convention, like `versioning_pattern`).

**Why a release-level block, not per-target rows:** a cargo-dist repo has ONE tag-triggered `release.yml` that emits all platforms + the installer + the tap PR — that is not "another registry row per artifact". Flattening it into `targets` would (a) lose the shared-workflow relationship, (b) have no place for the shell installer (which isn't a registry), and (c) blur the crates.io publish with the binary layer. A distinct block lets a downstream `/oss-*` member SEE the tap + installer and neither under-describe the release nor regenerate the existing `release.yml`.

**Validation floors added:** `adapter` required when the block is present (no silent cargo-dist default — inference is `/oss-init`'s job, per the `maturity` precedent); `homebrew` installer ⇒ a valid `homebrew_tap`; a tap without a `homebrew` installer is a *warning* (dead config); a distribution block is forbidden at `maturity: spike` (mirrors the `release.model: auto`-on-spike floor — a distribution ships public binaries); `homebrew_tap` validated to the GitHub name charset.

**`schema_version`: kept at 1 (NOT bumped).** The addition is a purely additive optional top-level field. The project's migration rule (AGENTS.md §migration) bumps `schema_version` only on a **breaking** change; a new optional field is the exact case the `extra_fields` forward-compat mechanism is built to absorb — an older reader preserves the unknown `distribution` key and emits a visible `unknown field(s) preserved (forward-compat)` warning rather than failing, and every existing registry-only contract's shape/validation is unchanged. Bumping `KNOWN_SCHEMA_VERSION` would make old binaries **refuse the whole contract**, discarding the registry-publish path they still handle correctly, over one advisory field they were already blind to (cargo-dist previously lived only in prose). The `/llm-review` panel split on this (2 for bump, 2 against); the reasoning is documented on `KNOWN_SCHEMA_VERSION` and in `history/review-contract-cargo-dist.md`. The wire-envelope version (`crate::SCHEMA_VERSION`) and `CONTRACT_SCHEMA_VERSION` are unaffected — this adds a key under `data`, not a new envelope.

**`SEAL_VERSION`: bumped 1→2.** The content-addressed `plan_id` pre-image (`plan.rs`) embeds the whole serialized `Contract`, so adding `distribution` (`null` for registry-only) changes the hash of every plan. Per the golden-vector test's own instruction, a deliberate pre-image change bumps `SEAL_VERSION` and updates the golden digest — done. Practical blast radius is ~nil (releases are hand-driven pre-1.0). Improving the post-bump drift diagnostic (`verify()` + recording `seal_version` on `ReleasePlan`) is spun off — it touches the release-engine seam.

**Spin-offs filed from the review:** SEAL/verify drift DX; monorepo `Vec<Distribution>` + package association; plan pre-image projection (vs whole-`Contract` hash); nested `distribution` `extra_fields`.

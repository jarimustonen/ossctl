---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: fixed
priority: normal
commits:
- hash: c8354e2
  summary: fix(facts) pre-1.0 full-release-infra reaches production
- hash: 55caf51
  summary: review(facts) harden ZeroVer path — 0.1.0 floor + release cadence
closed: 2026-08-04
---

# maturity inference: production-grade pre-1.0 repos can't reach `production` tier

_Source: ossctl facts / inferred_maturity truth table_

## Description

Surfaced running `/oss-init` on the **issuectl** repo (v0.6.x).

## Observed
`ossctl facts` returned `inferred_maturity: mvp` despite the repo being production-grade by infrastructure: CI, cargo-dist release pipeline, Homebrew tap, dual crates.io publish, hand-curated CHANGELOG, SECURITY.md, dependabot, 3 recent-year committers, 13 semver tags. The **only** failing signal for `production` was `has_ge_1_0_release: false` (the project is deliberately still 0.6.x).

## Expected / suggestion
The truth table gates `production` on `≥2 recent committers AND ≥1.0 release AND CI`, which conflates *version number* with *release maturity*. Many serious projects deliberately stay 0.x (ZeroVer). Options:
- Decouple release/quality maturity from the ≥1.0 version gate (e.g. treat a full release pipeline + CI + dependabot as a production signal regardless of major version), OR
- Add an explicit signal/override so a pre-1.0 repo with complete release infrastructure can infer (or be nudged toward) `production` without `--maturity` having to force it.

## Impact
Minor but real: a production-quality 0.x repo under-describes itself as `mvp`, so `/oss-ci` etc. right-size to a leaner tier than the project actually runs. Workaround today is `--maturity production`, but that then reports the missing coverage/security-lint gates as gaps (which may be desirable or not).

## Resolution

**Chosen: option (a) — decouple release maturity from the ≥1.0 version gate**, implemented as a *second, substantive path* to `production` rather than a wholesale replacement of the gate.

New truth-table rule (`crates/ossctl-core/src/facts/mod.rs`):

```
production = committers_recent_year >= 2
          && has_ci
          && ( has_ge_1_0_release              // path 1 — the original gate, unchanged
             || zerover_release_evidence )     // path 2 — NEW, for ZeroVer projects
zerover_release_evidence = dependency_bot.is_some()
                        && shipped_release_tags >= 2   // release cadence:
                                                       //   ≥2 non-prerelease ≥0.1.0 SemVer tags
```

`shipped_release_tags` counts non-prerelease SemVer tags at `>=0.1.0` (a `0.0.x` tag is SemVer's initial-scratch space; a `-rc`/`-alpha` is not shipped). `has_ci` is not repeated inside `zerover_release_evidence` — it is the always-required outer term for both paths.

### Why (a) over (b)
Option (b) — an explicit override field/flag — would add a *new authored input* to the wire contract and shift the burden back onto the human (essentially a nicer `--maturity production`). The issue's own framing is that the truth table *conflates version number with release maturity*; the honest fix is to correct the inference, not to bolt on an escape hatch. Decoupling keeps the signal automatic and self-explaining.

### Why this stays auditable and doesn't inflate a bare repo
- **No new wire field, no `schema_version` bump.** Every input is an *already-emitted* fact: `has_ci`, `dependency_bot`, `tags` (from which `shipped_release_tags` recomputes with the same SemVer parse), `committers_recent_year`, `has_ge_1_0_release`. A consumer reading the existing `ossctl facts` JSON can re-derive why `production` flipped. The JSON shape is unchanged, honoring the "prefer no shape change" constraint (§10). The `MaturitySignals::production` docstring is explicit that the shipped-release count is recomputed from `tags`, not a first-class field, and that these are presence/name heuristics — not proofs.
- **Substantive: a maintained release process, not "just a tag."** The ZeroVer path demands CI **and** a dependency-update bot **and** a *release cadence* (≥2 shipped ≥0.1.0 releases — two prove the project iterated a release more than once, which a lone `git tag` can't fake) **and** ≥2 recent-year committers. The `0.0.x` floor closes the "empty workflow + empty `dependabot.yml` + `v0.0.1`" inflation. Guard cases are unit-tested: `bare_0x_with_only_a_tag_is_not_production`, `zerover_v0_0_x_tag_is_not_a_shipped_release`, `pre_1_0_release_infra_requires_release_cadence`, `pre_1_0_release_infra_requires_dependency_bot`, `pre_1_0_release_infra_ignores_prerelease_tags_for_cadence`, `pre_1_0_release_infra_requires_two_recent_committers`. Positive cases: `pre_1_0_with_full_release_infra_is_production` (the issuectl-shaped repo) and `renovate_unlocks_the_zerover_release_path`. Regression guard that the ≥1.0 path needs no bot: `ge_1_0_release_reaches_production_without_a_dependency_bot`.

### Deliberate design points (raised in the `/llm-review` panel)
- **The dependency-bot requirement below 1.0 (but not at ≥1.0) is intentional, not an arbitrary burden.** `>=1.0` is itself a maintainer's stability *declaration*; below 1.0 that declaration is absent, so the ZeroVer path requires compensating evidence of a maintained process. This is documented in the code comment so the asymmetry reads as a choice.
- **Kept the dependency bot as a signal** despite panel pushback to drop it, because the issue explicitly names "CI + a dependency bot" as the substantive substitute — and it is paired here with the (harder-to-fake) release-cadence signal.

### Considered and rejected
Detecting a dedicated release-automation *workflow file* (scanning `.github/workflows` for `cargo-dist`/`cargo publish`/tag triggers). Rejected: it (1) would require a new emitted fact → a global `SCHEMA_VERSION` bump with large blast radius (envelope version + `SUPPORTED_SCHEMAS` + contract-doc compatibility), against "prefer no shape change", and (2) is *weaker* evidence than a real release cadence — a workflow can exist yet never have cut a release, whereas ≥2 shipped tags prove releases actually happened.

### Deferred (out of scope — noted for follow-up)
- **Committer-identity inflation** (`git shortlog` counts `(name,email)` pairs, so aliases / `[bot]` accounts can pad `committers_recent_year >= 2`). Pre-existing, shared by the old rule; excluding bot identities / normalizing would change `spike`/`mvp` too and belongs in its own issue.
- **A `maturity_ruleset` policy-version field** so consumers can distinguish inference-algorithm versions at a fixed `schema_version`. Sound idea, but adding it is itself the wire-shape change this issue avoids and needs a protocol-level decision.

`audit` is unaffected: it gates recommended artifacts on `contract.maturity` (the human-reviewed dial), not on `facts.inferred_maturity` directly — so broadening the inference only changes what `/oss-init` *drafts*, which the human still approves.

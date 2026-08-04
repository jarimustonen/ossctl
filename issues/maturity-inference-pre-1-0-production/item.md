---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: in-progress
priority: normal
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
             || full_release_infra )           // path 2 — NEW, for ZeroVer projects
full_release_infra = has_ci
                  && dependency_bot.is_some()
                  && has_stable_semver_tag      // ≥1 non-prerelease SemVer tag
```

### Why (a) over (b)
Option (b) — an explicit override field/flag — would add a *new authored input* to the wire contract and shift the burden back onto the human (essentially a nicer `--maturity production`). The issue's own framing is that the truth table *conflates version number with release maturity*; the honest fix is to correct the inference, not to bolt on an escape hatch. Decoupling keeps the signal automatic and self-explaining.

### Why this stays auditable and can't be gamed
- **No new wire field, no `schema_version` bump.** Every input to `full_release_infra` is an *already-emitted* fact: `has_ci`, `dependency_bot`, `tags` (from which `has_stable_semver_tag` is recomputable), `committers_recent_year`, `has_ge_1_0_release`. A consumer reading the existing `ossctl facts` JSON can re-derive exactly why `production` flipped. The JSON shape is unchanged, honoring the "prefer no shape change" constraint (§10).
- **Substantive, not "just a tag."** The second path demands the full trifecta — working CI **and** automated dependency hygiene (a dependency bot) **and** an *actually shipped* release (a non-prerelease SemVer tag) — plus active co-maintenance (≥2 recent-year committers). A bare 0.x repo carrying only a tag has no CI and no bot, so it stays `mvp`. Guard cases are unit-tested: `bare_0x_with_only_a_tag_is_not_production`, `pre_1_0_release_infra_requires_dependency_bot`, `pre_1_0_release_infra_requires_a_shipped_stable_tag`, `pre_1_0_release_infra_requires_two_recent_committers`. The positive case (the issuectl-shaped repo) is `pre_1_0_with_full_release_infra_is_production`.

### Considered and rejected
Detecting a dedicated release-automation *workflow file* (scanning `.github/workflows` for `cargo-dist`/`cargo publish`/tag triggers) as the "real release-automation" signal. Rejected because it (1) would require a new emitted fact → a global `SCHEMA_VERSION` bump with large blast radius (envelope version + `SUPPORTED_SCHEMAS` + contract-doc compatibility), against the "prefer no shape change" constraint, and (2) is *weaker* evidence than what we already have — a workflow can exist yet never have cut a release, whereas a shipped non-prerelease tag proves a release actually happened. A dependency bot + a shipped stable tag is the stronger, already-visible composite.

`audit` is unaffected: it gates recommended artifacts on `contract.maturity` (the human-reviewed dial), not on `facts.inferred_maturity` directly — so broadening the inference only changes what `/oss-init` *drafts*, which the human still approves.

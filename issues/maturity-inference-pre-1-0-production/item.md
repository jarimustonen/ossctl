---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
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

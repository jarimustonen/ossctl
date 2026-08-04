---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: fixed
priority: high
epic: ossctl-phase4-build
commits:
- hash: b815583
  summary: add distribution.platforms cross-platform target set (schema_version stays 1, additive)
- hash: 34d04e0
  summary: apply /llm-review findings (empty->error, honest triple validator, tempered docs, 2 spin-offs)
closed: 2026-08-04
---

# Distribution block needs a cross-platform target set with a Mac+Linux default

_Source: cross-platform install requirement (Mac+Linux) — user directive_

## Description

REQUIREMENT (user directive): all OSS software the /oss-* family produces MUST install on both macOS AND Linux. The contract's `Distribution` block ({adapter, gh_releases, installers, homebrew_tap}) has NO platform-target field, so a downstream project can't express WHICH platforms its binaries cover and nothing guarantees Linux. KEYSTONE fix: add a platform-target set (Rust target-triples) to `Distribution` in crates/ossctl-core/src/contract/schema.rs + normalize.rs, defaulting to a CROSS-PLATFORM set — macOS (aarch64+x86_64) and Linux (aarch64+x86_64, prefer musl for static/glibc-free), optionally Windows. Normalize/validate the triples; preserve the canonical-JSON schema-versioned contract (bump schema_version if the shape breaks; a new optional field with a cross-platform default should be additive — follow the project rule). This default is what makes every downstream cargo-dist distribution cover Linux by default. Blocks #2/#3/#5 which read this field.

## Resolution (fixed)

**Field name:** `distribution.platforms` — a `Vec<String>` of Rust target-triples. `targets` was already taken (the registry-publish `Vec<Target>`), and `binary_targets` reads awkwardly next to it; `platforms` is unambiguous and matches how the cargo-dist ecosystem talks about the set.

**Default (omitted/null → this set):**
```
aarch64-apple-darwin
x86_64-apple-darwin
aarch64-unknown-linux-musl
x86_64-unknown-linux-musl
```
macOS + Linux, `musl` over `gnu` (for a pure-Rust CLI a musl target links statically and sidesteps the glibc-version cliff — a C/native-dep repo may override to `-gnu`). Windows is a deliberate omission — a bonus a repo opts into by listing `x86_64-pc-windows-msvc` explicitly, never the default. The default always contains ≥1 Linux triple, so a distribution that OMITS the field covers Linux. (An explicit `[]` is a hard error, NOT defaulted — see the Review section; only omitted/null default. This corrects the first-draft behavior.)

**Validation:** each triple is STRUCTURALLY validated (`looks_like_target_triple`: 2–4 `-`-separated `[a-z0-9_.]` components) — rejects malformed/uppercase/injection/wrong-shape strings and accepts real dotted targets (`thumbv8m.main-none-eabi`); it is a well-formedness gate, not semantic rustc-validity (structurally-valid nonsense like `aa-bb` passes — the toolchain is the final authority). The OS component stays inspectable so the downstream `audit-cross-platform-gap` issue can flag Linux-less explicit sets. Explicit lists are de-duplicated preserving author order. This issue only guarantees the field is present + well-formed; it does NOT implement the Linux-coverage audit.

**schema_version decision: STAYS 1 (additive).** `platforms` is a NEW optional key added inside the already-optional `distribution` block — no existing field is renamed, removed, or re-meant. A reader keying into `distribution.adapter`/`installers`/… is unaffected; the new key never existed before, so there is no prior shape for it to be incompatible with (a pure addition, exactly the case `KNOWN_SCHEMA_VERSION`'s doc and the migration rule call additive-safe). Registry-only contracts (`distribution: null`) are wholly unchanged. The one nuance — omitted resolves to a populated set rather than `null` — does not make it breaking: it only ever ADDS a key, never alters an existing one.

Also updated the oss-init `SKILL.template.md` §4 field table so the generator documents `platforms` + its default (kept in lockstep with the binary).

## Review (4-model /llm-review, 2 rounds) — findings applied

Assessment persisted at `history/assessment-distribution-platforms.{json,md}`; report at `history/review-distribution-platforms.md`. Reviewers withdrew all serde/`schema_version`-bump findings once given FACT A (the canonical model is **Serialize-only** — no `Deserialize` anywhere; skills read the JSON textually via `ossctl contract show --json`, so the additive `schema_version=1` call is correct and no reader-crash path exists). Applied fixes:

- **Empty `platforms: []` is now a hard error** (was: silently defaulted). All 4 reviewers' top finding: silently substituting the default surprises the author with targets they never listed AND erases the `[]`-vs-omitted distinction the downstream `audit-cross-platform-gap` inspects. Only omitted/null yield the default now.
- **Triple validator made honest**: renamed `is_target_triple` → `looks_like_target_triple`, now allows `.` in components (real targets like `thumbv8m.main-none-eabi` were wrongly rejected), and the doc/error state plainly it is a STRUCTURAL well-formedness gate, not semantic rustc-validity (rejected the allowlist alternative — staleness/maintenance burden beyond scope).
- **Tempered over-claiming docs**: "every distribution covers Linux" → "every *omitting* distribution"; musl doc no longer implies a guaranteed static build (notes C/native-dep repos may override to gnu); SKILL template corrected ("omitted → default; explicit empty rejected").

Deferred to follow-up issues (filed, epic ossctl-phase4-build): `distribution-platforms-adapter-neutral` (platforms is Rust-triple-shaped but goreleaser uses GOOS/GOARCH — needs adapter-gating or an adapter-neutral remodel touching the coordinator seam) and `distribution-installer-platform-crosscheck` (warn when `installers` imply an OS the `platforms` set lacks — belongs with the coverage-aware audit work). Rejected: canonical-sorting `platforms` (author-order preservation is consistent with the sibling `targets` list; enum lists sort only because they have a defined enum order that open triple strings lack).

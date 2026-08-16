---
created: 2026-08-07
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
epic: ossctl-phase4-build
related: ['@distribution-installer-platform-crosscheck']
closed: 2026-08-16
---

# distribution: unify installer/target OS-compatibility into a structured, adapter-aware classifier

## Description

# distribution: unify installer/target OS-compatibility into a structured, adapter-aware classifier

_Source: /llm-review + /assess-findings spin-off (F6) from `distribution-installer-platform-crosscheck`._

## Problem

The installer↔platform cross-check warning (landed in `parse_distribution`) encodes a
SECOND, ad-hoc compatibility model in the normalizer: `installer_os_need` (a small
`Installer → OsNeed` table) plus stringly-typed OS predicates (`triple_os` / `is_*_triple`,
matching the 3rd triple component). It is:

- **Independent of the artifact generator** (`ossctl dist generate` / the cargo-dist adapter):
  if cargo-dist changes which installers/targets it supports, this table silently drifts.
- **Adapter-blind**: it ignores `distribution.adapter`, assuming every adapter gives `msi`
  and `homebrew` the same OS semantics.
- **Coarse/heuristic**: the OS-component match accepts exotic Linux-kernel targets that
  Linuxbrew does not serve (e.g. `*-linux-ohos`), and the warning wording ("has nothing to
  install") asserts more than a token match can prove.

## Suggested direction

Introduce a single shared target/OS classifier (a structured `TargetTriple` type, or adopt
`target-lexicon`) used by BOTH the normalizer cross-check AND the dist generator, and make
installer→OS compatibility adapter-aware:

```rust
fn installer_requirement(adapter: DistributionAdapter, installer: Installer) -> InstallerRequirement
```

Soften the heuristic warning wording ("may have no compatible artifact to install") until the
classification is authoritative against the generator.

## Why its own issue

Cross-cutting: touches `contract/normalize.rs`, the `dist` generator, and the adapter model
(ADR-0002). Needs its own design (shared type + interface); deliberately NOT bundled into the
warning diff, whose positional `triple_os` fix already covers the common desktop triples and
the Android/iOS false-negative. Non-urgent quality lift for the open triple space.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Quality lift for a broader adapter model, but no current consumer needs it. Closing under the no-backlog policy.

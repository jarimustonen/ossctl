---
created: 2026-08-02
updated: 2026-08-02
type: bug
status: in-progress
priority: high
epic: ossctl-phase4-build
---

# facts: enumerate Cargo workspace member packages instead of null

## Description

Surfaced by the /oss-init dogfood (stint #9). Repo-fact detection does NOT descend into Cargo workspace members.

SYMPTOM: on a virtual-workspace repo (root Cargo.toml has [workspace] + members, no [package]), 'ossctl facts' / infer-repo-facts.py emit a single packages[] entry with package: null and version: null. Observed on ossctl itself:
  packages: [ { ecosystem: rust, manifest: Cargo.toml, package: null, version: null } ]
The real packages (crates/ossctl-core, crates/ossctl-cli with bin ossctl) are never enumerated.

ROOT CAUSE: crates/ossctl-core/src/facts/mod.rs, detect_manifests() (~line 219-234) + parse_cargo() (~line 289). It sees the virtual workspace, correctly marks the repo rust, but pushes one package entry whose name comes from a [package] block that does not exist -> None. It never reads the [workspace].members list to open the member manifests.

FIX: when the root Cargo.toml is a virtual workspace ([workspace] with members[] and no [package]), read each member's Cargo.toml and emit one packages[] entry per member with its real name + version, honoring version.workspace = true inheritance from [workspace.package] (and license.workspace etc. as relevant to what facts records). A real (non-virtual) root [package] keeps today's behavior. Keep binary-ecosystem logic unchanged (binary only when NO package ecosystem).

CONSTRAINTS:
- This is a bug in ossctl's OWN detection for the workspace shape ossctl itself uses; many real Rust projects are workspaces, so it must work before ossctl audits/cuts them.
- The Python infer-repo-facts.py (in the oss-init skill, homebase) has the SAME gap conceptually, but THIS issue is scoped to the Rust ossctl-core port only. Note the Python parity gap in a follow-up if you like; do not edit homebase from here.
- Keep the canonical protocol DTO (crates/ossctl-core/src/protocol/facts.rs) shape stable if possible; Package already has Option<String> name + version, so detection logic should suffice. protocol/* is a TRUE shared-logic hot file - only touch it if unavoidable, and if so note why.

Green gate: cargo fmt --check, clippy -D warnings, test --workspace, build --workspace. Run /llm-review + /assess-findings before merge (production detection logic).

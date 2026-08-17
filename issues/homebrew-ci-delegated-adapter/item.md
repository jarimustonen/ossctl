---
created: 2026-08-17
updated: 2026-08-17
type: feature
status: open
priority: normal
lane: release-safety
---

# homebrew target needs a CI-delegated adapter (cargo-dist publish-jobs owns the tap)

## Description

Cross-repo audit finding (2026-08-17, stint #22). Three fleet repos (glasspad, orchestratectl, issuectl) publish their Homebrew formula via cargo-dist's publish-homebrew-formula CI job (dist-workspace.toml: installers include homebrew, publish-jobs = [homebrew], tap = ...). Their formulas are written by CI on every tag — unmarked, and correctly so. But the contract schema can only express a homebrew target as adapter homebrew-tap/homebrew-core, which the ENGINE writes in its dist phase — so declaring the homebrew target in those repos creates a DOUBLE WRITER (engine tap-write + cargo-dist job racing on the same formula, and the engine's marker check refuses the CI-written formula), while NOT declaring it under-declares the real surface and (per D4) blocks engine cuts when distribution.homebrew_tap is set. glasspad's contract has this double-writer shape TODAY. Fix: support a CI-delegated homebrew target — {registry: homebrew, adapter: cargo-dist} (mirroring the gh-releases delegation pattern: is_ci_delegated, engine never writes the tap, no marker requirement) — which the verify phase still OBSERVES (tap formula carries the released version; marker check applies only to homebrew-tap engine-owned targets — verify already fetches the formula, only the marker assertion must be conditional on the adapter). Then all four public repos can declare their full surface uniformly and D4/verify cover the tap leg everywhere. See scratchpad cross-repo recon + homebase cross-repo-release-standardisation.

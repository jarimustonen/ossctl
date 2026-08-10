# Root-cause analysis & fix — `release-cut-publish-noop`

## What the real cut path actually does (traced)

The real `ossctl release cut` / `resume` CLI path injects a **genuine subprocess
runner** (`RealCommandRunner`, `crates/ossctl-cli/src/sys.rs`) all the way down to
`CargoAdapter::publish` (`crates/ossctl-core/src/release/adapters/cargo.rs`), which
runs a real `cargo publish --registry crates-io -p <pkg>` via `run_all`. There is
**no dry-run/echo/no-op runner in the code path** — prime suspect #1 (a no-op
runner reaching the real publish path) is **not present in the source**.

## The version-drift footgun (found + fixed)

`cargo publish` uploads the version **already in the tree `Cargo.toml`**. The
engine threads the operator-supplied `--version` into the sealed plan, and
`coordinator::resolve_target_plans` sets each `AdapterTarget.version =
plan.version.clone()` (coordinator.rs:364) — the `--version` value, **not** the
manifest version. So the target's idempotency probe (`is_published`) and its
`PublishReceipt` are recorded at `--version`, while `cargo publish` uploads the
manifest version. When the two drift, nothing lands at the requested version and
the receipt is fictional.

`cut --version X.Y.Z` **does not bump** the manifest (the issue's "adjacent
friction" note). The `--version` flag strongly implies the engine manages the
version; it does not. This is a real, reproducible footgun.

**Fix:** `plan::check_version_matches_tree(contract, facts, version)` — for every
target whose resolved package declares a version in `facts` (facts resolves Cargo
workspace members incl. `version.workspace = true` inheritance), that version must
equal `--version`. Wired as a fast, pre-publish `version_mismatch` refusal into
**`plan`, `cut`, and `resume`**. The `resume` wiring matters specifically because
a manifest-version edit between a failed cut and its resume does **not** change
`plan_id` (manifest versions are not part of the content address), so the resume
`plan_stale` drift check would otherwise miss it.

## Nuance on the EXACT reported timeout (honest limitation)

The reported failure was `issuectl-core@0.8.1 not visible on the registry index
within 300s` while cutting the dependent `issuectl`. The dependency index-wait
(`cargo.rs::publish` → `wait_for_index(dep.name, dep.version)`) uses
`dep.version` = the **`cargo metadata`** (manifest) version, **not** `t.version`.
So *pure* `--version` drift would make the dependent wait for the manifest version
(which was published), i.e. it would **not** by itself produce a `@0.8.1` timeout.

The exact `core@0.8.1` timeout is most consistent with the **dependency's own
publish returning success without actually uploading** `core@0.8.1` — i.e. either:
- `cargo publish --registry crates-io -p issuectl-core` exited 0 without uploading
  in that environment (a registry/token/`--registry crates-io` credential
  resolution difference from the operator's manual `cargo publish -p issuectl-core`
  that worked), or
- `issuectl-core` was **not declared as its own release target**, so nothing ever
  published it and the dependent timed out waiting.

Both are environment/config conditions that **cannot be reproduced locally** (no
crates.io credentials here). Per the issue's own guidance ("if it turns out to be
an environment/registry-token issue, do not force a fix — keep the drift guard +
mock-registry test"), no speculative code change was made for that path.

## Deliverables (all met)

- `--version` drift guard on `plan`/`cut`/`resume` with an actionable message.
- Mock-registry integration test (`cut_actually_publishes_both_crates_to_the_mock_registry`)
  that asserts the versions **actually appear on the registry** (not merely cargo
  exit 0), each with a per-member receipt; the publishing runner uploads the
  version its served `cargo metadata` declares. Companion test documents that a
  no-op publish leaves the registry empty though the cut is green.
- Unit tests for the guard (match / drift / multi-member / skip-no-version).

## Deferred (spinoff candidates from llm-review)

1. **Post-`cargo publish` self-visibility check before journaling a receipt**
   (GPT-5.6, Opus). The adapter records a receipt on `cargo publish` exit 0 with
   no confirmation the target's *own* version landed — the strongest production
   hardening against a silent no-op upload. Behavior change on the irreversible
   phase (a lagging index would fail the cut), so deferred for a deliberate call.
2. **Fail-closed guard for manifest-versioned ecosystems** (all 4 reviewers). The
   guard `continue`s when a target's package has no detected manifest version;
   for rust this is effective, but node/python/unresolved packages fall open.
   Needs an ecosystem/adapter "version-source" capability model to distinguish
   "no manifest version by design" (homebrew/binary/cargo-dist) from "detector
   failed" — a bigger design change (see #4).
3. **Idempotency short-circuit can fabricate a receipt** (Opus). `is_published`
   true → return a receipt without publishing. This is the deliberate resume
   idempotency design; moving cross-run idempotency into resume/reconcile and
   digest-authenticating the skip is a design change with real blast radius.
4. **Drop `--version` / read the version from the manifest** (Gemini, Opus).
   Eliminates the two-masters footgun entirely (the version becomes a projection
   of facts, not an input). Or, alternatively, seal per-target *effective* publish
   versions resolved via each adapter's own metadata.
5. **Dirty-tree / clean-checkout execution + TOCTOU** (GPT-5.6). A pre-existing
   documented gap: cut/resume publish from a mutable working tree, so the guard is
   a point-in-time check. Executing from a clean checkout of the sealed `HEAD` is
   the robust fix.

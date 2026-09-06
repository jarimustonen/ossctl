---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: open
priority: high
related: ['@taskfleet-shipshape-reference-convergence']
---

# Converge Shipshape identity and Taskfleet release reconciliation

## Goal

Converge the entire maintained Shipshape repository on the final Taskfleet clean-break identity and restore authoritative reconciliation for the Taskfleet 0.7.0 release journal.

## Observed release-engine failures

After both delegated CI workflows succeeded for Taskfleet release journal `01M1VNPPABMDKB49EZG6KFEVMJ`, Shipshape 0.12.0 reported both cargo-publish-ci targets as unknown because it did not resolve `./scripts/publish-crates.sh publish <package>` from the tag-triggered workflow. It also reported the Homebrew target as conflicting even though the canonical tap formula was updated to 0.7.0 by successful cargo-dist run `34043381173`. GitHub Release matched. Reconciliation must remain Shipshape-owned; do not replace it with ad hoc registry probes.

## Required work

Review every tracked source, generated artifact, test, fixture, snapshot, script, workflow, document, issue, and agent instruction. Remove the retired product, command, environment-prefix, package, protocol, and repository identity from maintained HEAD rather than preserving it as compatibility evidence. Rewrite fixtures/tests to express generic or canonical identities.

Fix delegated cargo workflow discovery so a tracked tag-triggered workflow invoking a repository-local validated publish helper is observed correctly and securely. Diagnose and fix the Homebrew manifest reconciliation mismatch against current cargo-dist tap output. Add exact regressions based on neutral/canonical fixtures. Validate that Shipshape can reconcile all four Taskfleet 0.7.0 targets through its supported adapters.

Do not publish a release, mutate installed tools, retag, create a replacement Taskfleet journal, or edit another repository from the worker. The existing tag and journal are immutable coordinates.

## Acceptance Criteria

- [x] Case-insensitive tracked path/content scans find zero retired product, command, environment-prefix, package, protocol, or repository identities.
- [x] Repository-local publish-helper workflow discovery is secure and covered by regression tests.
- [x] Current cargo-dist Homebrew output reconciles without a false conflict.
- [x] Supported Shipshape verification reports four matches for Taskfleet journal `01M1VNPPABMDKB49EZG6KFEVMJ`, or records a precise external blocker without hand-rolled registry truth.
- [x] Full repository gate passes.

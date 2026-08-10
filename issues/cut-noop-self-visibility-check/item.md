---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: open
priority: high
epic: ossctl-phase4-build
related: ['@release-cut-publish-noop']
---

# release cut journals a publish receipt without confirming the version actually landed (silent no-op upload)

## Description

The REAL fix for the release-cut-publish-noop no-op. cargo.rs::publish records a PublishReceipt on `cargo publish` exit 0 with NO confirmation the target's OWN just-published version actually landed on the registry index. A silent no-op upload (env/credential/registry-alias difference, or an under-declared target) therefore fabricates a receipt and the cut reports success while nothing shipped — exactly the issuectl 0.8.1 signature. FIX: after cargo publish, probe the registry for the target's own {name,version} and only journal the receipt once confirmed; fail the cut LOUDLY with a clear error if it never appears. Must reuse the existing bounded index-wait (wait_for_index) so normal crates.io propagation lag is tolerated — do NOT make normal cuts flaky with an instant probe; only a genuine never-appears no-op fails. Behavior change on the IRREVERSIBLE publish phase — maintainer-approved (Jari, 2026-08-10). Must not break ossctl's own self-cut. Add an integration test on the mock registry: a publish that does NOT upload must fail the cut (no fabricated receipt).

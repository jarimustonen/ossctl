---
created: 2026-08-10
updated: 2026-08-11
type: improvement
status: done
priority: normal
epic: ossctl-phase4-build
related: ['@cargo-publish-receipt-provenance-resume-safety']
commits:
- hash: '3871174'
  summary: digest-authenticate the cargo resume idempotency skip
- hash: 8ed5059
  summary: apply llm-review fixes (target-dir resolution, flag terminator, digest boundary validation)
- hash: 5b8c02f
  summary: record digest-authenticate follow-ups on the receipt-provenance cluster
closed: 2026-08-11
---

# is_published idempotency short-circuit records a receipt without digest-authenticating the already-published crate

## Description

On resume, cargo.rs::publish (~:347) skips an already-published crate and records a receipt WITHOUT proving the on-registry crate is byte-identical to what we intended — the one remaining receipt-without-fresh-upload path (same shape as the no-op bug just fixed). Digest-authenticate the skip: RegistryQuery returns the crate checksum, compare against the intended package's checksum before trusting the skip; else fail closed. Overlaps heavily with cargo-publish-receipt-provenance-resume-safety — likely folds into / pulls from that cluster; scope to the is_published skip and note if it needs the broader provenance work. Filed from the cut-noop /llm-review (stint #16).

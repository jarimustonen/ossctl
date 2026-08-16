---
created: 2026-08-04
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
epic: ossctl-phase4-build
related: ['@distribution-cross-platform-targets']
closed: 2026-08-16
---

# distribution.platforms is Rust-triple-shaped but goreleaser/manual don't consume triples

_Source: crates/ossctl-core/src/contract/schema.rs (Distribution)_

## Description

_Source: /llm-review spin-off (F7) from `distribution-cross-platform-targets`_

## Problem

`Distribution.platforms` (added in `distribution-cross-platform-targets`) holds Rust target-triples (e.g. `x86_64-unknown-linux-musl`). That is the exact vocabulary the **cargo-dist** adapter consumes, but `DistributionAdapter` also has `goreleaser` (Go — models `GOOS`/`GOARCH`/`GOARM`, not Rust triples) and `manual` (no builder consumes the field at all). A downstream `/oss-*` skill reading the same canonical JSON under `adapter: goreleaser` sees a triple set it must either reinterpret with a lossy mapping (`x86_64`→`amd64`, `aarch64`→`arm64`, vendor dropped, musl/gnu has no clean GoReleaser analogue) or ignore. The field's meaning silently depends on an adapter it does not encode.

## Options

1. **Adapter-gate** `platforms`: only allowed/defaulted when `adapter: cargo-dist`; error or warn otherwise.
2. **Adapter-neutral model**: `platforms: Vec<Platform>` where `Platform = { os, arch, libc? }`, mapped to Rust triples for cargo-dist and `GOOS`/`GOARCH` for goreleaser downstream.
3. **Adapter-tagged `Distribution`** enum with per-adapter platform shapes.

## Why its own issue

Touches `DistributionAdapter` semantics and the release-engine `coordinator`/`adapters` seam — a strictly-sequenced shared-logic file per AGENTS.md — and would ripple to the canonical-JSON schema (a `schema_version` decision). Well beyond the "add a Rust target-triple set" scope of the parent issue. The parent diff mitigates in-place by documenting `platforms` as Rust-triple-form (cargo-dist vocabulary).

## Decision (Jari, 2026-08-10) — DEFER

**Deferred.** Not worth the investment until a real non-Rust (e.g. Go/goreleaser) distribution consumer
appears — ossctl and its current users are Rust (cargo-dist), for which the Rust-triple `platforms`
field is exactly right. Revisit when the first non-Rust consumer surfaces. Stays in the backlog, does
not gate anything.

## Resolution

### 2026-08-16T08:34:10Z · @issuectl

Maintainer already deferred this until a real non-Rust distribution consumer exists. Closing to avoid a permanent backlog item.

---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@registry-query-http-client']
---

# npm registry query fetches the full packument; use the abbreviated form

## Description

## Description

Surfaced by the 4-model /llm-review of the http_get seam refactor
(`registry-query-http-client`). The `node` arm of `RealRegistryQuery` fetches the
**full** npm packument from `registry.npmjs.org/<package>` and reads the keys of
its top-level `versions` object. For very active packages (aws-sdk, typescript,
lodash) the full packument embeds READMEs + contributor data for every version and
can exceed the 10 MiB body cap in `http_get` — the read then errors, which fails
CLOSED to `unknown` (never a false "published"), so the contract holds, but such a
package can never be positively verified.

## Fix

Request the **abbreviated** packument (version keys + tarball URLs, low-KB):
`Accept: application/vnd.npm.install-v1+json`. This needs one request header, so
the `http_get(url)` seam must grow an optional header (e.g.
`http_get(url, accept: Option<&str>)`); the crates.io arm passes `None`. Keep the
fail-closed classification unchanged.

## Priority

Low — current behavior is safe (degrades to `unknown`), and `node` is not one of
ossctl's own release targets (ossctl is Rust). Do when a real node consumer appears.

Refs-Issue: registry-query-http-client

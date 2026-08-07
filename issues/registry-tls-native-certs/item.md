---
created: 2026-08-07
updated: 2026-08-07
type: improvement
status: open
priority: normal
epic: ossctl-phase4-build
related: ['@registry-query-http-client']
---

# registry HTTP client uses bundled webpki-roots; no system/native cert store

## Description

## Description

Surfaced by the 4-model /llm-review of the http_get seam refactor
(`registry-query-http-client`). `RealRegistryQuery::http_get` builds a `ureq`
agent on rustls + **webpki-roots** (the Mozilla root bundle baked into the
binary). Unlike the old `curl`/`npm` shell-outs — which used the OS trust store
and honored `SSL_CERT_FILE`/`CURL_CA_BUNDLE`/system-installed corporate CAs — the
bundled roots do **not** trust a corporate TLS-interception (MITM) proxy's private
CA. In such an environment every registry probe fails the TLS handshake →
transport `Err` → `unknown`.

This is **fail-closed-safe** (never a false "published"/"not-published"), but it is
a usability regression for developers behind an intercepting proxy: their
publish-state probes all read `unknown`.

## Decision context

webpki-roots is a **deliberate** choice for ossctl's static-musl cross-platform
distribution (no OpenSSL / OS-TLS linkage; deterministic bundled roots). So this is
a documented tradeoff, not a defect. Revisit only if real MITM-proxy usage shows
up.

## Options

- Enable a native-cert path (`rustls-native-certs` / ureq `native-certs`) so the
  agent also trusts the OS store — restores corporate-CA compatibility at the cost
  of a runtime read of the system trust store.
- Keep webpki-roots and document the limitation prominently.

## Priority

Low.

Refs-Issue: registry-query-http-client

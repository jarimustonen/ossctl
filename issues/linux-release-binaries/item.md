---
created: 2026-08-04
updated: 2026-08-05
type: feature
status: fixed
priority: normal
closed: 2026-08-05
---

# Publish Linux release binaries (x86_64/aarch64-unknown-linux-gnu)

## Description

## Problem

ossctl cannot be installed on Linux. The v0.1.0 GitHub release ships only:

```
ossctl-aarch64-apple-darwin.tar.gz
```

No `x86_64-unknown-linux-gnu`, no `aarch64-unknown-linux-gnu`, and no
`ossctl-installer.sh`. By contrast, its sibling CLIs already publish Linux
targets + a cargo-dist installer:

- issuectl v0.6.6 → `issuectl-x86_64-unknown-linux-gnu.tar.xz` + `issuectl-installer.sh`
- orchestratectl v0.1.0 → `orchestratectl-{x86_64,aarch64}-unknown-linux-gnu.tar.xz` + installer

## Impact

maintainer's homebase provisions issuectl/ossctl/orchestratectl on every machine via
`dotfiles/setup.d/*.sh` hooks. On macOS all three install via brew; on Linux
(haapa — Ubuntu x86_64 — and any future Linux clone) the issuectl and
orchestratectl hooks fall back to the cargo-dist release installer, but the
ossctl hook has to skip because there is no Linux artifact to install. So ossctl
(and the `/oss-*` skills it owns) is unavailable on Linux servers.

## Ask

Add `x86_64-unknown-linux-gnu` (and ideally `aarch64-unknown-linux-gnu`) to the
release build matrix and publish an `ossctl-installer.sh`, matching what
issuectl/orchestratectl already do (same cargo-dist setup). Also worth adding
`x86_64-apple-darwin` for Intel Macs — the release currently ships arm-only.

Once a Linux build exists, the homebase ossctl hook can be extended to the same
version-gated release-installer fallback the other two now use (that change is
trivial and tracked on the homebase side).

## Resolution (done)

ossctl v0.1.2 now ships Linux release binaries — `ossctl-{x86_64,aarch64}-unknown-linux-musl.tar.xz` + `ossctl-installer.sh` (also a Windows zip + PS1 installer). The homebase `dotfiles/setup.d/ossctl.sh` hook gained the brew(macOS)/release-installer(Linux) split (same as issuectl/orchestratectl), so ossctl is now provisioned on haapa and any Linux clone. Verified: ossctl 0.1.2 installed on haapa and across the Mac fleet via the fleet-updater.

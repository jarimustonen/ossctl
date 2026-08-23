#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=scripts/lib/v0110-release-verify.sh
source "$repo_root/scripts/lib/v0110-release-verify.sh"
for command in jq tar gzip shasum git cmp python3; do
  command -v "$command" >/dev/null || { echo "missing test command: $command" >&2; exit 69; }
done
work=$(mktemp -d "${TMPDIR:-/tmp}/v0110-verify-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

expect_failure() {
  local label=$1; shift
  if "$@" >/dev/null 2>&1; then
    echo "$label unexpectedly passed" >&2
    exit 1
  fi
}

# Modified archives and independently modified sidecars both fail closed.
printf 'pinned archive bytes' >"$work/archive"
archive_sha=$(v0110_sha256 "$work/archive")
printf '%s *fixture.tar.xz\n' "$archive_sha" >"$work/archive.sha256"
v0110_verify_checksum_file "$work/archive.sha256" "$work/archive" fixture.tar.xz "$archive_sha"
printf mutation >>"$work/archive"
expect_failure modified-archive v0110_verify_checksum_file \
  "$work/archive.sha256" "$work/archive" fixture.tar.xz "$archive_sha"
printf 'pinned archive bytes' >"$work/archive"
printf '%064d *fixture.tar.xz\n' 0 >"$work/archive.sha256"
expect_failure modified-sidecar v0110_verify_checksum_file \
  "$work/archive.sha256" "$work/archive" fixture.tar.xz "$archive_sha"

# The asset set is exact: neither a missing item nor an extra upload is accepted.
v0110_expected_asset_names | sort >"$work/expected-assets"
jq -Rn '[inputs | {name:.}] | {assets:.}' <"$work/expected-assets" >"$work/release.json"
v0110_verify_asset_set "$work/release.json" "$work/expected-assets"
jq '.assets[1:]' "$work/release.json" | jq '{assets:.}' >"$work/missing.json"
expect_failure missing-asset v0110_verify_asset_set "$work/missing.json" "$work/expected-assets"
jq '.assets += [{name:"surprise.bin"}]' "$work/release.json" >"$work/extra.json"
expect_failure extra-asset v0110_verify_asset_set "$work/extra.json" "$work/expected-assets"

# Source bytes may have a different gzip header, but archive paths stay under the
# release prefix and critical tracked files come from the immutable commit.
git archive --format=tar.gz --prefix=shipshape-0.11.0/ HEAD >"$work/source-good.tar.gz"
v0110_verify_source_archive "$work/source-good.tar.gz" HEAD "$repo_root"
mkdir -p "$work/wrong/shipshape-0.11.0"
printf 'wrong source\n' >"$work/wrong/shipshape-0.11.0/Cargo.toml"
cp "$repo_root/dist-workspace.toml" "$work/wrong/shipshape-0.11.0/dist-workspace.toml"
tar -czf "$work/source-wrong.tar.gz" -C "$work/wrong" shipshape-0.11.0
expect_failure wrong-source-content v0110_verify_source_archive \
  "$work/source-wrong.tar.gz" HEAD "$repo_root"
python3 - "$work/source-traversal.tar.gz" <<'PY'
import io, tarfile, sys
with tarfile.open(sys.argv[1], "w:gz") as tf:
    data = b"escape"
    item = tarfile.TarInfo("shipshape-0.11.0/../escape")
    item.size = len(data)
    tf.addfile(item, io.BytesIO(data))
PY
expect_failure source-path-traversal v0110_verify_source_archive \
  "$work/source-traversal.tar.gz" HEAD "$repo_root"

cat >"$work/installer" <<'EOF'
            _archive="shipshape-aarch64-apple-darwin.tar.xz"
            _archive="shipshape-aarch64-unknown-linux-musl.tar.xz"
            _archive="shipshape-x86_64-unknown-linux-musl.tar.xz"
EOF
printf '%s\n' shipshape-aarch64-apple-darwin.tar.xz \
  shipshape-aarch64-unknown-linux-musl.tar.xz \
  shipshape-x86_64-unknown-linux-musl.tar.xz | sort >"$work/archives"
v0110_verify_installer "$work/installer" "$work/archives" "$work/installer"
printf '%s\n' '            _archive="shipshape-x86_64-apple-darwin.tar.xz"' >>"$work/installer"
expect_failure intel-installer v0110_verify_installer "$work/installer" "$work/archives" "$work/installer"

# A compact cargo-dist fixture pins all meaningful schema while allowing only
# the observed host Cargo line, upload temp root, and source-gzip hash to vary.
cat >"$work/manifest.json" <<'EOF'
{
  "announcement_tag":"v0.11.0",
  "announcement_title":"0.11.0 - 2026-08-23",
  "announcement_is_prerelease":false,
  "releases":[{"artifacts":["source.tar.gz","source.tar.gz.sha256","shipshape-installer.sh","sha256.sum","shipshape-aarch64-apple-darwin.tar.xz","shipshape-aarch64-apple-darwin.tar.xz.sha256","shipshape-aarch64-unknown-linux-musl.tar.xz","shipshape-aarch64-unknown-linux-musl.tar.xz.sha256","shipshape-x86_64-unknown-linux-musl.tar.xz","shipshape-x86_64-unknown-linux-musl.tar.xz.sha256"]}],
  "artifacts":{
    "shipshape-aarch64-apple-darwin.tar.xz":{"kind":"executable-zip","target_triples":["aarch64-apple-darwin"],"checksum":"shipshape-aarch64-apple-darwin.tar.xz.sha256","checksums":{"sha256":"15c35f00196da6da1d5d852b99dd5de51d8f1895aa97207690814b581273c988"}},
    "shipshape-aarch64-unknown-linux-musl.tar.xz":{"kind":"executable-zip","target_triples":["aarch64-unknown-linux-musl"],"checksum":"shipshape-aarch64-unknown-linux-musl.tar.xz.sha256","checksums":{"sha256":"b510af6cefcdf59b0d18dc0176620e19f40d81e0a5fb5a259544eeaeb39057fa"}},
    "shipshape-x86_64-unknown-linux-musl.tar.xz":{"kind":"executable-zip","target_triples":["x86_64-unknown-linux-musl"],"checksum":"shipshape-x86_64-unknown-linux-musl.tar.xz.sha256","checksums":{"sha256":"83313adf0e1bc91cffb78fea7146f3ecc8280a58b57180c5a0d1cbb3989d9d86"}},
    "source.tar.gz":{"kind":"source-tarball","checksum":"source.tar.gz.sha256","checksums":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}
  },
  "assets":{"one":{"target_triples":["aarch64-apple-darwin"]},"two":{"target_triples":["aarch64-unknown-linux-musl"]},"three":{"target_triples":["x86_64-unknown-linux-musl"]}},
  "systems":{"build:global:":{"cargo_version_line":"cargo old"}},
  "upload_files":["/tmp/old/source.tar.gz","/tmp/old/source.tar.gz.sha256","/tmp/old/shipshape-installer.sh","/tmp/old/sha256.sum"]
}
EOF
jq '[.releases[0].artifacts[]]' "$work/manifest.json" >"$work/release-artifacts.json"
jq -n --args '$ARGS.positional' \
  15c35f00196da6da1d5d852b99dd5de51d8f1895aa97207690814b581273c988 \
  b510af6cefcdf59b0d18dc0176620e19f40d81e0a5fb5a259544eeaeb39057fa \
  83313adf0e1bc91cffb78fea7146f3ecc8280a58b57180c5a0d1cbb3989d9d86 >"$work/hashes.json"
jq '.systems["build:global:"].cargo_version_line="cargo new"
  | .upload_files=["/private/random/new/source.tar.gz","/private/random/new/source.tar.gz.sha256","/private/random/new/shipshape-installer.sh","/private/random/new/sha256.sum"]
  | .artifacts["source.tar.gz"].checksums.sha256="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"' \
  "$work/manifest.json" >"$work/generated.json"
v0110_verify_manifest "$work/manifest.json" "$work/generated.json" \
  "$work/release-artifacts.json" "$work/hashes.json" \
  aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
jq '.artifacts["intel"]={"kind":"executable-zip","target_triples":["x86_64-apple-darwin"],"checksum":"intel.sha256","checksums":{"sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}' \
  "$work/manifest.json" >"$work/intel-manifest.json"
expect_failure intel-manifest v0110_verify_manifest "$work/intel-manifest.json" \
  "$work/generated.json" "$work/release-artifacts.json" "$work/hashes.json" \
  aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

# A meaningful, non-host field remains a conflict.
jq '.announcement_title="changed"' "$work/generated.json" >"$work/changed.json"
expect_failure meaningful-manifest-change v0110_verify_manifest "$work/manifest.json" \
  "$work/changed.json" "$work/release-artifacts.json" "$work/hashes.json" \
  aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

echo "v0.11.0 Release semantic verification fixtures pass."

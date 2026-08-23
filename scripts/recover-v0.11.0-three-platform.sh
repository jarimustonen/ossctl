#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=scripts/lib/github-content-decode.sh
source "$script_dir/lib/github-content-decode.sh"

# Safe, pinned fallback for the cancelled v0.11.0 cargo-dist run. By default this
# only prepares and validates local material. External writes require both
# --execute and the explicit environment acknowledgement below.
mode=prepare
if [[ ${1:-} == "--execute" ]]; then
  mode=execute
elif [[ ${1:-} != "" && ${1:-} != "--prepare-only" ]]; then
  echo "usage: $0 [--prepare-only|--execute]" >&2
  exit 64
fi
if [[ $mode == execute && ${SHIPSHAPE_V0110_RECOVER:-} != execute ]]; then
  echo "refusing external writes: set SHIPSHAPE_V0110_RECOVER=execute" >&2
  exit 64
fi

REPO=jarimustonen/ossctl
TAP=jarimustonen/homebrew-shipshape
TAG=v0.11.0
TAG_SHA=63a55a524bf5a08040b3447ede6c1985a2f177a9
WORKFLOW_RUN=32652510525
ENGINE_RUN=01M0QQMW8Y6SWRR2G383M0KVJX
DIST_VERSION=0.28.2
ABANDON_REASON="superseded four-platform plan; maintainer withdrew Intel macOS after cargo-dist workflow 32652510525 was cancelled"

platforms=(
  aarch64-apple-darwin
  aarch64-unknown-linux-musl
  x86_64-unknown-linux-musl
)
artifact_ids=(9496586399 9496606237 9496596260)
expected_sha256=(
  15c35f00196da6da1d5d852b99dd5de51d8f1895aa97207690814b581273c988
  b510af6cefcdf59b0d18dc0176620e19f40d81e0a5fb5a259544eeaeb39057fa
  83313adf0e1bc91cffb78fea7146f3ecc8280a58b57180c5a0d1cbb3989d9d86
)

for command in git gh jq curl unzip tar shasum cargo base64 tr; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 69; }
done
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
[[ -z $(git status --porcelain) ]] || { echo "repository must be clean" >&2; exit 65; }
workspace_version=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name == "shipshape-cli") | .version')
[[ $workspace_version == 0.11.0 ]] || { echo "workspace version is $workspace_version" >&2; exit 65; }
grep -qF "cargo-dist-version = \"$DIST_VERSION\"" dist-workspace.toml || {
  echo "dist-workspace.toml does not pin cargo-dist $DIST_VERSION" >&2; exit 65;
}
[[ $(git rev-parse "$TAG^{commit}") == "$TAG_SHA" ]] || { echo "local $TAG moved" >&2; exit 65; }
remote_tag_sha=$(git ls-remote origin "refs/tags/$TAG^{}" | awk '{print $1}')
[[ $remote_tag_sha == "$TAG_SHA" ]] || { echo "remote $TAG is absent or moved" >&2; exit 65; }
run_json=$(gh run view "$WORKFLOW_RUN" --repo "$REPO" --json conclusion,headSha,status)
jq -e --arg sha "$TAG_SHA" '.status == "completed" and .conclusion == "cancelled" and .headSha == $sha' <<<"$run_json" >/dev/null || {
  echo "workflow $WORKFLOW_RUN no longer has the pinned cancelled disposition" >&2; exit 65;
}
if [[ $mode == execute ]]; then
  default_branch=$(gh api "repos/$REPO" --jq .default_branch)
  canonical_sha=$(gh api "repos/$REPO/commits/$default_branch" --jq .sha)
  [[ $(git rev-parse HEAD) == "$canonical_sha" ]] || {
    echo "execute requires canonical $default_branch at $canonical_sha" >&2; exit 65;
  }
  gh run list --repo "$REPO" --workflow=ci.yml --branch "$default_branch" \
    --commit "$canonical_sha" --limit 1 --json conclusion \
    | jq -e 'length == 1 and .[0].conclusion == "success"' >/dev/null || {
    echo "CI is not green at canonical $default_branch $canonical_sha" >&2; exit 65;
  }
fi
observe_crates() {
  for crate in shipshape-core shipshape-cli; do
    curl -A 'shipshape-v0.11.0-recovery' -fsSL \
      "https://crates.io/api/v1/crates/$crate/0.11.0" \
      | jq -e '.version.num == "0.11.0"' >/dev/null || {
      echo "$crate 0.11.0 is not observable on crates.io" >&2; exit 69;
    }
  done
}
observe_crates

work=$(mktemp -d "${TMPDIR:-/tmp}/shipshape-v0110-recovery.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/local" "$work/release-files"

for i in 0 1 2; do
  platform=${platforms[$i]}
  id=${artifact_ids[$i]}
  zip="$work/$platform.zip"
  gh api -H 'Accept: application/vnd.github+json' \
    "repos/$REPO/actions/artifacts/$id/zip" >"$zip"
  mkdir "$work/local/$platform"
  unzip -q "$zip" -d "$work/local/$platform"
  archive="$work/local/$platform/shipshape-$platform.tar.xz"
  sidecar="$archive.sha256"
  [[ -f $archive && -f $sidecar ]] || { echo "artifact $id lacks $platform archive" >&2; exit 65; }
  actual=$(shasum -a 256 "$archive" | awk '{print $1}')
  [[ $actual == "${expected_sha256[$i]}" ]] || { echo "$platform checksum changed" >&2; exit 65; }
  (cd "$(dirname "$archive")" && shasum -a 256 -c "$(basename "$sidecar")")
  gh attestation verify "$archive" --repo "$REPO" >/dev/null
  cp "$archive" "$sidecar" "$work/release-files/"
done

# Generate only global cargo-dist artifacts from merged main's three-target
# config. cargo-dist obtains source.tar.gz from the immutable tag, while the
# installer and aggregate checksum consume the three observed local archives.
git clone -q --shared "$repo_root" "$work/source"
mkdir -p "$work/source/target/distrib"
find "$work/local" -type f -maxdepth 2 -exec cp {} "$work/source/target/distrib/" \;
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) dist_target=aarch64-apple-darwin ;;
  Darwin-x86_64) dist_target=x86_64-apple-darwin ;;
  Linux-x86_64) dist_target=x86_64-unknown-linux-musl ;;
  Linux-aarch64) dist_target=aarch64-unknown-linux-musl ;;
  *) echo "no pinned cargo-dist archive for this host" >&2; exit 69 ;;
esac
gh release download "v$DIST_VERSION" --repo axodotdev/cargo-dist \
  --pattern "cargo-dist-$dist_target.tar.xz" --pattern "cargo-dist-$dist_target.tar.xz.sha256" \
  -D "$work/dist-download"
(cd "$work/dist-download" && shasum -a 256 -c "cargo-dist-$dist_target.tar.xz.sha256")
tar -xJf "$work/dist-download/cargo-dist-$dist_target.tar.xz" -C "$work/dist-download"
dist_bin="$work/dist-download/cargo-dist-$dist_target/dist"
(
  cd "$work/source"
  "$dist_bin" build --tag="$TAG" --artifacts=global --output-format=json >"$work/global-manifest.json"
)
for name in shipshape-installer.sh; do
  cp "$work/source/target/distrib/$name" "$work/release-files/"
done
# Global generation must use merged main's three-platform installer config, but
# the public source artifact must be byte-derived from the immutable tag.
git archive --format=tar.gz --prefix=shipshape-0.11.0/ "$TAG_SHA" \
  >"$work/release-files/source.tar.gz"
source_sha=$(shasum -a 256 "$work/release-files/source.tar.gz" | awk '{print $1}')
printf '%s *source.tar.gz\n' "$source_sha" >"$work/release-files/source.tar.gz.sha256"
{
  for i in 0 1 2; do
    printf '%s *shipshape-%s.tar.xz\n' "${expected_sha256[$i]}" "${platforms[$i]}"
  done
  printf '%s *source.tar.gz\n' "$source_sha"
} >"$work/release-files/sha256.sum"

notes="$work/notes.md"
cat >"$notes" <<'EOF'
## Shipshape 0.11.0

This release completes the ossctl-to-Shipshape product migration. The maintained Cargo
packages are `shipshape-core` and `shipshape-cli`; the latter installs command
`shipshape`.

### Supported prebuilt platforms

This Release intentionally contains exactly macOS arm64 and Linux musl arm64/x86_64
archives. Intel macOS was withdrawn after the original cargo-dist run queued without
building it; Windows is also deliberately unsupported. Source installation with
`cargo install shipshape-cli` remains available. The tagged source changelog predates
that withdrawal and mentions the superseded Intel plan; this Release note is the binding
published support statement.
EOF

# Preserve cargo-dist's complete public schema while removing only the withdrawn
# local artifact records and replacing the stale four-platform announcement.
jq --rawfile notes "$notes" '
  .announcement_github_body = $notes
  | .releases[].artifacts |= map(select(contains("x86_64-apple-darwin") | not))
  | .artifacts |= with_entries(select(.key | contains("x86_64-apple-darwin") | not))
  | .artifacts["shipshape-installer.sh"].target_triples
      |= map(select(. != "x86_64-apple-darwin"))
' "$work/global-manifest.json" >"$work/release-files/dist-manifest.json"
if grep -n 'x86_64-apple-darwin' "$work/release-files/dist-manifest.json" \
  "$work/release-files/shipshape-installer.sh" "$work/release-files/sha256.sum"; then
  echo "prepared completion metadata or installer still claims Intel macOS" >&2
  exit 65
fi
for i in 0 1 2; do
  grep -qF "${expected_sha256[$i]} *shipshape-${platforms[$i]}.tar.xz" \
    "$work/release-files/sha256.sum" || {
    echo "aggregate checksum is missing ${platforms[$i]}" >&2; exit 65;
  }
done
# cargo-dist derives this archive from the announced tag. Check two tracked files
# byte-for-byte so a future cargo-dist behavior change cannot substitute main.
source_prefix=shipshape-0.11.0
tar -xOf "$work/release-files/source.tar.gz" "$source_prefix/dist-workspace.toml" \
  | cmp - <(git show "$TAG_SHA:dist-workspace.toml")
tar -xOf "$work/release-files/source.tar.gz" "$source_prefix/Cargo.toml" \
  | cmp - <(git show "$TAG_SHA:Cargo.toml")

formula="$work/shipshape.rb"
cat >"$formula" <<EOF
# Generated by shipshape; do not edit by hand (template-version: 2)
class Shipshape < Formula
  desc "Release & readiness coordinator"
  homepage "https://github.com/$REPO"
  version "0.11.0"
  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/$REPO/releases/download/$TAG/shipshape-aarch64-apple-darwin.tar.xz"
    sha256 "${expected_sha256[0]}"
  end
  if OS.linux? && Hardware::CPU.arm?
    url "https://github.com/$REPO/releases/download/$TAG/shipshape-aarch64-unknown-linux-musl.tar.xz"
    sha256 "${expected_sha256[1]}"
  end
  if OS.linux? && Hardware::CPU.intel?
    url "https://github.com/$REPO/releases/download/$TAG/shipshape-x86_64-unknown-linux-musl.tar.xz"
    sha256 "${expected_sha256[2]}"
  end
  license "MIT"
  def install
    bin.install "shipshape"
  end

  test do
    system bin/"shipshape", "version"
  end
end
EOF

if [[ $mode == prepare ]]; then
  echo "Prepared and verified the pinned three-platform recovery locally."
  echo "No journal or external channel was mutated. Re-run from merged clean main with:"
  echo "  SHIPSHAPE_V0110_RECOVER=execute $0 --execute"
  exit 0
fi

# Complete every read-only conflict check before making the old run terminal.
expected_assets="$work/expected-assets"
find "$work/release-files" -maxdepth 1 -type f -exec basename {} \; | sort >"$expected_assets"
release_state=absent
release_json="$work/release.json"
release_error="$work/release.error"
if gh api "repos/$REPO/releases/tags/$TAG" >"$release_json" 2>"$release_error"; then
  release_state=present
  jq -e --arg tag "$TAG" --arg title "0.11.0 - 2026-08-23" --rawfile notes "$notes" '
    .tag_name == $tag and .draft == false and .prerelease == false
    and .name == $title and .body == $notes
  ' "$release_json" >/dev/null || { echo "existing Release metadata conflicts" >&2; exit 65; }
  jq -r '.assets[].name' "$release_json" | sort >"$work/remote-assets"
  comm -13 "$expected_assets" "$work/remote-assets" | grep -q . && {
    echo "existing Release has unexpected assets" >&2; exit 65;
  }
  mkdir "$work/published"
  while IFS= read -r name; do
    [[ -n $name ]] || continue
    gh release download "$TAG" --repo "$REPO" --pattern "$name" -D "$work/published"
    cmp "$work/release-files/$name" "$work/published/$name" || {
      echo "existing Release asset differs: $name" >&2; exit 65;
    }
  done <"$work/remote-assets"
elif ! grep -q 'HTTP 404' "$release_error"; then
  cat "$release_error" >&2
  echo "could not classify GitHub Release as present or absent" >&2
  exit 69
fi

gh repo clone "$TAP" "$work/tap" -- --quiet
mkdir -p "$work/tap/Formula"
tap_state=absent
if [[ -e "$work/tap/Formula/shipshape.rb" ]]; then
  if cmp -s "$formula" "$work/tap/Formula/shipshape.rb"; then
    tap_state=matching
  elif [[ $(cat "$work/tap/Formula/shipshape.rb") == \
    '# Generated by shipshape; do not edit by hand (template-version: 2)' ]]; then
    # ADR-0005's authorized first-formula bootstrap is a marker-only file.
    tap_state=bootstrap
  else
    echo "tap formula conflicts" >&2
    exit 65
  fi
fi

cargo build --release -p shipshape-cli
engine_status=$(target/release/shipshape release show "$ENGINE_RUN" --json \
  | jq -r '.data.state.status')
case "$engine_status" in
  abandoned) ;;
  in_progress|failed)
    target/release/shipshape release abandon "$ENGINE_RUN" \
      --reason "$ABANDON_REASON" --json >/dev/null
    ;;
  *) echo "engine run has unexpected status: $engine_status" >&2; exit 65 ;;
esac

if [[ $release_state == absent ]]; then
  gh release create "$TAG" --repo "$REPO" --verify-tag --target "$TAG_SHA" \
    --title "0.11.0 - 2026-08-23" --notes-file "$notes"
fi
# Complete a partial prior upload without clobbering matching evidence.
while IFS= read -r name; do
  [[ -f "$work/published/$name" ]] && continue
  gh release upload "$TAG" --repo "$REPO" "$work/release-files/$name"
done <"$expected_assets"

# Write the engine-owned formula by ordinary fast-forward push. Existing content
# is accepted only when byte-identical, making retries safe after a partial run.
if [[ $tap_state != matching ]]; then
  cp "$formula" "$work/tap/Formula/shipshape.rb"
  git -C "$work/tap" add Formula/shipshape.rb
  git -C "$work/tap" -c user.name=shipshape -c user.email=shipshape@users.noreply.github.com \
    -c commit.gpgsign=false commit -m "shipshape 0.11.0"
  git -C "$work/tap" push origin HEAD
fi

# Observation-backed terminal checks: both crates, every supported archive and
# checksum, the no-Intel manifest, and the exact tap formula bytes.
for file in "$work/release-files"/*; do
  url="https://github.com/$REPO/releases/download/$TAG/$(basename "$file")"
  curl -fsSL "$url" -o "$work/observed-$(basename "$file")"
  cmp "$file" "$work/observed-$(basename "$file")"
done
gh release view "$TAG" --repo "$REPO" --json assets \
  --jq '.assets[].name' | sort >"$work/final-assets"
diff -u "$expected_assets" "$work/final-assets"
formula_response="$work/formula-content.json"
gh api "repos/$TAP/contents/Formula/shipshape.rb" >"$formula_response"
decode_nonempty_github_content "$formula_response" "$work/observed-formula.rb" || {
  echo "published formula content is missing, malformed, or not base64" >&2
  exit 69
}
cmp "$formula" "$work/observed-formula.rb"
if grep -qE 'x86_64-apple-darwin|OS\.mac\? && Hardware::CPU\.intel\?' \
  "$work/observed-formula.rb"; then
  echo "published formula claims Intel macOS" >&2
  exit 65
fi
observe_crates
echo "v0.11.0 recovery verified: crates.io x2, GitHub Release (three platforms), and Homebrew tap."
echo "Engine run $ENGINE_RUN remains honestly abandoned; do not resume it."

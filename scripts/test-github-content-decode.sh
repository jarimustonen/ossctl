#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=scripts/lib/github-content-decode.sh
source "$repo_root/scripts/lib/github-content-decode.sh"

for command in jq base64 cmp tr fold sed; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 69
  }
done

work=$(mktemp -d "${TMPDIR:-/tmp}/github-content-decode-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

# A formula normally ends in a newline. The decoded observer must preserve that
# byte exactly without adding jq's own output terminator.
printf 'class Shipshape < Formula\nend\n' >"$work/expected"
base64 <"$work/expected" >"$work/encoded"
jq -n --rawfile content "$work/encoded" \
  '{encoding: "base64", content: $content}' >"$work/response.json"
decode_nonempty_github_content "$work/response.json" "$work/observed"
cmp "$work/expected" "$work/observed"

# Exercise the real API shape: wrapped CRLF base64 and decoded bytes with no
# trailing newline. Any observer-added terminator makes the exact cmp fail.
{
  printf 'class Shipshape < Formula\n'
  printf '  desc "A long fixture that forces GitHub-style wrapped base64 decoding"\n'
  printf 'end'
} >"$work/expected-wrapped"
base64 <"$work/expected-wrapped" | tr -d '\r\n' | fold -w 20 \
  | sed 's/$/\r/' >"$work/encoded-wrapped"
jq -n --rawfile content "$work/encoded-wrapped" \
  '{sha: "fixture", size: 0, encoding: "base64", content: $content, _links: {}}' \
  >"$work/response-wrapped.json"
decode_nonempty_github_content "$work/response-wrapped.json" "$work/observed-wrapped"
cmp "$work/expected-wrapped" "$work/observed-wrapped"

# Missing, wrongly encoded, and malformed content must fail without replacing a
# previously observed file with partial or stale decoded bytes.
printf 'sentinel' >"$work/sentinel"
cp "$work/sentinel" "$work/unchanged"
for fixture in missing wrong-encoding malformed; do
  case $fixture in
    missing) printf '{"encoding":"base64"}\n' >"$work/$fixture.json" ;;
    wrong-encoding) printf '{"encoding":"utf-8","content":"YWJj"}\n' >"$work/$fixture.json" ;;
    malformed) printf '%s\n' '{"encoding":"base64","content":"YWJj%%%"}' >"$work/$fixture.json" ;;
  esac
  if decode_nonempty_github_content "$work/$fixture.json" "$work/unchanged" 2>/dev/null; then
    echo "$fixture content unexpectedly decoded" >&2
    exit 1
  fi
  cmp -s "$work/sentinel" "$work/unchanged" || {
    echo "$fixture content replaced the prior output" >&2
    exit 1
  }
done

echo "GitHub Contents base64 decoding preserves exact bytes and fails closed."

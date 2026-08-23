#!/usr/bin/env bash
# Semantic verifier for the one-time v0.11.0 recovery. The caller supplies the
# pinned repository facts; this file deliberately performs no network writes.

v0110_sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

v0110_expected_asset_names() {
  cat <<'EOF'
dist-manifest.json
sha256.sum
shipshape-aarch64-apple-darwin.tar.xz
shipshape-aarch64-apple-darwin.tar.xz.sha256
shipshape-aarch64-unknown-linux-musl.tar.xz
shipshape-aarch64-unknown-linux-musl.tar.xz.sha256
shipshape-installer.sh
shipshape-x86_64-unknown-linux-musl.tar.xz
shipshape-x86_64-unknown-linux-musl.tar.xz.sha256
source.tar.gz
source.tar.gz.sha256
EOF
}

v0110_verify_asset_set() {
  local release_json=$1 expected=$2 actual
  actual=$(mktemp "${TMPDIR:-/tmp}/v0110-assets.XXXXXX") || return
  jq -r '.assets[].name' "$release_json" | sort >"$actual"
  if ! diff -u "$expected" "$actual"; then
    echo "existing Release asset set conflicts" >&2
    rm -f "$actual"
    return 1
  fi
  rm -f "$actual"
}

v0110_verify_checksum_file() {
  local checksum_file=$1 asset=$2 expected_name=$3 expected_sha=$4
  local line actual
  [[ -f $checksum_file && -f $asset ]] || return 1
  line=$(cat "$checksum_file")
  [[ $line == "$expected_sha *$expected_name" || $line == "$expected_sha  $expected_name" ]] || {
    echo "checksum sidecar conflicts: $expected_name" >&2
    return 1
  }
  actual=$(v0110_sha256 "$asset")
  [[ $actual == "$expected_sha" ]] || {
    echo "published asset checksum conflicts: $expected_name" >&2
    return 1
  }
}

v0110_verify_source_archive() {
  local archive=$1 tag_sha=$2 repo_root=$3
  local prefix=shipshape-0.11.0 list entry relative component unsafe=0 expected_tar actual_tar
  local -a components=()
  list=$(mktemp "${TMPDIR:-/tmp}/v0110-tar-list.XXXXXX") || return
  if ! tar -tzf "$archive" >"$list"; then
    rm -f "$list"
    echo "published source archive cannot be listed" >&2
    return 1
  fi
  while IFS= read -r entry; do
    if [[ $entry != "$prefix" && $entry != "$prefix/" && $entry != "$prefix/"* ]]; then
      echo "source archive entry escapes expected prefix: $entry" >&2
      unsafe=1
      break
    fi
    relative=${entry#"$prefix"/}
    if [[ -n $relative ]]; then
      components=()
      IFS='/' read -r -a components <<<"$relative"
      for component in "${components[@]}"; do
        if [[ $component == .. || $component == . ]]; then
          echo "source archive contains unsafe path: $entry" >&2
          unsafe=1
          break 2
        fi
      done
    fi
  done <"$list"
  rm -f "$list"
  [[ $unsafe == 0 ]] || return 1
  # gzip headers vary, but both the published payload and the local recovery are
  # git archives of TAG_SHA. Compare the complete decompressed tar stream so
  # entry types, links, modes, names, and every tracked byte remain tag-derived.
  expected_tar=$(mktemp "${TMPDIR:-/tmp}/v0110-expected-tar.XXXXXX") || return
  actual_tar=$(mktemp "${TMPDIR:-/tmp}/v0110-actual-tar.XXXXXX") || return
  git -C "$repo_root" archive --format=tar --prefix="$prefix/" "$tag_sha" >"$expected_tar" || return 1
  gzip -dc "$archive" >"$actual_tar" || return 1
  cmp "$expected_tar" "$actual_tar" || {
    echo "published source archive contents differ from immutable tag" >&2
    rm -f "$expected_tar" "$actual_tar"
    return 1
  }
  rm -f "$expected_tar" "$actual_tar"
  for entry in dist-workspace.toml Cargo.toml; do
    tar -xOf "$archive" "$prefix/$entry" \
      | cmp - <(git -C "$repo_root" show "$tag_sha:$entry") || {
      echo "published source archive has wrong tagged $entry" >&2
      return 1
    }
  done
}

v0110_verify_installer() {
  local installer=$1 expected=$2 generated_installer=$3 archives
  cmp "$generated_installer" "$installer" || {
    echo "published installer differs from pinned cargo-dist output" >&2
    return 1
  }
  if grep -q 'x86_64-apple-darwin' "$installer"; then
    echo "published installer claims Intel macOS" >&2
    return 1
  fi
  archives=$(mktemp "${TMPDIR:-/tmp}/v0110-installer-archives.XXXXXX") || return
  sed -n 's/^[[:space:]]*_archive="\(shipshape-[^"]*\.tar\.xz\)"$/\1/p' "$installer" \
    | sort -u >"$archives"
  if ! diff -u "$expected" "$archives"; then
    echo "published installer does not map exactly the three supported archives" >&2
    rm -f "$archives"; return 1
  fi
  rm -f "$archives"
}

# cargo-dist records compatibility aliases for musl binaries in the installer
# artifact. The actual prebuilt target contract is represented by executable
# archives and executable assets, which must both be exactly this set.
v0110_verify_manifest() {
  local manifest=$1 generated=$2 expected_release_artifacts=$3 expected_sha_json=$4 actual_source_sha=$5
  local expected_triples='["aarch64-apple-darwin","aarch64-unknown-linux-musl","x86_64-unknown-linux-musl"]'
  for candidate in "$manifest" "$generated"; do
    jq -e '
      (.systems["build:global:"].cargo_version_line | type == "string" and startswith("cargo "))
      and (.artifacts["source.tar.gz"].checksums.sha256
        | type == "string" and test("^[0-9a-f]{64}$"))
      and (.upload_files | type == "array" and length == 4)
      and all(.upload_files[];
        type == "string" and startswith("/") and (split("/")[-1] | length > 0))
      and ([.upload_files[] | split("/")[-1]] | sort == [
        "sha256.sum", "shipshape-installer.sh", "source.tar.gz", "source.tar.gz.sha256"
      ])
    ' "$candidate" >/dev/null || {
      echo "dist manifest allowed-variance fields are missing or malformed" >&2
      return 1
    }
  done
  local manifest_source_sha
  manifest_source_sha=$(jq -er '.artifacts["source.tar.gz"].checksums.sha256' "$manifest") || return 1
  [[ $manifest_source_sha == "$actual_source_sha" || \
      $manifest_source_sha == 5e487160ec16b8da3649a3c32ff9107638579b7bdfbbd300986b61f7955be166 ]] || {
    echo "published dist manifest has an unknown source-gzip checksum" >&2
    return 1
  }
  jq -e \
    --argjson triples "$expected_triples" \
    --slurpfile hashes "$expected_sha_json" \
    --slurpfile release_artifacts "$expected_release_artifacts" '
      .announcement_tag == "v0.11.0"
      and .announcement_title == "0.11.0 - 2026-08-23"
      and (.announcement_is_prerelease == false)
      and ([.releases[].artifacts[]] | sort == ($release_artifacts[0] | sort))
      and ([.artifacts | to_entries[]
        | select(.value.kind == "executable-zip")
        | .value.target_triples[]] | sort == ($triples | sort))
      and ([.assets[]?.target_triples[]?] | unique | sort == ($triples | sort))
      and ([.artifacts | to_entries[]
        | select(.value.kind == "executable-zip")
        | .value.checksums.sha256] | sort == ($hashes[0] | sort))
      and ([.artifacts | to_entries[]
        | select(.value.kind == "executable-zip")
        | .value.checksum] | sort == [
          "shipshape-aarch64-apple-darwin.tar.xz.sha256",
          "shipshape-aarch64-unknown-linux-musl.tar.xz.sha256",
          "shipshape-x86_64-unknown-linux-musl.tar.xz.sha256"
        ])
      and (.artifacts["source.tar.gz"].checksum == "source.tar.gz.sha256")
    ' "$manifest" >/dev/null || {
    echo "published dist manifest topology conflicts" >&2
    return 1
  }
  if grep -q 'x86_64-apple-darwin' "$manifest"; then
    echo "published dist manifest claims Intel macOS" >&2
    return 1
  fi

  # Every manifest checksum except source gzip is byte-stable and checked above.
  # cargo-dist embeds the hash of its own timestamped source gzip; the recovery
  # then derives the public gzip independently from TAG_SHA. Its sidecar and
  # sha256.sum are authoritative and verified against the downloaded bytes.
  # Normalize only that identified gzip field when comparing the rest of the
  # generated schema; global Cargo version and upload temp roots are the other
  # two observed host-dependent fields.
  local normalized_a normalized_b
  normalized_a=$(mktemp "${TMPDIR:-/tmp}/v0110-manifest-a.XXXXXX") || return
  normalized_b=$(mktemp "${TMPDIR:-/tmp}/v0110-manifest-b.XXXXXX") || return
  jq -S '
    .upload_files |= map(split("/")[-1])
    | .systems["build:global:"].cargo_version_line = "<host-cargo>"
    | .artifacts["source.tar.gz"].checksums.sha256 = "<tag-source-gzip>"
  ' "$manifest" >"$normalized_a" || return 1
  jq -S '
    .upload_files |= map(split("/")[-1])
    | .systems["build:global:"].cargo_version_line = "<host-cargo>"
    | .artifacts["source.tar.gz"].checksums.sha256 = "<tag-source-gzip>"
  ' "$generated" >"$normalized_b" || return 1
  if ! cmp "$normalized_a" "$normalized_b"; then
    echo "published dist manifest differs beyond allowed Cargo/temp/gzip variance" >&2
    rm -f "$normalized_a" "$normalized_b"; return 1
  fi
  rm -f "$normalized_a" "$normalized_b"
}

v0110_verify_published_release() {
  local release_json=$1 published=$2 generated_manifest=$3 generated_installer=$4 notes=$5 tag_sha=$6 repo_root=$7
  local expected_names release_artifacts archive_names hashes_file source_sha aggregate_expected
  expected_names=$(mktemp "${TMPDIR:-/tmp}/v0110-expected.XXXXXX") || return
  release_artifacts=$(mktemp "${TMPDIR:-/tmp}/v0110-release-artifacts.XXXXXX") || return
  archive_names=$(mktemp "${TMPDIR:-/tmp}/v0110-archives.XXXXXX") || return
  hashes_file=$(mktemp "${TMPDIR:-/tmp}/v0110-hashes.XXXXXX") || return
  v0110_expected_asset_names | sort >"$expected_names"
  v0110_verify_asset_set "$release_json" "$expected_names" || return
  jq -e --arg tag v0.11.0 --arg title '0.11.0 - 2026-08-23' --rawfile notes "$notes" '
    .tag_name == $tag and .draft == false and .prerelease == false
    and .name == $title and .body == $notes
  ' "$release_json" >/dev/null || { echo "existing Release metadata conflicts" >&2; return 1; }

  printf '%s\n' \
    shipshape-aarch64-apple-darwin.tar.xz \
    shipshape-aarch64-unknown-linux-musl.tar.xz \
    shipshape-x86_64-unknown-linux-musl.tar.xz | sort >"$archive_names"
  local triples=(aarch64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl)
  local hashes=(
    15c35f00196da6da1d5d852b99dd5de51d8f1895aa97207690814b581273c988
    b510af6cefcdf59b0d18dc0176620e19f40d81e0a5fb5a259544eeaeb39057fa
    83313adf0e1bc91cffb78fea7146f3ecc8280a58b57180c5a0d1cbb3989d9d86)
  local i archive
  for i in 0 1 2; do
    archive="shipshape-${triples[$i]}.tar.xz"
    v0110_verify_checksum_file "$published/$archive.sha256" "$published/$archive" "$archive" "${hashes[$i]}" || return
  done
  source_sha=$(v0110_sha256 "$published/source.tar.gz")
  v0110_verify_checksum_file "$published/source.tar.gz.sha256" "$published/source.tar.gz" source.tar.gz "$source_sha" || return
  v0110_verify_source_archive "$published/source.tar.gz" "$tag_sha" "$repo_root" || return

  aggregate_expected=$(mktemp "${TMPDIR:-/tmp}/v0110-sums.XXXXXX") || return
  for i in 0 1 2; do printf '%s *shipshape-%s.tar.xz\n' "${hashes[$i]}" "${triples[$i]}"; done >"$aggregate_expected"
  printf '%s *source.tar.gz\n' "$source_sha" >>"$aggregate_expected"
  cmp "$aggregate_expected" "$published/sha256.sum" || { echo "published aggregate checksum conflicts" >&2; rm -f "$aggregate_expected"; return 1; }
  rm -f "$aggregate_expected"

  v0110_verify_installer "$published/shipshape-installer.sh" "$archive_names" "$generated_installer" || return
  jq -n --args '$ARGS.positional' "${hashes[@]}" >"$hashes_file"
  jq -n --args '$ARGS.positional' \
    source.tar.gz source.tar.gz.sha256 shipshape-installer.sh sha256.sum \
    shipshape-aarch64-apple-darwin.tar.xz shipshape-aarch64-apple-darwin.tar.xz.sha256 \
    shipshape-aarch64-unknown-linux-musl.tar.xz shipshape-aarch64-unknown-linux-musl.tar.xz.sha256 \
    shipshape-x86_64-unknown-linux-musl.tar.xz shipshape-x86_64-unknown-linux-musl.tar.xz.sha256 \
    >"$release_artifacts"
  v0110_verify_manifest "$published/dist-manifest.json" "$generated_manifest" \
    "$release_artifacts" "$hashes_file" "$source_sha" || return

  # GitHub's digest is destination evidence for all eleven downloaded bytes.
  local digest_file digest_count=0
  digest_file=$(mktemp "${TMPDIR:-/tmp}/v0110-digests.XXXXXX") || return
  jq -er '.assets[] | [.name, .digest] | @tsv' "$release_json" >"$digest_file" || return 1
  while IFS=$'\t' read -r name digest; do
    digest_count=$((digest_count + 1))
    [[ $digest == sha256:* ]] || { echo "Release metadata lacks sha256 digest for $name" >&2; return 1; }
    [[ ${digest#sha256:} == "$(v0110_sha256 "$published/$name")" ]] || {
      echo "Release metadata digest conflicts: $name" >&2; return 1
    }
  done <"$digest_file"
  rm -f "$digest_file"
  [[ $digest_count == 11 ]] || { echo "Release metadata digest count conflicts" >&2; return 1; }
}

v0110_verify_local_bundle() {
  local bundle=$1 notes=$2 tag_sha=$3 repo_root=$4 release_json
  release_json=$bundle/local-release.json
  jq -Rn --arg tag v0.11.0 --arg title '0.11.0 - 2026-08-23' --rawfile body "$notes" '
      {tag_name:$tag, name:$title, body:$body, draft:false, prerelease:false,
       assets: [inputs | {name:., digest:""}]}
    ' < <(v0110_expected_asset_names) >"$release_json" || return 1
  # Replace the temporary filename expressions with hashes of the real bundle.
  local name digest
  while IFS= read -r name; do
    digest=$(v0110_sha256 "$bundle/$name") || return 1
    jq --arg name "$name" --arg digest "sha256:$digest" \
      '(.assets[] | select(.name == $name).digest) = $digest' \
      "$release_json" >"$release_json.next" || return 1
    mv "$release_json.next" "$release_json"
  done < <(v0110_expected_asset_names)
  v0110_verify_published_release "$release_json" "$bundle" "$bundle/dist-manifest.json" \
    "$bundle/shipshape-installer.sh" "$notes" "$tag_sha" "$repo_root"
}

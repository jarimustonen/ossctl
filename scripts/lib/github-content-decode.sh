#!/usr/bin/env bash

# Decode a non-empty GitHub Contents API response without passing binary data
# through jq. jq always terminates raw output with a newline, so using @base64d
# there changes content that does not already end in one.
decode_nonempty_github_content() {
  local input=$1
  local output=$2
  local encoded compact decoded compact_value decode_flag

  encoded=$(mktemp "${output}.base64.XXXXXX") || return 1
  compact=$(mktemp "${output}.compact.XXXXXX") || {
    rm -f "$encoded"
    return 1
  }
  decoded=$(mktemp "${output}.decoded.XXXXXX") || {
    rm -f "$encoded" "$compact"
    return 1
  }

  if ! jq -er '
      select(.encoding == "base64")
      | .content
      | select(type == "string" and length > 0)
    ' "$input" >"$encoded" || ! tr -d '\r\n' <"$encoded" >"$compact"; then
    rm -f "$encoded" "$compact" "$decoded"
    return 1
  fi

  compact_value=$(cat "$compact")
  if [[ ! $compact_value =~ ^[A-Za-z0-9+/]+={0,2}$ ]] \
    || (( ${#compact_value} % 4 != 0 )); then
    rm -f "$encoded" "$compact" "$decoded"
    return 1
  fi

  if printf '' | base64 --decode >/dev/null 2>&1; then
    decode_flag=--decode
  elif printf '' | base64 -D >/dev/null 2>&1; then
    decode_flag=-D
  else
    echo "base64 is missing or supports neither --decode nor -D" >&2
    rm -f "$encoded" "$compact" "$decoded"
    return 1
  fi

  if ! base64 "$decode_flag" <"$compact" >"$decoded" || [[ ! -s $decoded ]]; then
    rm -f "$encoded" "$compact" "$decoded"
    return 1
  fi

  rm -f "$encoded" "$compact"
  if ! mv "$decoded" "$output"; then
    rm -f "$decoded"
    return 1
  fi
}

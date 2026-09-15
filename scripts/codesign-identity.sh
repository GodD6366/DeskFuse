#!/usr/bin/env bash
# Shared by development and packaging; pin local signing across rebuilds.
unidesk_select_codesign_identity() {
  local config_file identities requested matches match_count
  config_file="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.codesign-identity"
  requested="${CODESIGN_IDENTITY:-}"
  # Public prereleases may explicitly request ad-hoc signing. Never fall back to it.
  if [[ "$requested" == "-" ]]; then
    return 0
  fi
  if [[ -z "$requested" && -f "$config_file" ]]; then
    requested="$(cat "$config_file")"
    if [[ -z "$requested" ]]; then
      echo "error: empty local signing configuration: $config_file" >&2
      return 1
    fi
  fi
  identities="$(security find-identity -v -p codesigning)" || return 1
  if [[ -n "$requested" ]]; then
    matches="$(printf '%s\n' "$identities" | awk -v requested="$requested" '
      /^[[:space:]]*[0-9]+\)/ {
        hash=$2; name=$0; sub(/^[^"]*"/, "", name); sub(/".*$/, "", name)
        if (hash == toupper(requested) || name == requested) print hash
      }')"
  else
    matches="$(printf '%s\n' "$identities" | awk '
      /^[[:space:]]*[0-9]+\)/ && /"Apple Development: / { print $2 }')"
  fi
  match_count="$(printf '%s\n' "$matches" | awk 'NF { count++ } END { print count+0 }')"
  if [[ "$match_count" != 1 ]]; then
    echo "error: expected one available local signing certificate, found $match_count." >&2
    echo "Set CODESIGN_IDENTITY to a certificate SHA-1 from security find-identity -v -p codesigning." >&2
    echo "Local pin: $config_file. Signing will not fall back to ad-hoc." >&2
    return 1
  fi
  CODESIGN_IDENTITY="$matches"
  # First successful selection becomes the persistent local default. Explicit
  # overrides of an existing pin apply only to that invocation.
  if [[ ! -e "$config_file" ]]; then
    (umask 077; set -o noclobber; printf '%s\n' "$CODESIGN_IDENTITY" > "$config_file") || return 1
  fi
}
unidesk_select_codesign_identity

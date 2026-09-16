#!/usr/bin/env bash
# Signing helpers shared by packaging. Source after scripts/codesign-identity.sh.
# Developer ID builds use a secure timestamp and hardened runtime so notarytool
# can accept the product. Ad-hoc and Apple Development keep the existing flags.

unidesk_resolve_signing_kind() {
  if [[ "${CODESIGN_IDENTITY}" == "-" ]]; then
    SIGNING_KIND=ad-hoc
    return 0
  fi
  local listing name_line
  listing="$(security find-identity -v -p codesigning)" || return 1
  name_line="$(printf '%s\n' "$listing" | awk -v requested="$CODESIGN_IDENTITY" '
    /^[[:space:]]*[0-9]+\)/ {
      hash=$2; name=$0; sub(/^[^"]*"/, "", name); sub(/".*$/, "", name)
      if (hash == toupper(requested) || name == requested) { print name; exit }
    }')"
  case "$name_line" in
    "Developer ID Application:"*) SIGNING_KIND=developer-id ;;
    "Apple Development:"*) SIGNING_KIND=apple-development ;;
    *)
      echo "error: unrecognized codesign identity: ${name_line:-$CODESIGN_IDENTITY}" >&2
      return 1
      ;;
  esac
}

unidesk_codesign() {
  if [[ "$#" -lt 1 ]]; then
    echo "error: unidesk_codesign requires a target" >&2
    return 2
  fi
  # bash 3.2 + set -u treats an empty "${array[@]}" as unbound. Build one
  # argv that always has the codesign prefix so helpers with no extra flags
  # (update helper) do not abort after input-service signs.
  local target i
  local -a cmd
  target="${!#}"
  if [[ "${SIGNING_KIND:-}" == "developer-id" ]]; then
    cmd=(codesign --force --strict --timestamp --options runtime)
  else
    cmd=(codesign --force --strict --timestamp=none)
  fi
  i=1
  while [[ "$i" -lt "$#" ]]; do
    cmd+=("${!i}")
    i=$((i + 1))
  done
  cmd+=(--sign "$CODESIGN_IDENTITY" "$target")
  "${cmd[@]}"
}

unidesk_write_signing_status() {
  local directory="$1"
  local notarized="${2:-false}"
  mkdir -p "$directory"
  printf '%s\n%s\n' "$SIGNING_KIND" "$notarized" > "$directory/.signing"
  if [[ -n "${GITHUB_ENV:-}" ]]; then
    {
      printf 'UNIDESK_SIGNING=%s\n' "$SIGNING_KIND"
      printf 'UNIDESK_NOTARIZED=%s\n' "$notarized"
    } >> "$GITHUB_ENV"
  fi
}

unidesk_notarize_credentials_ready() {
  if [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER:-}" ]]; then
    return 0
  fi
  if [[ -n "${APPLE_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    return 0
  fi
  return 1
}

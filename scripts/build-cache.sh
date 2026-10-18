#!/usr/bin/env bash
# Shared build-cache helpers for the macOS packaging scripts.
# Source this file; it never changes the caller's working directory.
# Bash 3.2 compatible (macOS /bin/bash): no associative arrays, no declare -g.
#
# Scopes are deliberately narrow: only derived build state may be removed.
# Source files, Cargo.lock, committed resources and delivered dist files are
# never candidates here.

deskfuse_cache_target_dir() {
  if [[ -n "${DESKFUSE_CACHE_TARGET_DIR:-}" ]]; then
    printf '%s\n' "$DESKFUSE_CACHE_TARGET_DIR"
    return 0
  fi
  local root="${ROOT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local resolved=""
  resolved="$(cd "$root" && cargo metadata --locked --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys
try:
    print(json.load(sys.stdin)["target_directory"])
except Exception:
    pass' 2>/dev/null)" || true
  if [[ -z "$resolved" ]]; then
    resolved="$root/target"
  fi
  DESKFUSE_CACHE_TARGET_DIR="$resolved"
  printf '%s\n' "$resolved"
}

deskfuse_cache_log_dir() {
  local root="${ROOT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local dir="${DESKFUSE_LOG_DIR:-$root/dist/.build-logs}"
  mkdir -p "$dir"
  printf '%s\n' "$dir"
}

# Record which logical stage is running so the EXIT trap can report it.
deskfuse_stage() {
  DESKFUSE_STAGE_NAME="$1"
  DESKFUSE_STAGE_START="$(date +%s)"
  printf '\n==> %s\n' "$1"
}

deskfuse_stage_done() {
  local now elapsed
  now="$(date +%s)"
  elapsed=$(( now - ${DESKFUSE_STAGE_START:-$now} ))
  printf '==> %s: %ss\n' "${DESKFUSE_STAGE_NAME:-stage}" "$elapsed"
  DESKFUSE_STAGE_SUMMARY="${DESKFUSE_STAGE_SUMMARY:-}${DESKFUSE_STAGE_NAME:-stage} ${elapsed}s"$'\n'
}

# Run a command with stdout/stderr captured to dist/.build-logs/<name>.log while
# still streaming it to the terminal. DESKFUSE_LAST_LOG is set on failure.
deskfuse_run_logged() {
  local name="$1"
  shift
  local log status
  log="$(deskfuse_cache_log_dir)/$name.log"
  printf '\n==> %s\n' "$name"
  set +e
  "$@" 2>&1 | tee "$log"
  status=${PIPESTATUS[0]}
  set -e
  DESKFUSE_LAST_LOG="$log"
  return "$status"
}

# Cache-corruption signatures only. A normal compiler error that names a source
# span must never trigger a multi-minute cache rebuild.
deskfuse_failure_is_cache_suspect() {
  local log="$1"
  local scope="${2:-rust}"
  [[ -s "$log" ]] || return 1
  case "$scope" in
    rust)
      grep -Eqi \
        'internal compiler error|SIGSEGV|SIGBUS|SIGABRT|signal: (6|11)|invalid metadata|can.t find crate|proc.macro.*(not found|panicked)|failed to write|No space left on device|failed to load .*\.rlib|\.rlib: (invalid|corrupt)|corrupt' \
        "$log"
      ;;
    swift)
      grep -Eqi \
        'build database|unable to load build|invalid manifest|malformed build|could not read build description|no such file or directory: .*\.build/' \
        "$log"
      ;;
    freerdp)
      grep -Eqi \
        'CMakeCache|undefined reference|libdeskfuse-rdp\.a|No such file or directory' \
        "$log"
      ;;
    *)
      return 1
      ;;
  esac
}

# Resolve both paths and require child to live under prefix.
deskfuse_assert_under() {
  local path="$1"
  local prefix="$2"
  local resolved prefix_resolved
  resolved="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$path" 2>/dev/null)" || return 1
  prefix_resolved="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$prefix" 2>/dev/null)" || return 1
  case "$resolved/" in
    "$prefix_resolved"/*) return 0 ;;
    *)
      echo "refusing to clean unexpected path: $resolved (expected under $prefix_resolved)" >&2
      return 1
      ;;
  esac
}

deskfuse_remove_path() {
  local path="$1"
  local prefix="$2"
  deskfuse_assert_under "$path" "$prefix" || return 1
  if [[ "${DESKFUSE_DRY_RUN:-0}" == 1 ]]; then
    printf 'would remove: %s\n' "$path"
    return 0
  fi
  if [[ ! -e "$path" && ! -L "$path" ]]; then
    return 0
  fi
  printf 'remove: %s\n' "$path"
  rm -rf -- "$path"
}

# deskfuse_cache_clean <scope> [args]
#   rust-target <triple> [package ...]   cargo clean for one target (all profiles)
#   rust-host-tools                      host-side packaging tools (bindgen/update)
#   swift-build                          apps/deskfuse-macos/.build
#   swift-fingerprint <triple> <config>  fingerprinted Swift receipt + linked app
#   bindings                             uniffi Swift bindings receipt
#   assets                               generated icon/asset receipt
#   freerdp [arch]                       vendored FreeRDP archive + matching build
deskfuse_cache_clean() {
  local scope="$1"
  shift || true
  local root="${ROOT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local target_dir
  target_dir="$(deskfuse_cache_target_dir)"
  local sdk="$root/apps/deskfuse-macos/SDK"
  case "$scope" in
    rust-target)
      local triple="${1:-}"
      shift || true
      [[ -n "$triple" ]] || { echo "rust-target requires a target triple" >&2; return 2; }
      if [[ "${DESKFUSE_DRY_RUN:-0}" == 1 ]]; then
        printf 'would run: cargo clean --target %s%s\n' "$triple" "${*:+ -p $*}"
        return 0
      fi
      if [[ "$#" -eq 0 ]]; then
        ( cd "$root" && cargo clean --target "$triple" )
      else
        local package
        for package in "$@"; do
          ( cd "$root" && cargo clean --target "$triple" -p "$package" )
        done
      fi
      ;;
    rust-host-tools)
      if [[ "${DESKFUSE_DRY_RUN:-0}" == 1 ]]; then
        printf 'would run: cargo clean -p deskfuse-bindgen -p deskfuse-update\n'
        return 0
      fi
      local package
      for package in deskfuse-bindgen deskfuse-update; do
        ( cd "$root" && cargo clean -p "$package" )
      done
      ;;
    swift-build)
      deskfuse_remove_path "$root/apps/deskfuse-macos/.build" "$root/apps/deskfuse-macos"
      deskfuse_remove_path "$root/apps/deskfuse-macos/.build-fast" "$root/apps/deskfuse-macos"
      ;;
    swift-fingerprint)
      local triple="${1:-}"
      local config="${2:-release}"
      [[ -n "$triple" ]] || { echo "swift-fingerprint requires a target triple" >&2; return 2; }
      local variant
      for variant in "" "-fast"; do
        deskfuse_remove_path "$sdk/.swift-fingerprint-$triple-$config$variant" "$sdk"
        deskfuse_remove_path "$sdk/.swift-bin-path-$triple-$config$variant" "$sdk"
      done
      deskfuse_remove_path "$target_dir/native-macos/$config/deskfuse-app" "$target_dir/native-macos"
      deskfuse_remove_path "$target_dir/native-macos/$config/Metadata.appintents" "$target_dir/native-macos"
      ;;
    bindings)
      deskfuse_remove_path "$sdk/.swift-bindings-fingerprint.json" "$sdk"
      ;;
    assets)
      deskfuse_remove_path "$root/resources/generated/.assets-fingerprint" "$root/resources/generated"
      ;;
    freerdp)
      local arch="${1:-${DESKFUSE_MACOS_ARCH:-arm64}}"
      local vendor="$root/apps/deskfuse-macos/RemoteApplication/Vendor/FreeRDP"
      deskfuse_remove_path "$vendor/.fingerprint" "$vendor"
      deskfuse_remove_path "$vendor/lib/libdeskfuse-rdp.a" "$vendor/lib"
      deskfuse_remove_path "$vendor/module.modulemap" "$vendor"
      deskfuse_remove_path "$vendor/RdpBridge.h" "$vendor"
      local cache="${FREERDP_CACHE_DIR:-$HOME/.cache/deskfuse-freerdp}"
      if [[ -d "$cache" ]]; then
        local directory
        for directory in "$cache"/build-*-"$arch"-*; do
          [[ -e "$directory" ]] || continue
          deskfuse_remove_path "$directory" "$cache"
        done
      fi
      ;;
    *)
      echo "unknown cache scope: $scope" >&2
      return 2
      ;;
  esac
}

deskfuse_rust_cache_key() {
  local triple="$1"
  local profile="$2"
  local rustc_id cargo_id host sdk_id
  rustc_id="$(rustc -vV 2>/dev/null | shasum -a 256 | awk '{print $1}')"
  cargo_id="$(cargo -V 2>/dev/null)"
  host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
  sdk_id="$( { xcrun --sdk macosx --show-sdk-path 2>/dev/null; xcrun --sdk macosx --show-sdk-version 2>/dev/null; } \
    | shasum -a 256 | awk '{print $1}')"
  printf '%s\n' \
    "rustc=$rustc_id" \
    "cargo=$cargo_id" \
    "host=$host" \
    "triple=$triple" \
    "profile=$profile" \
    "deployment=${MACOSX_DEPLOYMENT_TARGET:-}" \
    "rustflags=${RUSTFLAGS:-}" \
    "incremental=${CARGO_INCREMENTAL:-}" \
    "sdk=$sdk_id" \
    | shasum -a 256 | awk '{print $1}'
}

# Refuse to reuse artifacts produced by another rustc/SDK/target environment.
# The key is recorded only after a successful build.
deskfuse_rust_cache_guard() {
  local triple="$1"
  local profile="$2"
  local target_dir state key_file key
  target_dir="$(deskfuse_cache_target_dir)"
  state="$target_dir/.deskfuse-cache"
  key_file="$state/rust-key-$triple-$profile"
  key="$(deskfuse_rust_cache_key "$triple" "$profile")"
  mkdir -p "$state"
  if [[ -f "$key_file" && "$(cat "$key_file")" != "$key" ]]; then
    echo "rust cache: toolchain/SDK/environment changed for $triple/$profile" >&2
    echo "rust cache: cleaning stale artifacts instead of reusing them" >&2
    deskfuse_cache_clean rust-target "$triple"
    deskfuse_cache_clean rust-host-tools
    deskfuse_cache_clean swift-fingerprint "$triple" release
    deskfuse_cache_clean swift-fingerprint "$triple" debug
    deskfuse_cache_clean bindings || true
  fi
  DESKFUSE_RUST_CACHE_KEY="$key"
  DESKFUSE_RUST_CACHE_KEY_FILE="$key_file"
}

deskfuse_rust_cache_record() {
  if [[ -z "${DESKFUSE_RUST_CACHE_KEY:-}" || -z "${DESKFUSE_RUST_CACHE_KEY_FILE:-}" ]]; then
    return 0
  fi
  mkdir -p "$(dirname "$DESKFUSE_RUST_CACHE_KEY_FILE")"
  printf '%s\n' "$DESKFUSE_RUST_CACHE_KEY" > "$DESKFUSE_RUST_CACHE_KEY_FILE"
}

deskfuse_cache_note_failure() {
  local stage="$1"
  local log="${2:-}"
  local target_dir state
  target_dir="$(deskfuse_cache_target_dir)"
  state="$target_dir/.deskfuse-cache"
  mkdir -p "$state"
  {
    printf 'stage=%s\n' "$stage"
    printf 'log=%s\n' "$log"
    printf 'when=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  } > "$state/last-failure.txt"
}

# Source-time metadata used by EXIT traps.
DESKFUSE_STAGE_NAME="${DESKFUSE_STAGE_NAME:-preflight}"
DESKFUSE_STAGE_START="${DESKFUSE_STAGE_START:-$(date +%s)}"

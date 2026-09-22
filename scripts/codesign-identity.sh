#!/usr/bin/env bash
# 开发构建与打包共用。本目录指纹跨次构建保持不变，禁止静默退回 ad-hoc。
deskfuse_select_codesign_identity() {
  local config_file identities requested matches match_count common_git_dir main_root main_pin
  config_file="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.codesign-identity"
  requested="${CODESIGN_IDENTITY:-}"
  # 只有调用方显式传入 CODESIGN_IDENTITY=- 才使用 ad-hoc。
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
  # 关联 worktree 没有本目录指纹时，沿用主检出已固定的证书，不在多张证书里猜测。
  if [[ -z "$requested" ]] \
    && common_git_dir="$(git -C "$(dirname "$config_file")" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
    && [[ -n "$common_git_dir" ]] \
    && main_root="$(cd "$(dirname "$common_git_dir")" 2>/dev/null && pwd)" \
    && [[ "$main_root/.codesign-identity" != "$config_file" ]] \
    && [[ -f "$main_root/.codesign-identity" ]]; then
    main_pin="$main_root/.codesign-identity"
    requested="$(cat "$main_pin")"
    if [[ -z "$requested" ]]; then
      echo "error: empty local signing configuration: $main_pin" >&2
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
  # 本目录第一次成功选定后写入指纹。已有指纹时，环境变量只覆盖本次调用。
  if [[ ! -e "$config_file" ]]; then
    (umask 077; set -o noclobber; printf '%s\n' "$CODESIGN_IDENTITY" > "$config_file") || return 1
  fi
}
deskfuse_select_codesign_identity

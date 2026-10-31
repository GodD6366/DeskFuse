#!/usr/bin/env bash
# Shared by local installation and its lifecycle regression tests.

deskfuse_stop_for_install() {
  local binary_name="$1" wait_round
  pkill -x "$binary_name" 2>/dev/null || true
  pkill -x "deskfuse-input-service" 2>/dev/null || true
  # WebView 界面是独立进程：宿主被直接结束后它要么自己检测到管道关闭退出，
  # 要么在这里被收掉，避免安装后留下旧版本的第二个 Dock 图标。
  pkill -x "deskfuse-mygo" 2>/dev/null || true
  for ((wait_round = 0; wait_round < 20; wait_round++)); do
    if ! pgrep -x "$binary_name" >/dev/null 2>&1 \
      && ! pgrep -x "deskfuse-input-service" >/dev/null 2>&1 \
      && ! pgrep -x "deskfuse-mygo" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "error: DeskFuse processes did not exit; keeping the installed app unchanged" >&2
  return 1
}

deskfuse_open_installed_app() {
  local bundle="$1" attempt output status
  for attempt in 1 2 3; do
    if [[ "$attempt" == 1 ]]; then
      # Do not route the initial launch to the old LaunchServices process record.
      set -- -n "$bundle"
    else
      # A -600 reply can arrive after the new process has started. Reuse it on
      # retry rather than creating another instance during its registration.
      set -- "$bundle"
    fi
    if output="$(open "$@" 2>&1)"; then
      if [[ -n "$output" ]]; then printf '%s\n' "$output"; fi
      return 0
    else
      status=$?
    fi
    printf '%s\n' "$output" >&2
    # Retry only procNotFound. Signing, permissions and other launch failures
    # must retain their original error and exit status.
    if [[ "$output" != *"error -600"* && "$output" != *"error: -600"* ]] \
      || [[ "$attempt" == 3 ]]; then
      return "$status"
    fi
    echo "auto install: LaunchServices process registration pending; retrying launch" >&2
    sleep "$attempt"
  done
}

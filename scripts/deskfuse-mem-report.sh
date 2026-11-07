#!/bin/bash
#
# deskfuse-mem-report.sh — 用户电脑上的一次性内存报告（只读，不改系统设置）。
#
# 用法（用户端）：
#   bash ~/Downloads/deskfuse-mem-report.sh
#
# 脚本会在 2 分钟内采样 DeskFuse 的内存，然后把一个文本报告放到桌面并选中它，
# 用户把这个文件发回即可。不安装任何东西，不需要管理员权限，不联网。
#
# 支持选项（一般给支持人员用）：
#   --duration 秒    采样时长，默认 120
#   --interval 秒    采样间隔，默认 3
#   --out 目录       报告目录，默认桌面
#   --pid PID        指定进程，默认查找 deskfuse-app
#
# 报告只含进程内存统计、系统版本和机型，不含剪贴板内容、文件名、窗口标题、
# 设备名称、账号名或任何网络信息。

set -u

DURATION=120
INTERVAL=3
OUT_DIR=""
TARGET_PID=""
JUMP_MB="${JUMP_MB:-40}"
PEAK_CAPTURED=0
TMP=""
REPORT=""

say() { printf '%s\n' "$*"; }

usage() {
  cat <<'EOF'
用法: bash deskfuse-mem-report.sh [--duration 秒] [--interval 秒] [--out 目录] [--pid PID]

默认采样 120 秒、每 3 秒一次，报告写到桌面。
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --duration) DURATION="${2:-}"; shift 2 ;;
    --interval) INTERVAL="${2:-}"; shift 2 ;;
    --out) OUT_DIR="${2:-}"; shift 2 ;;
    --pid) TARGET_PID="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) say "未知参数: $1"; usage; exit 2 ;;
  esac
done

case "$DURATION" in ''|*[!0-9]*) say "错误: --duration 必须是整数秒"; exit 2 ;; esac
case "$INTERVAL" in ''|*[!0-9]*) say "错误: --interval 必须是整数秒"; exit 2 ;; esac
[ "$INTERVAL" -lt 1 ] && INTERVAL=1
[ "$DURATION" -lt "$INTERVAL" ] && DURATION="$INTERVAL"

for tool in footprint vmmap heap ps pgrep awk sysctl sw_vers; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    say "错误: 系统缺少 $tool，无法生成报告。"
    exit 1
  fi
done

find_app_pid() {
  if [ -n "$TARGET_PID" ]; then
    if ps -p "$TARGET_PID" -o command= >/dev/null 2>&1; then
      printf '%s' "$TARGET_PID"
      return 0
    fi
    return 1
  fi
  local pid
  pid="$(pgrep -x deskfuse-app 2>/dev/null | tail -1)"
  [ -n "$pid" ] && printf '%s' "$pid"
}

# ---------------------------------------------------------------- 解析函数

to_mb() { # value unit
  awk -v v="$1" -v u="$2" 'BEGIN {
    x = v + 0
    if (u == "KB") x = x / 1024
    else if (u == "GB") x = x * 1024
    else if (u == "B") x = x / 1048576
    printf "%.1f", x
  }'
}

footprint_total() { # file -> MB
  local raw value unit
  raw="$(awk '/Footprint:/ {
    for (i = 1; i <= NF; i++) if ($i == "Footprint:") { print $(i + 1) " " $(i + 2); exit }
  }' "$1")"
  [ -z "$raw" ] && return 1
  value="${raw%% *}"
  unit="${raw##* }"
  to_mb "$value" "$unit"
}

footprint_peak() { # file -> MB
  local raw value unit
  raw="$(awk '/phys_footprint_peak:/ { print $2 " " $3; exit }' "$1")"
  [ -z "$raw" ] && return 1
  value="${raw%% *}"
  unit="${raw##* }"
  to_mb "$value" "$unit"
}

categories_tsv() { # file -> "Category<TAB>dirty_MB"
  awk '
    function mb(v, u,  x) {
      x = v + 0
      if (u == "KB") x = x / 1024
      else if (u == "GB") x = x * 1024
      else if (u == "B") x = x / 1048576
      return x
    }
    {
      line = $0
      sub(/^[ \t]+/, "", line)
      if (line == "" || line ~ /^---/ || line ~ /^TOTAL/ || line ~ /^Dirty/) next
      n = split(line, f, /[ \t][ \t]+/)
      if (n < 5) next
      if (f[4] !~ /^[0-9]+$/) next
      if (f[5] == "TOTAL") next
      split(f[1], p, " ")
      if (p[2] !~ /^(B|KB|MB|GB)$/) next
      printf "%s\t%.1f\n", f[5], mb(p[1], p[2])
    }' "$1"
}

rss_mb() { # pid
  ps -p "$1" -o rss= 2>/dev/null | awk '{ printf "%.1f", $1 / 1024 }'
}

redact() { # 去掉用户主目录和用户文件路径，保留系统库路径
  sed -e "s|${HOME}|~|g" -e "s|/Users/[^ ]*|<user path>|g"
}

# ---------------------------------------------------------------- 进程与元数据

APP_PID="$(find_app_pid)"
if [ -z "$APP_PID" ]; then
  say "没有找到正在运行的 DeskFuse。"
  say "请先打开 DeskFuse，等主窗口出现后重新运行本脚本。"
  exit 1
fi

if ! ps -p "$APP_PID" -o command= | grep -q deskfuse; then
  say "错误: PID $APP_PID 不是 DeskFuse 进程。"
  exit 1
fi

EXE="$(ps -p "$APP_PID" -o comm=)"
APP_BUNDLE="${EXE%/Contents/MacOS/*}"
APP_VERSION="$(defaults read "$APP_BUNDLE/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo "?")"
APP_BUILD="$(defaults read "$APP_BUNDLE/Contents/Info.plist" CFBundleVersion 2>/dev/null || echo "?")"

if [ -z "$OUT_DIR" ]; then
  if [ -d "$HOME/Desktop" ] && [ -w "$HOME/Desktop" ]; then
    OUT_DIR="$HOME/Desktop"
  else
    OUT_DIR="$HOME"
  fi
fi
mkdir -p "$OUT_DIR" 2>/dev/null || OUT_DIR="$HOME"
REPORT="$OUT_DIR/DeskFuse-mem-$(date '+%Y%m%d-%H%M%S').txt"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-mem-report.XXXXXX")" || exit 1
trap 'rm -rf "$TMP"' EXIT

if [ "$DURATION" -ge 60 ]; then
  say "正在采集 DeskFuse 内存，大约需要 $((DURATION / 60)) 分 $((DURATION % 60)) 秒。"
else
  say "正在采集 DeskFuse 内存，大约需要 ${DURATION} 秒。"
fi
say "采集期间请保持 DeskFuse 运行，不要关闭这个终端窗口。"
say ""

# ---------------------------------------------------------------- 采样

echo "second,role,footprint_mb,peak_mb,rss_mb" > "$TMP/series.csv"

sample_one() { # second pid role -> echoes footprint MB
  local second="$1" pid="$2" role="$3"
  local fp="$TMP/fp-$role.txt"
  if ! footprint -p "$pid" > "$fp" 2>/dev/null; then
    return 1
  fi
  local total peak rss
  total="$(footprint_total "$fp")"
  [ -z "$total" ] && return 1
  peak="$(footprint_peak "$fp" 2>/dev/null || echo "")"
  rss="$(rss_mb "$pid")"
  printf '%s,%s,%s,%s,%s\n' "$second" "$role" "$total" "$peak" "$rss" >> "$TMP/series.csv"
  categories_tsv "$fp" > "$TMP/cats-$role.tsv"
  printf '%s' "$total"
}

capture_state() { # pid dir label
  local pid="$1" dir="$2" label="$3"
  mkdir -p "$dir" || return 0
  echo "$label" > "$dir/label.txt"
  date '+%Y-%m-%d %H:%M:%S' > "$dir/time.txt"
  footprint -p "$pid" > "$dir/footprint.txt" 2>&1
  vmmap -summary "$pid" > "$dir/vmmap-summary.txt" 2>&1
  {
    echo "# 按 dirty+swapped 排名的前 40 个区域"
    vmmap -sortBySize -wide "$pid" 2>/dev/null | awk '
      function mb(v,  n, last) {
        n = v + 0
        last = substr(v, length(v))
        if (last == "K") n = n / 1024
        else if (last == "G") n = n * 1024
        else if (last != "M") n = n / 1048576
        return n
      }
      match($0, /\[[^]]*\]/) {
        b = substr($0, RSTART + 1, RLENGTH - 2)
        n = split(b, a, " ")
        if (n < 4) next
        d = mb(a[3]) + mb(a[4])
        if (d < 1) next
        if ($0 ~ /\(empty\)/) next
        printf "%.1f\t%s\n", d, $0
      }' | sort -k1,1 -nr | head -40 | cut -f2-
  } > "$dir/vmmap-regions.txt"
  {
    echo "# 按总字节排名的堆对象（前 25 行）"
    heap -s -H "$pid" 2>&1 | awk '
      /CLASS_NAME/ { started = 1; print; next }
      started && NF && $0 !~ /^ *=/ { print; if (++n == 25) exit }
    '
  } > "$dir/heap-classes.txt"
}

# ---------------------------------------------------------------- 采样循环

STARTED="$(date +%s)"
LAST_PROGRESS=-1
PREVIOUS=""
INDEX=0
EXITED=0

while :; do
  NOW="$(date +%s)"
  ELAPSED=$((NOW - STARTED))
  [ "$ELAPSED" -ge "$DURATION" ] && break

  if ! ps -p "$APP_PID" >/dev/null 2>&1; then
    EXITED=1
    break
  fi

  INDEX=$((INDEX + 1))
  MAIN="$(sample_one "$ELAPSED" "$APP_PID" main)"
  if [ -z "$MAIN" ]; then
    say "暂时无法读取 DeskFuse 的内存数据，稍后重试…"
  else
    [ -s "$TMP/cats-baseline.tsv" ] || cp "$TMP/cats-main.tsv" "$TMP/cats-baseline.tsv"
    for child in deskfuse-mygo deskfuse-input-service; do
      cp="$(pgrep -x "$child" 2>/dev/null | tail -1)"
      [ -n "$cp" ] && sample_one "$ELAPSED" "$cp" "$child" >/dev/null
    done

    if [ -n "$PREVIOUS" ] && [ "$PEAK_CAPTURED" = 0 ]; then
      if awk -v a="$MAIN" -v b="$PREVIOUS" -v j="$JUMP_MB" 'BEGIN { exit !(a - b >= j) }'; then
        say "检测到内存上涨，已记录当时的现场。"
        capture_state "$APP_PID" "$TMP/peak" \
          "采样期间上涨 $(awk -v a="$MAIN" -v b="$PREVIOUS" 'BEGIN { printf "%+.0f", a - b }') MB"
        PEAK_CAPTURED=1
      fi
    fi
    PREVIOUS="$MAIN"

    if [ "$LAST_PROGRESS" -lt 0 ] || [ $((ELAPSED - LAST_PROGRESS)) -ge 15 ]; then
      say "  [ ${ELAPSED}/${DURATION} 秒 ] 当前内存 ${MAIN} MB"
      LAST_PROGRESS="$ELAPSED"
    fi
  fi

  SLEEP=$((INTERVAL - ($(date +%s) - NOW)))
  [ "$SLEEP" -gt 0 ] && sleep "$SLEEP"
done

if ps -p "$APP_PID" >/dev/null 2>&1; then
  capture_state "$APP_PID" "$TMP/end" "报告生成时"
fi

# ---------------------------------------------------------------- 生成报告

MODEL="$(sysctl -n hw.model 2>/dev/null || echo "?")"
MEM_GB="$(sysctl -n hw.memsize 2>/dev/null | awk '{ printf "%.0f", $1 / 1073741824 }')"
MACOS="$(sw_vers -productVersion 2>/dev/null || echo "?")"
SWAP="$(sysctl -n vm.swapusage 2>/dev/null | sed 's/^vm.swapusage: //' || echo "?")"
PRESSURE="$(memory_pressure 2>/dev/null | tail -1 || true)"
UPTIME="$(ps -p "$APP_PID" -o etime= 2>/dev/null | tr -d ' ')"
CPU="$(ps -p "$APP_PID" -o %cpu= 2>/dev/null | awk '{ print $1 }')"
CURRENT_FOOT="$(footprint_total "$TMP/fp-main.txt" 2>/dev/null || true)"
CURRENT_PEAK="$(footprint_peak "$TMP/fp-main.txt" 2>/dev/null || true)"
[ -z "$CURRENT_FOOT" ] && CURRENT_FOOT="$PREVIOUS"
[ -z "$CURRENT_PEAK" ] && CURRENT_PEAK="?"

SAMPLES="$(awk -F, '$2 == "main" { n++ } END { print n + 0 }' "$TMP/series.csv")"
START_FOOT="$(awk -F, '$2 == "main" { print $3; exit }' "$TMP/series.csv")"
END_FOOT="$(awk -F, '$2 == "main" { v = $3 } END { print v }' "$TMP/series.csv")"
MAX_FOOT="$(awk -F, '$2 == "main" { if ($3 + 0 > m) m = $3 + 0 } END { printf "%.1f", m }' "$TMP/series.csv")"
if [ "$SAMPLES" -eq 0 ]; then
  START_FOOT="?"
  END_FOOT="?"
  MAX_FOOT="?"
fi

CAPTURE_DIR="$TMP/end"
[ "$PEAK_CAPTURED" = 1 ] && [ -d "$TMP/peak" ] && CAPTURE_DIR="$TMP/peak"

{
  echo "DeskFuse 内存报告"
  echo "生成时间: $(date '+%Y-%m-%d %H:%M:%S %z')"
  echo "开始时间: $(date -r "$STARTED" '+%Y-%m-%d %H:%M:%S %z')"
  echo "采集时长: ${DURATION} 秒（每 ${INTERVAL} 秒一次，共 ${SAMPLES} 次主进程采样）"
  echo
  echo "== 系统 =="
  echo "macOS: ${MACOS}"
  echo "机型: ${MODEL}"
  echo "内存: ${MEM_GB} GB"
  echo "交换空间: ${SWAP}"
  echo "内存压力: ${PRESSURE:-未读取}"
  echo
  echo "== DeskFuse =="
  echo "路径: $(printf '%s' "$APP_BUNDLE" | redact)"
  echo "版本: ${APP_VERSION} (${APP_BUILD})"
  echo "PID: ${APP_PID}"
  echo "运行时长: ${UPTIME:-已退出}"
  echo "CPU: ${CPU:-?}%"
  echo "footprint: ${CURRENT_FOOT:-?} MB"
  echo "历史峰值: ${CURRENT_PEAK:-?} MB"
  echo "RSS: $(rss_mb "$APP_PID") MB"
  echo
  echo "== footprint 分类（当前）=="
  if [ -s "$TMP/cats-main.tsv" ]; then
    sort -t "$(printf '\t')" -k2 -nr "$TMP/cats-main.tsv" \
      | awk -F'\t' '{ printf "  %8.1f MB  %s\n", $2, $1 }'
  else
    echo "  无数据"
  fi
  echo
  echo "== 采样结果 =="
  echo "footprint: 开始 ${START_FOOT:-?} MB，结束 ${END_FOOT:-?} MB，最高 ${MAX_FOOT:-?} MB"
  if [ "$EXITED" = 1 ]; then
    echo "注意: 采样期间 DeskFuse 退出，以上为退出前的数据。"
  fi
  echo
  echo "分类变化（开始 → 结束）:"
  if [ -s "$TMP/cats-baseline.tsv" ] && [ -s "$TMP/cats-main.tsv" ]; then
    CHANGES="$(awk -F'\t' '
      NR == FNR { before[$1] = $2; next }
      { after[$1] = $2 }
      END {
        for (name in before) if (!(name in after)) after[name] = 0
        for (name in after) {
          b = (name in before) ? before[name] : 0
          a = after[name]
          d = a - b
          if (d >= 0.5 || d <= -0.5) printf "%s\t%.1f\t%.1f\t%+.1f\n", name, b, a, d
        }
      }' "$TMP/cats-baseline.tsv" "$TMP/cats-main.tsv" \
      | sort -t "$(printf '\t')" -k4 -nr | head -12 \
      | awk -F'\t' '{ printf "  %-34s %7.1f → %7.1f  (%+.1f)\n", $1, $2, $3, $4 }')"
    if [ -n "$CHANGES" ]; then
      printf '%s\n' "$CHANGES"
    else
      echo "  采集期间没有分类变化超过 0.5 MB"
    fi
  else
    echo "  无数据"
  fi
  echo
  echo "子进程:"
  awk -F, '$2 != "main" && $2 != "role" {
      name = $2
      last[name] = $3
      if ($4 + 0 > peak[name]) peak[name] = $4 + 0
    }
    END {
      if (length(last) == 0) print "  无数据"
      for (name in last) printf "  %-28s 当前 %6.1f MB  峰值 %6.1f MB\n", name, last[name], peak[name]
    }' "$TMP/series.csv"
  echo
  echo "采样明细（秒,角色,footprint,peak,rss）:"
  sed 's/^/  /' "$TMP/series.csv"
  echo
  echo "== 最大内存区域（dirty+swapped）=="
  echo "采集时间: $(cat "$CAPTURE_DIR/time.txt" 2>/dev/null)"
  echo "采集时机: $(cat "$CAPTURE_DIR/label.txt" 2>/dev/null)"
  sed -n '1,35p' "$CAPTURE_DIR/vmmap-regions.txt" 2>/dev/null | redact || echo "  无法读取"
  echo
  echo "== 最大堆对象 =="
  sed -n '1,26p' "$CAPTURE_DIR/heap-classes.txt" 2>/dev/null | redact || echo "  无法读取"
  echo
  echo "== 说明 =="
  echo "本报告只包含 DeskFuse 进程的内存统计、系统版本与机型。"
  echo "不包含剪贴板内容、文件名、窗口标题、设备名称、账号名或网络信息。"
  echo "生成脚本: deskfuse-mem-report.sh"
} > "$REPORT"

say ""
say "完成。报告已保存到:"
say "  $REPORT"
say ""
say "请把这个文件发给支持人员。"
open -R "$REPORT" >/dev/null 2>&1 || true
exit 0

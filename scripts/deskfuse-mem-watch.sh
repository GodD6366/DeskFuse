#!/usr/bin/env bash
#
# deskfuse-mem-watch.sh — single-file memory watcher for DeskFuse on macOS.
#
# Uses only macOS built-ins (footprint, vmmap, heap, ps, pgrep, awk), so this
# file can be copied to any Mac and run there without the repository.
#
#   ./deskfuse-mem-watch.sh --once --heap
#   ./deskfuse-mem-watch.sh --duration 7200
#   ./deskfuse-mem-watch.sh --interval 1 --jump-mb 100 --no-children
#   ./deskfuse-mem-watch.sh --role deskfuse-mygo --duration 3600
#
# To keep a test Mac awake for an overnight run:
#   caffeinate -is ./deskfuse-mem-watch.sh --duration 28800
#
# Every sample writes one CSV row (footprint, peak, RSS, per-category dirty
# memory) for the app and its helper processes. When the watched role crosses a
# threshold, grows over baseline, or jumps between samples, the script dumps
# `vmmap` and `heap` for that moment under snapshots/.
#
# Reading the result:
#   summary.txt                     start/end/peak and region growth
#   snapshots/*/vmmap-regions.txt   biggest regions and their type (IOSurface,
#                                   untagged VM, ImageIO, ...)
#   snapshots/*/heap-classes.txt    biggest malloc classes (NSConcreteData,
#                                   NSMutableArray, non-object, ...)
#
# Allocation call stacks need a relaunch with backtraces enabled:
#   launchctl setenv MallocStackLogging 1
#   open -a /Applications/DeskFuse.app
#   ./deskfuse-mem-watch.sh --duration 3600 --threshold-mb 250
#   launchctl unsetenv MallocStackLogging
# With stack logging on, `vmmap -stacks <pid>` in a snapshot carries stacks.

set -u

APP_NAME="deskfuse-app"
CHILD_ROLES="deskfuse-mygo deskfuse-input-service"

APP_PID=""
INTERVAL=15
DURATION=0
OUT=""
THRESHOLD_MB=300
GROWTH_MB=60
JUMP_MB=25
COOLDOWN=180
MAX_SNAPSHOTS=8
ROLE="main"
ONCE=0
WITH_HEAP=0
INCLUDE_CHILDREN=1

usage() {
  cat <<'EOF'
Usage: deskfuse-mem-watch.sh [options]

  --pid PID                 watch this process; default: newest deskfuse-app
  --interval SECONDS        sampling interval, whole seconds (default 15)
  --duration SECONDS        stop after this long; 0 = until Ctrl-C (default 0)
  --out DIR                 output directory; default ./deskfuse-memory-<time>
  --threshold-mb MB         snapshot when the watched role reaches this (default 300)
  --growth-mb MB            snapshot after this growth over the first sample (default 60)
  --jump-mb MB              snapshot on a single-sample jump (default 25)
  --snapshot-cooldown SEC   minimum seconds between snapshots (default 180)
  --max-snapshots N         stop snapshotting after N (default 8)
  --role NAME               role that triggers snapshots: main, deskfuse-mygo,
                            deskfuse-input-service (default main)
  --no-children             skip helper processes
  --once                    print the current state and exit
  --heap                    with --once: print the largest heap classes
  -h, --help                this text

Common runs:
  ./deskfuse-mem-watch.sh --once --heap
  ./deskfuse-mem-watch.sh --duration 7200 --threshold-mb 250
  ./deskfuse-mem-watch.sh --role deskfuse-mygo --duration 3600

Run it as the same user that owns the DeskFuse process; otherwise footprint,
vmmap and heap cannot inspect it.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --pid) APP_PID="${2:-}"; shift 2 ;;
    --interval) INTERVAL="${2:-}"; shift 2 ;;
    --duration) DURATION="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --threshold-mb) THRESHOLD_MB="${2:-}"; shift 2 ;;
    --growth-mb) GROWTH_MB="${2:-}"; shift 2 ;;
    --jump-mb) JUMP_MB="${2:-}"; shift 2 ;;
    --snapshot-cooldown) COOLDOWN="${2:-}"; shift 2 ;;
    --max-snapshots) MAX_SNAPSHOTS="${2:-}"; shift 2 ;;
    --role) ROLE="${2:-}"; shift 2 ;;
    --no-children) INCLUDE_CHILDREN=0; shift ;;
    --once) ONCE=1; shift ;;
    --heap) WITH_HEAP=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
  esac
done

for tool in /usr/bin/footprint /usr/bin/vmmap /usr/bin/heap /bin/ps /usr/bin/pgrep /usr/bin/awk; do
  if [ ! -x "$tool" ] && ! command -v "${tool##*/}" >/dev/null 2>&1; then
    echo "error: ${tool##*/} is required" >&2
    exit 1
  fi
done

case "$INTERVAL" in ''|*[!0-9]*) echo "error: --interval must be whole seconds" >&2; exit 2 ;; esac
case "$DURATION" in ''|*[!0-9]*) echo "error: --duration must be whole seconds" >&2; exit 2 ;; esac
case "$MAX_SNAPSHOTS" in ''|*[!0-9]*) echo "error: --max-snapshots must be a whole number" >&2; exit 2 ;; esac
if [ "$INTERVAL" -lt 1 ]; then
  echo "warning: interval raised to 1 second" >&2
  INTERVAL=1
fi

# --------------------------------------------------------------- process lookup

find_app_pid() {
  if [ -n "$APP_PID" ]; then
    if ps -p "$APP_PID" -o command= >/dev/null 2>&1; then
      printf '%s' "$APP_PID"
      return 0
    fi
    return 1
  fi
  local pid
  pid="$(pgrep -x "$APP_NAME" 2>/dev/null | tail -1)"
  [ -n "$pid" ] && printf '%s' "$pid"
}

pid_for_role() {
  if [ "$1" = "main" ]; then
    find_app_pid
  else
    pgrep -x "$1" 2>/dev/null | tail -1
  fi
}

# ------------------------------------------------------------- footprint parsing

to_mb() { # value unit
  awk -v v="$1" -v u="$2" 'BEGIN {
    x = v + 0
    if (u == "KB") x = x / 1024
    else if (u == "GB") x = x * 1024
    else if (u == "B") x = x / 1048576
    printf "%.1f", x
  }'
}

header_footprint() { # file -> "199 MB"
  awk '/Footprint:/ {
    for (i = 1; i <= NF; i++) if ($i == "Footprint:") { print $(i + 1) " " $(i + 2); exit }
  }' "$1"
}

header_peak() { # file -> "1.2 GB"
  awk '/phys_footprint_peak:/ { print $2 " " $3; exit }' "$1"
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

top_categories() { # role -> "Malloc Large 67 | Malloc Small 61 | ..."
  sort -t "$(printf '\t')" -k2 -nr "$TMP/cats-$1.tsv" 2>/dev/null \
    | head -3 \
    | awk -F'\t' '{ printf "%s%s %.0f", (NR > 1 ? " | " : ""), $1, $2 }'
}

# --------------------------------------------------------------- snapshot state

MAX_PEAK=""
BASELINE=""
PREVIOUS=""
LAST_SNAPSHOT_AT=0
SNAPSHOT_COUNT=0
STARTED_AT="$(date +%s)"
DONE=0
CSV=""
SUMMARY=""
SNAPSHOTS=""
TMP=""

finish() {
  [ "$DONE" = 1 ] && return
  DONE=1
  [ -z "$CSV" ] && return
  local end_footprint="" end_peak="" role_pid base
  role_pid="$(pid_for_role "$ROLE")"
  if [ -n "$role_pid" ]; then
    footprint -p "$role_pid" > "$TMP/final.txt" 2>/dev/null || true
    end_footprint="$(header_footprint "$TMP/final.txt")"
    end_peak="$(header_peak "$TMP/final.txt")"
    categories_tsv "$TMP/final.txt" > "$TMP/cats-final.tsv"
    [ -n "$end_footprint" ] && end_footprint="$(to_mb "${end_footprint%% *}" "${end_footprint##* }")"
    [ -n "$end_peak" ] && end_peak="$(to_mb "${end_peak%% *}" "${end_peak##* }")"
  fi
  {
    echo "sample window: $(date -r "$STARTED_AT" '+%Y-%m-%d %H:%M:%S') → $(date '+%H:%M:%S')"
    echo "csv: $CSV"
    echo "watched role: $ROLE"
    printf 'footprint: start %s MB → end %s MB' "${BASELINE:-?}" "${end_footprint:-?}"
    if [ -n "$BASELINE" ] && [ -n "$end_footprint" ]; then
      awk -v a="$BASELINE" -v b="$end_footprint" 'BEGIN { printf " (%+.1f)", b - a }'
    fi
    echo
    echo "peak footprint: ${MAX_PEAK:-${end_peak:-?}} MB"
    echo "snapshots: $SNAPSHOT_COUNT in $SNAPSHOTS"
    base="$TMP/cats-baseline.tsv"
    if [ -s "$base" ] && [ -s "$TMP/cats-final.tsv" ]; then
      echo
      echo "region growth (start → end, MB):"
      growth="$(awk -F'\t' '
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
        }' "$base" "$TMP/cats-final.tsv" \
        | sort -t "$(printf '\t')" -k4 -nr | head -10 \
        | awk -F'\t' '{ printf "  %-34s %7.1f → %7.1f  (%+.1f)\n", $1, $2, $3, $4 }')"
      if [ -n "$growth" ]; then
        printf '%s\n' "$growth"
      else
        echo "  no region changed by 0.5 MB"
      fi
    fi
  } > "$SUMMARY"
  echo
  cat "$SUMMARY"
}

on_signal() {
  finish
  exit 0
}
trap on_signal INT TERM HUP

take_snapshot() { # pid footprint reason
  local pid="$1" footprint="$2" reason="$3" dir
  dir="$SNAPSHOTS/$(date '+%Y%m%d-%H%M%S')-$(printf '%.0f' "$footprint")mb"
  mkdir -p "$dir" || return 0
  {
    echo "captured: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "pid: $pid"
    echo "role: $ROLE"
    echo "reason: $reason"
    echo "footprint: $footprint MB"
  } > "$dir/meta.txt"
  footprint -p "$pid" > "$dir/footprint.txt" 2>&1
  vmmap -summary "$pid" > "$dir/vmmap-summary.txt" 2>&1
  {
    echo "# biggest regions by dirty + swapped (MB), empty malloc space removed"
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
  heap -s -H "$pid" > "$dir/heap-classes.txt" 2>&1
  ps -M -p "$pid" > "$dir/ps.txt" 2>&1
  { vm_stat; sysctl vm.swapusage 2>/dev/null; } > "$dir/vm-stat.txt" 2>&1
  echo "          snapshot: $dir  ($reason)"
  awk '
    /CLASS_NAME/ { started = 1; next }
    started && NF && $0 !~ /^ *=/ { print "          heap: " $0; if (++n == 4) exit }
  ' "$dir/heap-classes.txt" 2>/dev/null || true
}

# ------------------------------------------------------------------- once mode

if [ "$ONCE" = 1 ]; then
  pid="$(pid_for_role "$ROLE")"
  if [ -z "$pid" ]; then
    echo "error: no process for role '$ROLE' (is DeskFuse running as this user?)" >&2
    exit 1
  fi
  work="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-mem.XXXXXX")"
  trap 'rm -rf "$work"' EXIT
  if ! footprint -p "$pid" > "$work/fp.txt" 2>/dev/null; then
    echo "error: footprint could not inspect pid $pid" >&2
    exit 1
  fi
  total="$(header_footprint "$work/fp.txt")"
  total="$(to_mb "${total%% *}" "${total##* }")"
  peak="$(header_peak "$work/fp.txt")"
  [ -n "$peak" ] && peak="$(to_mb "${peak%% *}" "${peak##* }")"
  rss="$(ps -p "$pid" -o rss= | awk '{ printf "%.1f", $1 / 1024 }')"
  cpu="$(ps -p "$pid" -o %cpu= | awk '{ print $1 }')"
  name="$ROLE"
  [ "$ROLE" = "main" ] && name="$APP_NAME"
  echo "$name pid=$pid footprint=$total MB peak=$peak MB rss=$rss MB cpu=$cpu%"
  categories_tsv "$work/fp.txt" | sort -t "$(printf '\t')" -k2 -nr \
    | awk -F'\t' '{ printf "  %8.1f MB  %s\n", $2, $1 }'
  if [ "$WITH_HEAP" = 1 ]; then
    echo
    echo "largest heap classes:"
    heap -s -H "$pid" 2>/dev/null | awk '
      /CLASS_NAME/ { started = 1; next }
      started && NF && $0 !~ /^ *=/ { print "  " $0; if (++n == 8) exit }
    '
  fi
  exit 0
fi

# ------------------------------------------------------------------ loop setup

APP_PID="$(find_app_pid)"
if [ -z "$APP_PID" ]; then
  echo "error: DeskFuse is not running as this user (or pass --pid)" >&2
  exit 1
fi
if ! ps -p "$APP_PID" -o command= | grep -q deskfuse; then
  echo "error: pid $APP_PID is not a DeskFuse process" >&2
  exit 1
fi

case " $CHILD_ROLES " in
  *" $ROLE "*) ;;
  *) if [ "$ROLE" != "main" ]; then
       echo "warning: unknown role '$ROLE'; known: main $CHILD_ROLES" >&2
     fi ;;
esac

if [ -z "$OUT" ]; then
  OUT="$(pwd)/deskfuse-memory-$(date '+%Y%m%d-%H%M%S')"
fi
mkdir -p "$OUT" || exit 1
OUT="$(cd "$OUT" && pwd)"
SNAPSHOTS="$OUT/snapshots"
mkdir -p "$SNAPSHOTS"
CSV="$OUT/samples.csv"
SUMMARY="$OUT/summary.txt"
TMP="$OUT/.state"
mkdir -p "$TMP"

echo "time,iso,role,pid,footprint_mb,peak_mb,rss_mb,malloc_large,malloc_small,malloc_tiny,untagged,os_alloc_once,cg_image,image_io,iosurface,coreanimation,coreservices,stack,top_categories" > "$CSV"

echo "pid $APP_PID  role $ROLE  interval ${INTERVAL}s  out $OUT"
echo "time      footprint   Δbase    peak     top regions"

if [ -z "$(pid_for_role "$ROLE")" ]; then
  echo "error: role '$ROLE' is not running" >&2
  exit 1
fi
if ! footprint -p "$(pid_for_role "$ROLE")" >/dev/null 2>&1; then
  echo "error: footprint cannot inspect role '$ROLE'; run as the user that owns DeskFuse" >&2
  exit 1
fi

while :; do
  now="$(date +%s)"
  if [ "$DURATION" != 0 ] && [ $((now - STARTED_AT)) -ge "$DURATION" ]; then
    break
  fi

  current_app="$(find_app_pid || true)"
  if [ -z "$current_app" ]; then
    echo "process exited; writing summary"
    break
  fi

  pids="main:$current_app"
  if [ "$INCLUDE_CHILDREN" = 1 ]; then
    for child in $CHILD_ROLES; do
      cp="$(pgrep -x "$child" 2>/dev/null | tail -1)"
      [ -n "$cp" ] && pids="$pids $child:$cp"
    done
  fi

  primary_footprint=""
  primary_pid=""
  for entry in $pids; do
    role="${entry%%:*}"
    pid="${entry#*:}"
    fp="$TMP/footprint-$role.txt"
    if ! footprint -p "$pid" > "$fp" 2>/dev/null; then
      continue
    fi
    foot_raw="$(header_footprint "$fp")"
    [ -z "$foot_raw" ] && continue
    foot="$(to_mb "${foot_raw%% *}" "${foot_raw##* }")"
    peak_raw="$(header_peak "$fp")"
    if [ -n "$peak_raw" ]; then
      peak="$(to_mb "${peak_raw%% *}" "${peak_raw##* }")"
    else
      peak=""
    fi
    rss="$(ps -p "$pid" -o rss= 2>/dev/null | awk '{ printf "%.1f", $1 / 1024 }')"
    categories_tsv "$fp" > "$TMP/cats-$role.tsv"
    top="$(top_categories "$role")"
    iso="$(date '+%Y-%m-%dT%H:%M:%S%z')"
    row="$(awk -F'\t' -v time="$now" -v iso="$iso" -v role="$role" -v pid="$pid" \
        -v foot="$foot" -v peak="$peak" -v rss="$rss" -v top="$top" '
      { v[$1] = $2 }
      END {
        printf "%s,%s,%s,%s,%s,%s,%s,", time, iso, role, pid, foot, peak, rss
        printf "%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,\"%s\"\n", \
          v["Malloc Large"], v["Malloc Small"], v["Malloc Tiny"], v["Untagged"], \
          v["OS Alloc Once"], v["CG Image"], v["Image IO"], v["IOSurface"], \
          v["CoreAnimation"], v["CoreServices"], v["Stack"], top
      }' "$TMP/cats-$role.tsv")"
    printf '%s\n' "$row" >> "$CSV"

    if [ "$role" = "$ROLE" ]; then
      primary_footprint="$foot"
      primary_pid="$pid"
      if [ -n "$peak" ]; then
        if [ -z "$MAX_PEAK" ] || awk -v a="$peak" -v b="$MAX_PEAK" 'BEGIN { exit !(a > b) }'; then
          MAX_PEAK="$peak"
        fi
      fi
    fi
  done

  if [ -n "$primary_footprint" ] && [ -n "$primary_pid" ]; then
    if [ -z "$BASELINE" ]; then
      BASELINE="$primary_footprint"
      cp "$TMP/cats-$ROLE.tsv" "$TMP/cats-baseline.tsv"
    fi
    delta="$(awk -v a="$primary_footprint" -v b="$BASELINE" 'BEGIN { printf "%+.1f", a - b }')"
    printf '%s  %7s MB %8s  %7s MB  %s\n' \
      "$(date '+%H:%M:%S')" "$primary_footprint" "$delta" "${MAX_PEAK:-0}" "$(top_categories "$ROLE")"

    reasons=""
    if awk -v a="$primary_footprint" -v t="$THRESHOLD_MB" 'BEGIN { exit !(a >= t) }'; then
      reasons="threshold ${primary_footprint} MB"
    fi
    if awk -v a="$primary_footprint" -v b="$BASELINE" -v g="$GROWTH_MB" 'BEGIN { exit !(a - b >= g) }'; then
      [ -n "$reasons" ] && reasons="$reasons; "
      reasons="${reasons}grew ${delta} MB over ${BASELINE} MB"
    fi
    if [ -n "$PREVIOUS" ] && awk -v a="$primary_footprint" -v p="$PREVIOUS" -v j="$JUMP_MB" 'BEGIN { exit !(a - p >= j) }'; then
      [ -n "$reasons" ] && reasons="$reasons; "
      reasons="${reasons}jumped in ${INTERVAL}s"
    fi

    now_s="$(date +%s)"
    if [ -n "$reasons" ] && [ "$SNAPSHOT_COUNT" -lt "$MAX_SNAPSHOTS" ] \
       && [ $((now_s - LAST_SNAPSHOT_AT)) -ge "$COOLDOWN" ]; then
      take_snapshot "$primary_pid" "$primary_footprint" "$reasons"
      LAST_SNAPSHOT_AT="$now_s"
      SNAPSHOT_COUNT=$((SNAPSHOT_COUNT + 1))
    fi
    PREVIOUS="$primary_footprint"
  else
    echo "$(date '+%H:%M:%S')  role '$ROLE' not found in this sample"
  fi

  elapsed=$(( $(date +%s) - now ))
  sleep_for=$((INTERVAL - elapsed))
  [ "$sleep_for" -gt 0 ] && sleep "$sleep_for"
done

finish

<script setup lang="ts">
import { computed } from 'vue'

interface LatencyPoint {
  second: number
  rttMs: number
  /** 相邻采样间隔超过 2 秒会换段，避免把空档连成斜线。 */
  series: number
}

interface HitchTick {
  second: number
  /** true 表示卡顿来自本机采集排队，false 表示网络路径。 */
  capture: boolean
}

const props = withDefaults(
  defineProps<{
    points?: LatencyPoint[]
    hitches?: HitchTick[]
    width?: number
    height?: number
    color?: string
    /** 迷你模式：无坐标轴、无面积、无标注，只画趋势。 */
    mini?: boolean
    ceiling?: number
    calloutSecond?: number | null
  }>(),
  {
    points: () => [],
    hitches: () => [],
    width: 72,
    height: 22,
    color: 'var(--accent)',
    mini: false,
    ceiling: 100,
    calloutSecond: null,
  },
)

const gradientId = `latency-fill-${Math.random().toString(36).slice(2)}`

const margin = computed(() =>
  props.mini
    ? { top: 0, right: 0, bottom: 0, left: 0 }
    : { top: 16, right: 8, bottom: 20, left: 44 },
)

const plot = computed(() => ({
  left: margin.value.left,
  top: margin.value.top,
  width: Math.max(1, props.width - margin.value.left - margin.value.right),
  height: Math.max(1, props.height - margin.value.top - margin.value.bottom),
}))

const xMin = computed(() => (props.mini ? 0 : -1))
const xMax = computed(() => (props.mini ? 60 : 61))
const yMax = computed(() => props.ceiling * (props.mini ? 1.15 : 1.28))

function x(second: number): number {
  const ratio = (second - xMin.value) / (xMax.value - xMin.value)
  return plot.value.left + ratio * plot.value.width
}

function y(rtt: number): number {
  const clamped = Math.min(Math.max(rtt, 0), props.ceiling)
  return plot.value.top + plot.value.height - (clamped / yMax.value) * plot.value.height
}

const runs = computed<LatencyPoint[][]>(() => {
  const groups: LatencyPoint[][] = []
  for (const point of props.points) {
    const last = groups[groups.length - 1]
    if (last && last[0].series === point.series) last.push(point)
    else groups.push([point])
  }
  return groups
})

const runCounts = computed(() => {
  const counts = new Map<number, number>()
  for (const point of props.points) counts.set(point.series, (counts.get(point.series) ?? 0) + 1)
  return counts
})

function linePath(run: LatencyPoint[]): string {
  return run
    .map((point, index) => `${index === 0 ? 'M' : 'L'}${x(point.second).toFixed(2)} ${y(point.rttMs).toFixed(2)}`)
    .join(' ')
}

function areaPath(run: LatencyPoint[]): string {
  const baseline = y(0).toFixed(2)
  const top = run
    .map((point) => `L${x(point.second).toFixed(2)} ${y(point.rttMs).toFixed(2)}`)
    .join(' ')
  const first = x(run[0].second).toFixed(2)
  const last = x(run[run.length - 1].second).toFixed(2)
  return `M${first} ${baseline} ${top} L${last} ${baseline} Z`
}

const yTicks = computed(() => (props.mini ? [] : [0, props.ceiling / 2, props.ceiling]))
const xTicks = computed(() =>
  props.mini
    ? []
    : [
        { second: 0, label: '60秒前' },
        { second: 30, label: '30秒前' },
        { second: 60, label: '现在' },
      ],
)

function axisLabel(value: number): string {
  const body = value.toFixed(0)
  return Math.abs(value - props.ceiling) < 0.05 ? `${body} ms` : body
}

function timeAnchor(second: number): 'start' | 'middle' | 'end' {
  if (second <= 0) return 'start'
  if (second >= 60) return 'end'
  return 'middle'
}

const callout = computed<LatencyPoint | null>(() => {
  if (props.calloutSecond === null) return null
  return props.points.find((point) => point.second === props.calloutSecond) ?? null
})

function isCallout(point: LatencyPoint): boolean {
  const target = callout.value
  return target !== null && target.second === point.second && target.rttMs === point.rttMs
}

function showPoint(point: LatencyPoint): boolean {
  if (props.points.length <= 8) return true
  if ((runCounts.value.get(point.series) ?? 0) === 1) return true
  return props.points[props.points.length - 1] === point
}

const calloutBox = computed(() => {
  const point = callout.value
  if (!point) return null
  const px = x(point.second)
  const py = y(point.rttMs)
  // 与原生 latencyCalloutPosition 一致：贴近上限放不下时放到点下方，靠两侧边缘时向内收。
  if (point.rttMs > props.ceiling * 0.72) return { x: px, y: py + 14, anchor: 'middle' as const }
  if (point.second > 48) return { x: px - 4, y: py - 8, anchor: 'end' as const }
  if (point.second < 8) return { x: px + 4, y: py - 8, anchor: 'start' as const }
  return { x: px, y: py - 8, anchor: 'middle' as const }
})

function formatMs(value: number): string {
  return value.toFixed(1) + ' ms'
}
</script>

<template>
  <svg
    class="latency-chart"
    :class="{ mini }"
    :width="width"
    :height="height"
    :viewBox="`0 0 ${width} ${height}`"
    :style="{ color }"
    role="img"
    aria-label="最近 60 秒往返延迟，3 秒平滑趋势"
  >
    <defs>
      <linearGradient :id="gradientId" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="currentColor" stop-opacity="0.30" />
        <stop offset="100%" stop-color="currentColor" stop-opacity="0.03" />
      </linearGradient>
    </defs>

    <g v-if="!mini" class="axis">
      <line
        v-for="tick in yTicks"
        :key="`grid-${tick}`"
        class="grid"
        :x1="plot.left"
        :x2="plot.left + plot.width"
        :y1="y(tick)"
        :y2="y(tick)"
      />
      <text
        v-for="tick in yTicks"
        :key="`y-${tick}`"
        class="tick-label"
        :x="plot.left - 6"
        :y="y(tick) + 3"
        text-anchor="end"
      >
        {{ axisLabel(tick) }}
      </text>
      <text
        v-for="tick in xTicks"
        :key="`x-${tick.second}`"
        class="tick-label"
        :x="x(tick.second)"
        :y="height - 4"
        :text-anchor="timeAnchor(tick.second)"
      >
        {{ tick.label }}
      </text>
    </g>

    <g v-if="!mini" class="hitches">
      <rect
        v-for="tick in hitches"
        :key="`hitch-${tick.second}`"
        :class="tick.capture ? 'hitch-capture' : 'hitch-network'"
        :x="x(tick.second) - 1"
        :y="plot.top + plot.height - 6"
        width="2"
        height="6"
        rx="1"
      />
    </g>

    <path
      v-for="(run, index) in runs"
      v-show="!mini && run.length >= 2"
      :key="`area-${index}`"
      class="area"
      :d="areaPath(run)"
      :fill="`url(#${gradientId})`"
    />

    <path
      v-for="(run, index) in runs"
      v-show="run.length >= 2"
      :key="`line-${index}`"
      class="line"
      :d="linePath(run)"
      :stroke-width="mini ? 1.5 : 2"
    />

    <line
      v-if="points.length === 1"
      class="line"
      :x1="x(points[0].second)"
      :x2="x(points[0].second)"
      :y1="y(0)"
      :y2="y(points[0].rttMs)"
      :stroke-width="mini ? 1.5 : 2"
      stroke-linecap="round"
    />

    <circle
      v-for="(point, index) in points"
      v-show="!isCallout(point) && showPoint(point)"
      :key="`point-${index}`"
      class="point"
      :cx="x(point.second)"
      :cy="y(point.rttMs)"
      :r="points.length === 1 ? 4 : 2.5"
    />

    <g v-if="callout && calloutBox">
      <circle class="point" :cx="x(callout.second)" :cy="y(callout.rttMs)" :r="points.length === 1 ? 4 : 3" />
      <text
        class="callout"
        :x="calloutBox.x"
        :y="calloutBox.y"
        :text-anchor="calloutBox.anchor"
      >
        {{ formatMs(callout.rttMs) }}
      </text>
    </g>
  </svg>
</template>

<style scoped>
.latency-chart {
  display: block;
  overflow: visible;
}
.latency-chart.mini {
  flex: none;
}
.line {
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.area {
  stroke: none;
}
.point {
  fill: currentColor;
}
.grid {
  stroke: var(--line);
  stroke-width: 1;
  opacity: 0.7;
}
.tick-label {
  fill: var(--secondary);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.callout {
  fill: var(--secondary);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.hitch-capture {
  fill: var(--warning);
}
.hitch-network {
  fill: var(--accent);
}
</style>

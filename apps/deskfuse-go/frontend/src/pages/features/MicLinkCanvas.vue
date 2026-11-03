<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  samples?: number[]
  /** 宿主窗口时长（毫秒）：samples 覆盖的音频时间长度。 */
  spanMs?: number
  /** 数据末端距投影读取时刻的时延（毫秒），用于把远端窗口对齐到本机时钟。 */
  ageMs?: number
  active?: boolean
  paused?: boolean
  vertical?: boolean
}>(), {
  samples: () => [],
  spanMs: 0,
  ageMs: 0,
  active: false,
  paused: false,
  vertical: false,
})

// 画布显示一段固定的音频时间，波形以 1× 实时速度从来源插口流向去向插口。
// 显示光标 = 页面时钟 − 平滑后的数据时延 − 300ms：光标永远不会越过已知数据，
// 每次换帧只是同一段历史滚过去一点，不再整体换形、也不会因时延抖动而横向跳动。
const VIEW_SPAN = 1.4
const TRAIL = 0.3

const canvas = ref<HTMLCanvasElement | null>(null)
let context: CanvasRenderingContext2D | null = null
// 宿主窗口：最旧 → 最新。
let values = new Float32Array(0)
let windowSpan = 0
// 最新样本对应的页面时钟（秒）。所有绘制时间都换算到这个时钟上。
let newestAt = 0
// 显示光标：画布最左（来源侧）对应的音频时刻，按真实时间推进。
let cursor = Number.NaN
// 数据时延的平滑值：向上跳变快速跟随（数据迟到时不冻结），回落缓慢（不改变滚动速度）。
let ageSmooth = Number.NaN
let lastFrame = 0
let peak = 0.02
let width = 1
let height = 1
let dpr = 1
let raf = 0
let observer: ResizeObserver | null = null
const reduced = window.matchMedia('(prefers-reduced-motion: reduce)')
let accent = '#3276f3'
let line = 'rgba(0,0,0,.09)'
let surface = '#fff'
// 来源侧淡、去向侧实的线性渐变，保留「声音沿链路流向目的地」的方向暗示。
let flowGradient: CanvasGradient | null = null
// 每帧复用的缓冲，避免 rAF 里反复分配。
let columns = new Float32Array(0)

function nowSeconds(): number {
  return performance.now() / 1000
}

function cssVar(name: string, fallback: string): string {
  const element = canvas.value
  if (!element) return fallback
  return getComputedStyle(element).getPropertyValue(name).trim() || fallback
}

function rgba(color: string, alpha: number): string {
  const value = color.trim()
  if (value.startsWith('#')) {
    const hex = value.slice(1)
    const full = hex.length === 3 ? hex.split('').map((part) => part + part).join('') : hex
    if (full.length === 6) {
      const number = Number.parseInt(full, 16)
      if (Number.isFinite(number)) {
        return `rgba(${(number >> 16) & 255},${(number >> 8) & 255},${number & 255},${alpha})`
      }
    }
  }
  return value
}

function refreshPalette() {
  accent = cssVar('--accent', '#3276f3')
  line = cssVar('--line', 'rgba(0,0,0,.09)')
  surface = cssVar('--surface', '#fff')
  refreshGradient()
}

function refreshGradient() {
  if (!context) { flowGradient = null; return }
  const start = props.vertical ? 16 : 18
  const end = (props.vertical ? height : width) - (props.vertical ? 16 : 18)
  const gradient = props.vertical
    ? context.createLinearGradient(0, start, 0, end)
    : context.createLinearGradient(start, 0, end, 0)
  gradient.addColorStop(0, rgba(accent, 0.55))
  gradient.addColorStop(0.4, accent)
  gradient.addColorStop(1, accent)
  flowGradient = gradient
}

function flowing(): boolean {
  return props.active === true && props.paused !== true
    && values.length > 8 && windowSpan > 0 && Number.isFinite(cursor)
}

/** 在窗口 [0,1] 内线性插值取一个样本。 */
function valueAt(progress: number): number {
  const length = values.length
  if (!length) return 0
  const clamped = Math.max(0, Math.min(1, progress))
  const position = clamped * (length - 1)
  const index = Math.floor(position)
  if (index >= length - 1) return values[length - 1]
  const fraction = position - index
  return values[index] * (1 - fraction) + values[index + 1] * fraction
}

function sampleAt(audioTime: number): number {
  return valueAt((audioTime - (newestAt - windowSpan)) / windowSpan)
}

function applySamples() {
  const input = props.samples ?? []
  const next = new Float32Array(input.length)
  for (let index = 0; index < input.length; index += 1) {
    const value = Number(input[index])
    next[index] = Number.isFinite(value) ? value : 0
  }
  values = next
  windowSpan = Math.max(0, Number(props.spanMs) || 0) / 1000
  const age = Math.max(0, Number(props.ageMs) || 0) / 1000
  if (windowSpan <= 0 || values.length <= 8) {
    cursor = Number.NaN
    ageSmooth = Number.NaN
    peak = 0.02
    return
  }
  // 幅度按窗口峰值缓慢跟随，避免每换一帧整条波形忽大忽小。
  let target = 0
  for (let index = 0; index < values.length; index += 1) target = Math.max(target, Math.abs(values[index]))
  peak += (target - peak) * (target > peak ? 0.5 : 0.08)
  // 时延平滑：向上跳变快速跟随（数据迟到时不冻结），回落缓慢（不改变滚动速度）。
  if (!Number.isFinite(ageSmooth) || Math.abs(age - ageSmooth) > 0.25) {
    // 首次到达或长时间断流后重新对齐：此时数据本身已经换了，可以直接吸收落差。
    ageSmooth = age
  } else {
    ageSmooth += (age - ageSmooth) * (age > ageSmooth ? 0.5 : 0.08)
  }
  // 窗口末端也用同一份平滑时延定位：每次换帧不重排采样相位，波形不会横向抖一下。
  newestAt = nowSeconds() - ageSmooth
  if (!Number.isFinite(cursor)) cursor = newestAt - TRAIL
}

function waveGain(): number {
  // 峰值归一化到链路高度的 15%（峰到峰约三成）；0.004 的噪声地板限制静音放大。
  return Math.min(40, 0.15 / Math.max(peak, 0.004))
}

/** 画布某一列的 y 值（横向布局）：最左是最新样本，向右变旧。 */
function columnY(progress: number, cy: number, amplitude: number): number {
  const value = sampleAt(cursor - progress * VIEW_SPAN) * amplitude
  return Math.max(5, Math.min(height - 5, cy + value))
}

/** 画布某一行的 x 值（纵向布局）：最上是最新样本，向下变旧。 */
function rowX(progress: number, cx: number, amplitude: number): number {
  const value = sampleAt(cursor - progress * VIEW_SPAN) * amplitude
  return Math.max(6, Math.min(width - 6, cx + value))
}

function port(x: number, y: number, time: number, offset: number) {
  if (!context) return
  context.beginPath()
  context.arc(x, y, 4.1, 0, Math.PI * 2)
  context.fillStyle = surface
  context.fill()
  context.strokeStyle = rgba(accent, 0.55)
  context.lineWidth = 1
  context.stroke()
  context.beginPath()
  context.arc(x, y, 2.1, 0, Math.PI * 2)
  context.fillStyle = accent
  context.fill()
  if (!flowing() || reduced.matches) return
  for (let index = 0; index < 2; index += 1) {
    const progress = (time * 0.55 + offset + index * 0.5) % 1
    context.beginPath()
    context.arc(x, y, 5 + progress * 13, 0, Math.PI * 2)
    context.strokeStyle = rgba(accent, (1 - progress) * 0.18)
    context.lineWidth = 1
    context.stroke()
  }
}

function strokeCurve(path: Path2D) {
  if (!context) return
  context.strokeStyle = rgba(accent, 0.12)
  context.lineWidth = 8
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke(path)
  context.strokeStyle = flowGradient ?? accent
  context.lineWidth = 3
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke(path)
}

function drawPacketsHorizontal(time: number, x0: number, x1: number) {
  if (!context) return
  const packets = 6
  const span = Math.max(1, x1 - x0)
  for (let index = 0; index < packets; index += 1) {
    // 数据包从来源插口走向去向插口；位置直接取渲染曲线，包始终贴着波形走。
    const progress = (time * 0.085 + index / packets) % 1
    const x = x0 + progress * span
    const y = columns[Math.max(0, Math.min(columns.length - 1, Math.round(x - x0)))]
    context.beginPath()
    for (let step = 0; step <= 9; step += 1) {
      const trail = Math.max(0, progress - step * 0.006)
      const trailX = x0 + trail * span
      const trailY = columns[Math.max(0, Math.min(columns.length - 1, Math.round(trailX - x0)))]
      if (step === 0) context.moveTo(trailX, trailY)
      else context.lineTo(trailX, trailY)
    }
    context.strokeStyle = rgba(accent, 0.26)
    context.lineWidth = 1.4
    context.lineCap = 'round'
    context.stroke()
    context.beginPath()
    context.arc(x, y, 2.5, 0, Math.PI * 2)
    context.fillStyle = accent
    context.fill()
    context.beginPath()
    context.arc(x, y, 0.9, 0, Math.PI * 2)
    context.fillStyle = surface
    context.fill()
  }
}

function drawPacketsVertical(time: number, y0: number, y1: number) {
  if (!context) return
  const packets = 4
  const span = Math.max(1, y1 - y0)
  for (let index = 0; index < packets; index += 1) {
    const progress = (time * 0.105 + index / packets) % 1
    const y = y0 + progress * span
    const x = columns[Math.max(0, Math.min(columns.length - 1, Math.round(y - y0)))]
    context.beginPath()
    for (let step = 0; step <= 8; step += 1) {
      const trail = Math.max(0, progress - step * 0.008)
      const trailY = y0 + trail * span
      const trailX = columns[Math.max(0, Math.min(columns.length - 1, Math.round(trailY - y0)))]
      if (step === 0) context.moveTo(trailX, trailY)
      else context.lineTo(trailX, trailY)
    }
    context.strokeStyle = rgba(accent, 0.26)
    context.lineWidth = 1.4
    context.lineCap = 'round'
    context.stroke()
    context.beginPath()
    context.arc(x, y, 2.5, 0, Math.PI * 2)
    context.fillStyle = accent
    context.fill()
    context.beginPath()
    context.arc(x, y, 0.9, 0, Math.PI * 2)
    context.fillStyle = surface
    context.fill()
  }
}

function drawIdleLine(horizontal: boolean, axisStart: number, axisEnd: number, center: number) {
  if (!context) return
  context.beginPath()
  if (horizontal) {
    context.moveTo(axisStart, center + 0.5)
    context.lineTo(axisEnd, center + 0.5)
  } else {
    context.moveTo(center + 0.5, axisStart)
    context.lineTo(center + 0.5, axisEnd)
  }
  context.setLineDash([3, 4])
  context.strokeStyle = line
  context.lineWidth = 1
  context.stroke()
  context.setLineDash([])
}

function ensureColumns(size: number) {
  if (columns.length < size) columns = new Float32Array(size)
}

function drawHorizontal(time: number) {
  if (!context) return
  const x0 = 18
  const x1 = width - 18
  const cy = height / 2
  context.clearRect(0, 0, width, height)

  if (!flowing()) {
    drawIdleLine(true, x0, x1, cy)
    port(x0, cy, time, 0)
    port(x1, cy, time, 1.1)
    return
  }

  const amplitude = height * waveGain()
  const count = Math.max(2, Math.min(2048, Math.round(x1 - x0) + 1))
  ensureColumns(count)
  const path = new Path2D()
  for (let index = 0; index < count; index += 1) {
    const x = x0 + (x1 - x0) * (index / (count - 1))
    const y = columnY(index / (count - 1), cy, amplitude)
    columns[index] = y
    if (index === 0) path.moveTo(x, y)
    else path.lineTo(x, y)
  }
  // 传输中只画一条线：真实样本平滑后的连续曲线，从来源插口一直走到去向插口。
  strokeCurve(path)
  if (!reduced.matches) drawPacketsHorizontal(time, x0, x1)
  port(x0, cy, time, 0)
  port(x1, cy, time, 1.1)
}

function drawVertical(time: number) {
  if (!context) return
  const y0 = 16
  const y1 = height - 16
  const cx = width / 2
  const amplitude = Math.min(width * waveGain(), 64)
  context.clearRect(0, 0, width, height)

  if (!flowing()) {
    drawIdleLine(false, y0, y1, cx)
    port(cx, y0, time, 0.25)
    port(cx, y1, time, 1.35)
    return
  }

  const count = Math.max(2, Math.min(2048, Math.round(y1 - y0) + 1))
  ensureColumns(count)
  const path = new Path2D()
  for (let index = 0; index < count; index += 1) {
    const y = y0 + (y1 - y0) * (index / (count - 1))
    const x = rowX(index / (count - 1), cx, amplitude)
    columns[index] = x
    if (index === 0) path.moveTo(x, y)
    else path.lineTo(x, y)
  }
  strokeCurve(path)
  if (!reduced.matches) drawPacketsVertical(time, y0, y1)
  port(cx, y0, time, 0.25)
  port(cx, y1, time, 1.35)
}

function drawFrame(time: number) {
  if (!context) return
  if (props.vertical) drawVertical(time)
  else drawHorizontal(time)
}

function frame(timestamp: number) {
  const time = timestamp / 1000
  lastFrame = time
  if (!flowing()) {
    stopAnimation()
    drawFrame(time)
    return
  }
  // 光标由页面时钟直接给出：速度恒为 1×，只有平滑时延的缓慢漂移参与修正；
  // 数据迟到时最多贴住最新样本，不越过它。
  cursor = Math.min(time - ageSmooth - TRAIL, newestAt - 0.004)
  drawFrame(time)
  raf = requestAnimationFrame(frame)
}

function stopAnimation() {
  if (!raf) return
  cancelAnimationFrame(raf)
  raf = 0
}

/** 静态帧：减弱动态效果、无样本、暂停态都停在这一帧。 */
function drawStill() {
  stopAnimation()
  lastFrame = 0
  if (windowSpan > 0 && values.length > 8) cursor = newestAt - TRAIL
  drawFrame(nowSeconds())
}

function ensureRunning() {
  if (!flowing() || reduced.matches) {
    drawStill()
    return
  }
  if (raf) return
  lastFrame = 0
  raf = requestAnimationFrame(frame)
}

function sizeCanvas() {
  const element = canvas.value
  if (!element) return
  context = element.getContext('2d')
  const rect = element.getBoundingClientRect()
  dpr = Math.min(2, window.devicePixelRatio || 1)
  width = Math.max(1, rect.width)
  height = Math.max(1, rect.height)
  const nextWidth = Math.round(width * dpr)
  const nextHeight = Math.round(height * dpr)
  if (element.width !== nextWidth) element.width = nextWidth
  if (element.height !== nextHeight) element.height = nextHeight
  context?.setTransform(dpr, 0, 0, dpr, 0, 0)
  refreshPalette()
}

function resize() {
  const running = Boolean(raf)
  sizeCanvas()
  // 尺寸变化后若动画在跑，下一帧自然重绘；否则补一张静态帧。
  if (!running) drawStill()
}

function onReducedChange() {
  stopAnimation()
  sizeCanvas()
  ensureRunning()
}

watch(() => props.samples, () => {
  applySamples()
  ensureRunning()
}, { deep: false })

watch(() => [props.active, props.paused, props.vertical], () => {
  stopAnimation()
  sizeCanvas()
  ensureRunning()
})

onMounted(() => {
  applySamples()
  sizeCanvas()
  observer = new ResizeObserver(resize)
  if (canvas.value) observer.observe(canvas.value)
  reduced.addEventListener('change', onReducedChange)
  ensureRunning()
})

onBeforeUnmount(() => {
  stopAnimation()
  observer?.disconnect()
  observer = null
  reduced.removeEventListener('change', onReducedChange)
})
</script>

<template>
  <canvas ref="canvas" aria-hidden="true" />
</template>

<style scoped>
canvas {
  display: block;
  width: 100%;
  height: 100%;
}
</style>

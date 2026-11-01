<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  samples?: number[]
  active?: boolean
  paused?: boolean
  vertical?: boolean
}>(), {
  samples: () => [],
  active: false,
  paused: false,
  vertical: false,
})

const canvas = ref<HTMLCanvasElement | null>(null)
let context: CanvasRenderingContext2D | null = null
let display = new Float32Array(256)
// 显示用缓冲：对真实样本做 3 点平滑，抹掉采样点之间的毛刺，
// 让波形读成一条连续的线；峰值仍按原始样本计算，不虚报电平。
let smooth = new Float32Array(256)
let peak = 0
let smoothPeak = 0.02
let width = 1
let height = 1
let dpr = 1
let raf = 0
let observer: ResizeObserver | null = null
let reduced = window.matchMedia('(prefers-reduced-motion: reduce)')
let accent = '#3276f3'
let line = 'rgba(0,0,0,.09)'
let surface = '#fff'

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
}

function flowing(): boolean {
  return props.active === true && props.paused !== true && display.length > 0
}

function updateSamples() {
  const input = (props.samples ?? []).map(Number).filter(Number.isFinite)
  if (!input.length) {
    display.fill(0)
    smooth.fill(0)
    peak = 0
    smoothPeak = 0.02
    if (reduced.matches) drawFrame(0)
    return
  }
  const length = display.length
  let start = Math.max(0, input.length - length)
  // 把最近的上升过零点放到窗口前面，避免每次刷新时波形横向跳动。
  if (peak > 0.01) {
    const limit = Math.min(input.length - 1, start + Math.max(16, Math.floor(length / 3)))
    for (let index = Math.max(1, start); index < limit; index += 1) {
      if (input[index - 1] <= 0 && input[index] > 0) {
        start = index
        break
      }
    }
  }
  display.fill(0)
  const count = Math.min(length, input.length - start)
  for (let index = 0; index < count; index += 1) display[index] = input[start + index]
  smooth.fill(0)
  for (let index = 0; index < count; index += 1) {
    const prev = index > 0 ? display[index - 1] : display[index]
    const next = index < count - 1 ? display[index + 1] : display[index]
    smooth[index] = display[index] * 0.5 + (prev + next) * 0.25
  }
  peak = 0
  for (let index = 0; index < count; index += 1) peak = Math.max(peak, Math.abs(display[index]))
  const target = Math.max(peak, 0.002)
  smoothPeak += (target - smoothPeak) * (target > smoothPeak ? 0.35 : 0.08)
  if (reduced.matches) drawFrame(0)
}

function waveGain(): number {
  // 真实语音大多在 −40 dBFS 附近，上限 3× 会把整段说话压成近乎直线。
  // 以峰值归一化到链路高度的 15%（峰到峰约三成），波形是一条可读的带子，
  // 而不是铺满整个舞台的面积块；0.004 的噪声地板限制静音时被过度放大。
  return Math.min(40, 0.15 / Math.max(smoothPeak, 0.004))
}

function sampleAt(progress: number): number {
  if (!flowing() || smooth.length === 0) return 0
  const index = Math.max(0, Math.min(smooth.length - 1, Math.floor(progress * smooth.length)))
  return smooth[index]
}

function traceHorizontal(x0: number, x1: number, cy: number, amplitude: number) {
  if (!context) return
  const span = Math.max(1, x1 - x0)
  context.beginPath()
  for (let x = x0; x <= x1; x += 1) {
    const t = (x - x0) / span
    const y = Math.max(5, Math.min(height - 5, cy + sampleAt(t) * amplitude))
    if (x === x0) context.moveTo(x, y)
    else context.lineTo(x, y)
  }
}

function traceVertical(y0: number, y1: number, cx: number, amplitude: number) {
  if (!context) return
  const span = Math.max(1, y1 - y0)
  context.beginPath()
  for (let y = y0; y <= y1; y += 1) {
    const t = (y - y0) / span
    const x = Math.max(6, Math.min(width - 6, cx + sampleAt(t) * amplitude))
    if (y === y0) context.moveTo(x, y)
    else context.lineTo(x, y)
  }
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

function drawPacketsHorizontal(time: number, x0: number, x1: number, cy: number, amplitude: number) {
  if (!context) return
  const packets = 6
  const span = Math.max(1, x1 - x0)
  for (let index = 0; index < packets; index += 1) {
    const progress = (time * 0.085 + index / packets) % 1
    const x = x0 + progress * span
    const y = Math.max(5, Math.min(height - 5, cy + sampleAt(progress) * amplitude))
    context.beginPath()
    for (let step = 0; step <= 9; step += 1) {
      const t = Math.max(0, progress - step * 0.006)
      const tx = x0 + t * span
      const ty = Math.max(5, Math.min(height - 5, cy + sampleAt(t) * amplitude))
      if (step === 0) context.moveTo(tx, ty)
      else context.lineTo(tx, ty)
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

function drawPacketsVertical(time: number, y0: number, y1: number, cx: number, amplitude: number) {
  if (!context) return
  const packets = 4
  const span = Math.max(1, y1 - y0)
  for (let index = 0; index < packets; index += 1) {
    const progress = (time * 0.105 + index / packets) % 1
    const y = y0 + progress * span
    const x = Math.max(6, Math.min(width - 6, cx + sampleAt(progress) * amplitude))
    context.beginPath()
    for (let step = 0; step <= 8; step += 1) {
      const t = Math.max(0, progress - step * 0.008)
      const ty = y0 + t * span
      const tx = Math.max(6, Math.min(width - 6, cx + sampleAt(t) * amplitude))
      if (step === 0) context.moveTo(tx, ty)
      else context.lineTo(tx, ty)
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

function drawHorizontal(time: number) {
  if (!context) return
  const x0 = 18
  const x1 = width - 18
  const cy = height / 2
  const amplitude = height * waveGain()
  context.clearRect(0, 0, width, height)

  if (!flowing()) {
    drawIdleLine(true, x0, x1, cy)
    port(x0, cy, time, 0)
    port(x1, cy, time, 1.1)
    return
  }

  // 传输中只画一条线：真实样本平滑后的连续曲线，从来源插口一直走到去向插口。
  // 不再叠加面积填充，波形不会盖过两侧设备卡。
  traceHorizontal(x0, x1, cy, amplitude)
  context.strokeStyle = rgba(accent, 0.12)
  context.lineWidth = 8
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke()

  const flowGradient = context.createLinearGradient(x0, 0, x1, 0)
  flowGradient.addColorStop(0, rgba(accent, 0.55))
  flowGradient.addColorStop(0.4, accent)
  flowGradient.addColorStop(1, accent)

  traceHorizontal(x0, x1, cy, amplitude)
  context.strokeStyle = flowGradient
  context.lineWidth = 3
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke()

  if (!reduced.matches) drawPacketsHorizontal(time, x0, x1, cy, amplitude)
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

  // 纵向布局同样只画一条线：来源在上、去向在下。
  traceVertical(y0, y1, cx, amplitude)
  context.strokeStyle = rgba(accent, 0.12)
  context.lineWidth = 8
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke()

  const flowGradient = context.createLinearGradient(0, y0, 0, y1)
  flowGradient.addColorStop(0, rgba(accent, 0.55))
  flowGradient.addColorStop(0.4, accent)
  flowGradient.addColorStop(1, accent)

  traceVertical(y0, y1, cx, amplitude)
  context.strokeStyle = flowGradient
  context.lineWidth = 3
  context.lineJoin = 'round'
  context.lineCap = 'round'
  context.stroke()

  if (!reduced.matches) drawPacketsVertical(time, y0, y1, cx, amplitude)
  port(cx, y0, time, 0.25)
  port(cx, y1, time, 1.35)
}

function drawFrame(time: number) {
  if (!context) return
  if (props.vertical) drawVertical(time)
  else drawHorizontal(time)
}

function frame(now: number) {
  drawFrame(now / 1000)
  raf = requestAnimationFrame(frame)
}

function start() {
  cancelAnimationFrame(raf)
  raf = 0
  if (reduced.matches) {
    drawFrame(0)
    return
  }
  raf = requestAnimationFrame(frame)
}

function resize() {
  const element = canvas.value
  if (!element) return
  context = element.getContext('2d')
  const rect = element.getBoundingClientRect()
  dpr = Math.min(2, window.devicePixelRatio || 1)
  width = Math.max(1, rect.width)
  height = Math.max(1, rect.height)
  element.width = Math.round(width * dpr)
  element.height = Math.round(height * dpr)
  context?.setTransform(dpr, 0, 0, dpr, 0, 0)
  refreshPalette()
  if (reduced.matches) drawFrame(0)
}

function onReducedChange() {
  start()
}

watch(() => props.samples, () => {
  updateSamples()
  if (!reduced.matches) start()
}, { deep: false })
watch(() => [props.active, props.paused], () => {
  if (reduced.matches) drawFrame(0)
})

onMounted(() => {
  updateSamples()
  resize()
  observer = new ResizeObserver(resize)
  if (canvas.value) observer.observe(canvas.value)
  reduced.addEventListener('change', onReducedChange)
  start()
})

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
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

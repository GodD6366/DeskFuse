<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'

const host = ref<HTMLElement>()
const canvas = ref<HTMLCanvasElement>()

let ctx: CanvasRenderingContext2D | null = null
let frame = 0
let resizeObserver: ResizeObserver | null = null
let visibilityObserver: IntersectionObserver | null = null
let visible = true
let reduced = false
let startedAt = 0
let width = 0
let height = 0
let scale = 1
let offsetX = 0
let offsetY = 0
let pointerX = 0
let pointerY = 0
let targetX = 0
let targetY = 0
let rafId = 0
let running = false

const W = 1000
const H = 760

function roundRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const radius = Math.min(r, Math.abs(w) / 2, Math.abs(h) / 2)
  context.beginPath()
  context.moveTo(x + radius, y)
  context.arcTo(x + w, y, x + w, y + h, radius)
  context.arcTo(x + w, y + h, x, y + h, radius)
  context.arcTo(x, y + h, x, y, radius)
  context.arcTo(x, y, x + w, y, radius)
  context.closePath()
}

function dot(context: CanvasRenderingContext2D, x: number, y: number, r: number, color: string) {
  context.beginPath()
  context.arc(x, y, r, 0, Math.PI * 2)
  context.fillStyle = color
  context.fill()
}

function cubicPoint(t: number, p0: number, p1: number, p2: number, p3: number) {
  const u = 1 - t
  return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3
}

function bezier(
  context: CanvasRenderingContext2D,
  points: readonly [number, number][],
  t0 = 0,
  t1 = 1,
) {
  const [a, b, c, d] = points
  context.beginPath()
  const steps = 50
  for (let i = 0; i <= steps; i += 1) {
    const t = t0 + ((t1 - t0) * i) / steps
    const x = cubicPoint(t, a[0], b[0], c[0], d[0])
    const y = cubicPoint(t, a[1], b[1], c[1], d[1])
    if (i === 0) context.moveTo(x, y)
    else context.lineTo(x, y)
  }
}

const easeOut = (t: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 3)

function drawWindowA(context: CanvasRenderingContext2D, time: number) {
  const w = 600
  const h = 372
  const cx = 335
  const cy = 300
  const rotation = -0.052

  context.save()
  context.translate(cx, cy)
  context.rotate(rotation)
  const x = -w / 2
  const y = -h / 2

  // shadow plate
  roundRect(context, x + 16, y + 22, w, h, 14)
  context.fillStyle = 'rgba(0, 0, 0, 0.36)'
  context.fill()

  roundRect(context, x, y, w, h, 14)
  context.fillStyle = 'rgba(8, 19, 16, 0.96)'
  context.fill()
  context.strokeStyle = 'rgba(196, 255, 241, 0.34)'
  context.lineWidth = 1.2
  context.stroke()

  // title bar
  context.save()
  roundRect(context, x, y, w, h, 14)
  context.clip()
  context.fillStyle = 'rgba(207, 255, 243, 0.035)'
  context.fillRect(x, y, w, 40)
  ;[
    ['#ff6a3c', x + 22],
    ['#d8b75e', x + 38],
    ['#59f3da', x + 54],
  ].forEach(([color, dx]) => dot(context, Number(dx), y + 19, 4, String(color)))

  context.fillStyle = 'rgba(233, 243, 238, 0.7)'
  context.font = '500 11px "PingFang SC", "Hiragino Sans GB", sans-serif'
  context.textAlign = 'left'
  context.fillText('剪贴板 · 所有设备', x + 76, y + 23)

  context.fillStyle = 'rgba(148, 170, 163, 0.6)'
  context.font = '500 9px "SF Mono", ui-monospace, monospace'
  context.textAlign = 'right'
  context.fillText('⌥ ⌘ V', x + w - 18, y + 23)

  // sidebar
  context.fillStyle = 'rgba(207, 255, 243, 0.025)'
  context.fillRect(x, y + 40, 132, h - 40)
  context.strokeStyle = 'rgba(207, 255, 243, 0.08)'
  context.lineWidth = 1
  context.beginPath()
  context.moveTo(x + 132, y + 40)
  context.lineTo(x + 132, y + h)
  context.stroke()

  const nav = [
    ['全部', '326', true],
    ['图片', '48', false],
    ['文件', '24', false],
    ['我的列表', '6', false],
  ] as const

  nav.forEach(([label, count, active], index) => {
    const rowY = y + 62 + index * 34
    if (active) {
      roundRect(context, x + 12, rowY, 108, 26, 6)
      context.fillStyle = 'rgba(89, 243, 218, 0.1)'
      context.fill()
    }
    dot(context, x + 24, rowY + 13, 2.6, active ? '#59f3da' : 'rgba(148,170,163,.5)')
    context.fillStyle = active ? '#e9f3ee' : 'rgba(148, 170, 163, 0.85)'
    context.font = '500 10.5px "PingFang SC", sans-serif'
    context.textAlign = 'left'
    context.fillText(label, x + 34, rowY + 17)
    ctx!.fillStyle = 'rgba(148, 170, 163, 0.55)'
    ctx!.font = '500 8.5px "SF Mono", monospace'
    ctx!.textAlign = 'right'
    ctx!.fillText(count, x + 108, rowY + 17)
  })

  // bottom sync note in sidebar
  context.fillStyle = 'rgba(148, 170, 163, 0.65)'
  context.font = '400 8.5px "PingFang SC", sans-serif'
  context.textAlign = 'left'
  context.fillText('自动同步 · 已配对 2 台', x + 18, y + h - 20)
  dot(context, x + 13, y + h - 24, 2, '#59f3da')

  // main pane
  const mainX = x + 148
  const mainW = w - 164
  roundRect(context, mainX, y + 54, mainW, 27, 7)
  context.fillStyle = 'rgba(207, 255, 243, 0.045)'
  context.fill()
  context.fillStyle = 'rgba(148, 170, 163, 0.7)'
  context.font = '400 9.5px "PingFang SC", sans-serif'
  context.textAlign = 'left'
  context.fillText('搜索剪贴板', mainX + 12, y + 71)
  context.textAlign = 'right'
  context.fillText('⌘ F', mainX + mainW - 12, y + 71)

  // cards
  const cardW = (mainW - 10) / 2
  const cardH = 82
  const cards = [
    { title: 'https://deskfuse.godd.cc', meta: '13:24 · 这台 Mac', active: true, image: false },
    { title: '设计稿评审改到 15:00', meta: '13:23 · MacBook Pro', active: false, image: false },
    { title: 'IMG_2887.JPG', meta: '2.4 MB · 图片', active: false, image: true },
    { title: 'release / v0.7.2', meta: '13:20 · 这台 Mac', active: false, image: false },
  ]

  cards.forEach((card, index) => {
    const col = index % 2
    const row = Math.floor(index / 2)
    const cardX = mainX + col * (cardW + 10)
    const cardY = y + 94 + row * (cardH + 10)
    roundRect(context, cardX, cardY, cardW, cardH, 9)
    context.fillStyle = card.active ? 'rgba(89, 243, 218, 0.075)' : 'rgba(255, 255, 255, 0.035)'
    context.fill()
    context.strokeStyle = card.active
      ? 'rgba(89, 243, 218, 0.48)'
      : 'rgba(207, 255, 243, 0.075)'
    context.lineWidth = card.active ? 1.3 : 1
    context.stroke()

    if (card.image) {
      const grad = context.createLinearGradient(cardX + 12, cardY + 12, cardX + 72, cardY + 54)
      grad.addColorStop(0, 'rgba(89, 243, 218, 0.65)')
      grad.addColorStop(0.55, 'rgba(82, 125, 255, 0.5)')
      grad.addColorStop(1, 'rgba(255, 106, 60, 0.4)')
      roundRect(context, cardX + 12, cardY + 12, 58, 40, 6)
      context.fillStyle = grad
      context.fill()
      context.fillStyle = 'rgba(233, 243, 238, 0.9)'
      context.font = '500 9px "PingFang SC", sans-serif'
      context.textAlign = 'left'
      context.fillText(card.title, cardX + 12, cardY + 68)
    } else {
      context.fillStyle = 'rgba(233, 243, 238, 0.92)'
      context.font = '500 10.5px "PingFang SC", sans-serif'
      context.textAlign = 'left'
      const maxWidth = cardW - 24
      const words = card.title.split('')
      let line = ''
      let lineY = cardY + 26
      for (const char of words) {
        if (context.measureText(line + char).width > maxWidth) {
          context.fillText(line, cardX + 12, lineY)
          line = char
          lineY += 15
        } else {
          line += char
        }
      }
      context.fillText(line, cardX + 12, lineY)
    }

    context.fillStyle = 'rgba(148, 170, 163, 0.7)'
    context.font = '400 8px "SF Mono", monospace'
    context.textAlign = 'left'
    context.fillText(card.meta, cardX + 12, cardY + cardH - 10)
  })

  // scanline
  const scan = ((time / 4600) % 1) * (h - 40)
  context.fillStyle = 'rgba(89, 243, 218, 0.055)'
  context.fillRect(x, y + 40 + scan, w, 1.2)

  context.restore()
  context.restore()
}

function drawWindowB(context: CanvasRenderingContext2D) {
  const w = 500
  const h = 320
  const cx = 706
  const cy = 528
  const rotation = 0.042

  context.save()
  context.translate(cx, cy)
  context.rotate(rotation)
  const x = -w / 2
  const y = -h / 2

  roundRect(context, x + 16, y + 22, w, h, 14)
  context.fillStyle = 'rgba(0, 0, 0, 0.3)'
  context.fill()

  roundRect(context, x, y, w, h, 14)
  context.fillStyle = 'rgba(9, 22, 18, 0.97)'
  context.fill()
  context.strokeStyle = 'rgba(89, 243, 218, 0.6)'
  context.lineWidth = 1.3
  context.stroke()

  context.save()
  roundRect(context, x, y, w, h, 14)
  context.clip()

  context.fillStyle = 'rgba(89, 243, 218, 0.045)'
  context.fillRect(x, y, w, 40)
  ;[
    ['#ff6a3c', x + 22],
    ['#d8b75e', x + 38],
    ['#59f3da', x + 54],
  ].forEach(([color, dx]) => dot(context, Number(dx), y + 19, 4, String(color)))

  context.fillStyle = 'rgba(233, 243, 238, 0.7)'
  context.font = '500 11px "PingFang SC", sans-serif'
  context.textAlign = 'left'
  context.fillText('MacBook Pro · 剪贴板', x + 76, y + 23)

  roundRect(context, x + w - 78, y + 12, 62, 18, 9)
  context.fillStyle = 'rgba(89, 243, 218, 0.12)'
  context.fill()
  context.fillStyle = '#59f3da'
  context.font = '500 8px "PingFang SC", sans-serif'
  context.textAlign = 'center'
  context.fillText('已同步', x + w - 47, y + 24)

  // received card
  roundRect(context, x + 18, y + 56, w - 36, 128, 11)
  context.fillStyle = 'rgba(207, 255, 243, 0.045)'
  context.fill()
  context.strokeStyle = 'rgba(89, 243, 218, 0.22)'
  context.lineWidth = 1
  context.stroke()

  context.fillStyle = 'rgba(148, 170, 163, 0.85)'
  context.font = '400 9px "PingFang SC", sans-serif'
  context.textAlign = 'left'
  context.fillText('来自 · 这台 Mac', x + 34, y + 78)

  context.fillStyle = '#e9f3ee'
  context.font = '600 18px "PingFang SC", sans-serif'
  context.fillText('https://', x + 34, y + 108)
  context.fillStyle = '#59f3da'
  context.font = '600 18px "PingFang SC", sans-serif'
  context.fillText('deskfuse.godd.cc', x + 112, y + 108)

  context.fillStyle = 'rgba(148, 170, 163, 0.75)'
  context.font = '400 9px "SF Mono", monospace'
  context.fillText('TEXT · 25 BYTES · 原格式保留', x + 34, y + 130)

  roundRect(context, x + w - 120, y + 145, 86, 24, 6)
  context.fillStyle = 'rgba(89, 243, 218, 0.14)'
  context.fill()
  context.fillStyle = '#59f3da'
  context.font = '500 9px "PingFang SC", sans-serif'
  context.textAlign = 'center'
  context.fillText('粘 贴', x + w - 77, y + 161)

  // bottom rows
  const rows = [
    ['设计稿 · 首页.png', '2.4 MB'],
    ['项目资料 /', '文件夹'],
  ]
  rows.forEach(([label, meta], index) => {
    const rowY = y + 200 + index * 34
    roundRect(context, x + 18, rowY, w - 36, 26, 7)
    context.fillStyle = 'rgba(255, 255, 255, 0.025)'
    context.fill()
    dot(context, x + 32, rowY + 13, 3, index === 0 ? 'rgba(255, 106, 60, 0.8)' : 'rgba(89, 243, 218, 0.65)')
    context.fillStyle = 'rgba(233, 243, 238, 0.82)'
    context.font = '400 9.5px "PingFang SC", sans-serif'
    context.textAlign = 'left'
    context.fillText(label, x + 44, rowY + 17)
    context.fillStyle = 'rgba(148, 170, 163, 0.65)'
    context.font = '400 8px "SF Mono", monospace'
    context.textAlign = 'right'
    context.fillText(meta, x + w - 30, rowY + 17)
  })

  context.restore()
  context.restore()
}

function drawLink(context: CanvasRenderingContext2D, time: number, intro: number) {
  const points: readonly [number, number][] = [
    [600, 140],
    [880, 50],
    [980, 240],
    [935, 425]
  ]

  context.save()
  context.lineCap = 'round'

  const pulse = 0.75 + Math.sin(time / 900) * 0.1
  context.strokeStyle = 'rgba(89, 243, 218, 0.1)'
  context.lineWidth = 9
  bezier(context, points, 0, intro)
  context.stroke()

  context.strokeStyle = `rgba(89, 243, 218, ${0.72 * pulse})`
  context.lineWidth = 1.6
  context.shadowColor = 'rgba(89, 243, 218, 0.55)'
  context.shadowBlur = 16
  bezier(context, points, 0, intro)
  context.stroke()
  context.shadowBlur = 0

  context.strokeStyle = 'rgba(207, 255, 243, 0.22)'
  context.lineWidth = 0.8
  bezier(context, points, 0, intro)
  context.stroke()

  // moving packets
  if (intro >= 1) {
    const travel = (time / 3000) % 1
    const t = easeOut(travel)
    const px = cubicPoint(t, 600, 880, 980, 935)
    const py = cubicPoint(t, 140, 50, 240, 425)
    context.shadowColor = 'rgba(255, 106, 60, 0.7)'
    context.shadowBlur = 14
    dot(context, px, py, 3.6, '#ff8a64')
    context.shadowBlur = 0
    context.strokeStyle = 'rgba(255, 106, 60, 0.35)'
    context.lineWidth = 1
    context.beginPath()
    context.arc(px, py, 9 + Math.sin(time / 260) * 2, 0, Math.PI * 2)
    context.stroke()
  }

  // ports
  ;[
    [600, 140],
    [935, 425],
  ].forEach(([x, y]) => {
    dot(context, x, y, 5, '#06100e')
    context.strokeStyle = 'rgba(89, 243, 218, 0.9)'
    context.lineWidth = 1.4
    context.beginPath()
    context.arc(x, y, 5, 0, Math.PI * 2)
    context.stroke()
    dot(context, x, y, 1.6, '#59f3da')
  })

  context.restore()
}

function drawAnnotations(context: CanvasRenderingContext2D, intro: number) {
  context.save()
  context.globalAlpha = intro * 0.9

  context.strokeStyle = 'rgba(148, 170, 163, 0.4)'
  context.lineWidth = 0.8
  context.fillStyle = 'rgba(148, 170, 163, 0.8)'
  context.font = '500 9px "SF Mono", monospace'

  const corners: [number, number][] = [
    [28, 28],
    [972, 28],
    [28, 732],
    [972, 732],
  ]
  corners.forEach(([x, y]) => {
    context.beginPath()
    context.moveTo(x - 5, y)
    context.lineTo(x + 5, y)
    context.moveTo(x, y - 5)
    context.lineTo(x, y + 5)
    context.stroke()
  })

  context.textAlign = 'left'
  context.fillText('LOCAL LINK / 02', 48, 36)
  context.fillStyle = 'rgba(89, 243, 218, 0.85)'
  dot(context, 38, 33, 2.2, '#59f3da')

  context.textAlign = 'right'
  context.fillStyle = 'rgba(148, 170, 163, 0.75)'
  context.fillText('QUIC · 42424', 950, 36)

  // ruler ticks
  for (let i = 0; i < 18; i += 1) {
    const y = 150 + i * 26
    context.strokeStyle = i % 4 === 0 ? 'rgba(148, 170, 163, 0.38)' : 'rgba(148, 170, 163, 0.18)'
    context.beginPath()
    context.moveTo(30, y)
    context.lineTo(i % 4 === 0 ? 40 : 35, y)
    context.stroke()
  }

  // label at arc
  context.fillStyle = 'rgba(148, 170, 163, 0.8)'
  context.font = '500 9px "SF Mono", monospace'
  context.textAlign = 'left'
  context.fillText('设备间加密', 812, 96)
  context.strokeStyle = 'rgba(148, 170, 163, 0.28)'
  context.beginPath()
  context.moveTo(810, 102)
  context.lineTo(826, 128)
  context.stroke()

  // floating file chip near bottom
  roundRect(context, 238, 642, 168, 46, 8)
  context.fillStyle = 'rgba(10, 23, 20, 0.92)'
  context.fill()
  context.strokeStyle = 'rgba(255, 106, 60, 0.32)'
  context.lineWidth = 1
  context.stroke()
  context.fillStyle = 'rgba(255, 106, 60, 0.9)'
  roundRect(context, 252, 654, 22, 22, 4)
  context.fill()
  context.fillStyle = '#06100e'
  context.font = '600 9px "PingFang SC", sans-serif'
  context.textAlign = 'center'
  context.fillText('件', 263, 669)
  context.textAlign = 'left'
  context.fillStyle = 'rgba(233, 243, 238, 0.9)'
  context.font = '500 10px "PingFang SC", sans-serif'
  context.fillText('设计稿 · 首页.png', 284, 664)
  context.fillStyle = 'rgba(148, 170, 163, 0.7)'
  context.font = '400 8px "SF Mono", monospace'
  context.fillText('2.4 MB / TRANSFER', 284, 678)

  context.restore()
}

function draw(now: number) {
  if (!ctx) return
  const context = ctx
  context.setTransform(1, 0, 0, 1, 0, 0)
  context.clearRect(0, 0, canvas.value?.width ?? 0, canvas.value?.height ?? 0)

  const dpr = Math.min(window.devicePixelRatio || 1, 2)
  context.setTransform(
    dpr * scale,
    0,
    0,
    dpr * scale,
    dpr * (offsetX + pointerX * 7),
    dpr * (offsetY + pointerY * 5),
  )
  context.translate(0, 0)

  const elapsed = reduced ? 2600 : now - startedAt
  const intro = reduced ? 1 : easeOut(elapsed / 1150)
  const time = reduced ? 0 : now

  // central orbit
  context.save()
  context.strokeStyle = 'rgba(89, 243, 218, 0.07)'
  context.lineWidth = 1
  context.beginPath()
  context.arc(500, 380, 330, Math.PI * 0.78, Math.PI * 1.86)
  context.stroke()
  context.setLineDash([2, 12])
  context.strokeStyle = 'rgba(148, 170, 163, 0.12)'
  context.beginPath()
  context.arc(500, 380, 368, Math.PI * 0.9, Math.PI * 1.6)
  context.stroke()
  context.restore()

  context.save()
  context.globalAlpha = intro
  context.translate(0, (1 - intro) * 16)

  drawWindowA(context, time)
  drawWindowB(context)
  drawLink(context, time, intro)
  drawAnnotations(context, intro)

  context.restore()
}

function startLoop() {
  if (running || reduced) return
  running = true
  rafId = requestAnimationFrame(animate)
}

function stopLoop() {
  running = false
  cancelAnimationFrame(rafId)
}

function animate(now: number) {
  if (!running) return
  if (document.hidden) {
    stopLoop()
    return
  }

  pointerX += (targetX - pointerX) * 0.06
  pointerY += (targetY - pointerY) * 0.06
  draw(now)
  rafId = requestAnimationFrame(animate)
}

function onVisibilityChange() {
  if (document.hidden) stopLoop()
  else if (visible) startLoop()
}

function resize() {
  if (!canvas.value || !host.value) return
  const rect = host.value.getBoundingClientRect()
  width = Math.max(1, rect.width)
  height = Math.max(1, rect.height)
  const dpr = Math.min(window.devicePixelRatio || 1, 2)
  canvas.value.width = Math.round(width * dpr)
  canvas.value.height = Math.round(height * dpr)
  canvas.value.style.width = `${width}px`
  canvas.value.style.height = `${height}px`

  scale = Math.min(width / W, height / H)
  offsetX = (width - W * scale) / 2
  offsetY = (height - H * scale) / 2

  draw(performance.now())
}

function onPointerMove(event: PointerEvent) {
  if (reduced || !host.value) return
  const rect = host.value.getBoundingClientRect()
  targetX = ((event.clientX - rect.left) / rect.width - 0.5) * 2
  targetY = ((event.clientY - rect.top) / rect.height - 0.5) * 2
}

function onPointerLeave() {
  targetX = 0
  targetY = 0
}

onMounted(() => {
  if (!canvas.value || !host.value) return
  const context = canvas.value.getContext('2d')
  if (!context) return
  ctx = context
  reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  startedAt = performance.now()

  resizeObserver = new ResizeObserver(resize)
  resizeObserver.observe(host.value)

  visibilityObserver = new IntersectionObserver(
    (entries) => {
      visible = entries.some((entry) => entry.isIntersecting)
      if (visible) startLoop()
      else stopLoop()
    },
    { threshold: 0.01 },
  )
  visibilityObserver.observe(host.value)

  host.value.addEventListener('pointermove', onPointerMove)
  host.value.addEventListener('pointerleave', onPointerLeave)
  document.addEventListener('visibilitychange', onVisibilityChange)

  resize()
  if (reduced) {
    draw(performance.now())
    return
  }
  startLoop()
})

onBeforeUnmount(() => {
  stopLoop()
  resizeObserver?.disconnect()
  visibilityObserver?.disconnect()
  host.value?.removeEventListener('pointermove', onPointerMove)
  host.value?.removeEventListener('pointerleave', onPointerLeave)
  document.removeEventListener('visibilitychange', onVisibilityChange)
})
</script>

<template>
  <figure ref="host" class="link-scene" aria-label="两台 Mac 通过本地链路同步剪贴板的示意">
    <canvas ref="canvas" aria-hidden="true"></canvas>
    <figcaption class="link-scene__caption">
      <span class="link-scene__status"><i aria-hidden="true"></i>本地链路已连接</span>
      <span class="link-scene__detail">QUIC · 设备间加密 · 02 台</span>
    </figcaption>
  </figure>
</template>

<style scoped>
.link-scene {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 320px;
  overflow: visible;
  isolation: isolate;
}

.link-scene::before {
  content: "";
  position: absolute;
  inset: 12% 4% 8% 8%;
  z-index: -1;
  border-radius: 50%;
  background: radial-gradient(
    ellipse at 58% 45%,
    rgba(89, 243, 218, 0.16),
    rgba(89, 243, 218, 0.03) 38%,
    transparent 68%
  );
  filter: blur(18px);
}

canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
}

.link-scene__caption {
  position: absolute;
  right: 3.5%;
  bottom: 3%;
  left: 5%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.04em;
  pointer-events: none;
}

.link-scene__status {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--muted);
}

.link-scene__status i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--aqua);
  box-shadow: 0 0 8px rgba(89, 243, 218, 0.8);
  animation: link-blink 2.8s ease-in-out infinite;
}

.link-scene__detail {
  text-align: right;
}

@keyframes link-blink {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

@media (max-width: 900px) {
  .link-scene {
    min-height: 300px;
  }

  .link-scene__caption {
    right: 5%;
    bottom: 1%;
    font-size: 8.5px;
  }
}

@media (max-width: 560px) {
  .link-scene__caption {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .link-scene__status i {
    animation: none;
  }
}
</style>

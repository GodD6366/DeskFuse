<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'

/*
 * 首屏的屏幕排布，与应用「键鼠共享」页同一套规则：
 * 几台电脑的屏幕按真实位置拼在一起，光标只能从两块屏幕相接的那段边穿过去，
 * 光标所在的屏幕亮成橙色。屏幕可以拖动，松手后吸附到相邻屏幕的边上。
 * 坐标都用舞台的逻辑单位，渲染时换算成百分比。
 */

interface Screen {
  id: string
  name: string
  x: number
  y: number
  w: number
  h: number
}

interface Point {
  x: number
  y: number
}

const W = 540
const H = 330
/** 两块屏幕相接至少要有这么长，才算能穿过去 */
const MIN_SHARED = 28

const screens = reactive<Screen[]>([
  { id: 'mbp', name: 'MacBook Pro', x: 0, y: 0, w: 210, h: 132 },
  { id: 'mini', name: 'Mac mini', x: 210, y: 22, w: 210, h: 132 },
  { id: 'imac', name: 'iMac', x: 44, y: 132, w: 166, h: 104 },
])

const cursor = reactive({ id: 'mbp', u: 0.42, v: 0.46 })
const dragging = ref<{ id: string; offset: Point; start: Point; pointerId: number } | null>(null)
const stage = ref<HTMLElement | null>(null)
const live = ref(false)
/** 松手后屏幕滑到吸附位置的那段时间，光标跟着一起滑 */
const settling = ref(false)
let settleTimer: ReturnType<typeof setTimeout> | undefined
const canHover = ref(true)

const byId = (id: string) => screens.find((s) => s.id === id)!
const activeName = computed(() => byId(cursor.id).name)

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi)

function inside(p: Point, s: Screen, eps = 0) {
  return p.x >= s.x - eps && p.x <= s.x + s.w + eps && p.y >= s.y - eps && p.y <= s.y + s.h + eps
}

function clampInto(p: Point, s: Screen): Point {
  return { x: clamp(p.x, s.x + 1, s.x + s.w - 1), y: clamp(p.y, s.y + 1, s.y + s.h - 1) }
}

function cursorPoint(): Point {
  const s = byId(cursor.id)
  return { x: s.x + cursor.u * s.w, y: s.y + cursor.v * s.h }
}

function setCursor(s: Screen, p: Point) {
  cursor.id = s.id
  cursor.u = (p.x - s.x) / s.w
  cursor.v = (p.y - s.y) / s.h
}

/** 从屏幕内的 c 朝 p 走，返回离开屏幕 s 时碰到的边界点 */
function exitPoint(c: Point, p: Point, s: Screen): Point {
  const dx = p.x - c.x
  const dy = p.y - c.y
  const tx = dx > 0 ? (s.x + s.w - c.x) / dx : dx < 0 ? (s.x - c.x) / dx : Infinity
  const ty = dy > 0 ? (s.y + s.h - c.y) / dy : dy < 0 ? (s.y - c.y) / dy : Infinity
  const t = clamp(Math.min(tx, ty), 0, 1)
  return { x: c.x + dx * t, y: c.y + dy * t }
}

/** 把光标移向 p：屏幕内直接到；越界时只有出口处挨着另一块屏幕才过去，否则停在边上 */
function moveCursor(p: Point) {
  let s = byId(cursor.id)
  let c = cursorPoint()
  for (let hop = 0; hop < 4; hop++) {
    if (inside(p, s)) {
      c = p
      break
    }
    const e = exitPoint(c, p, s)
    const next = screens.find((t) => t !== s && inside(e, t, 0.75))
    if (!next) {
      c = clampInto(p, s)
      break
    }
    s = next
    c = clampInto(e, next)
  }
  setCursor(s, c)
}

function sharedLength(a: Screen, b: Screen): number {
  const touchX = Math.abs(a.x + a.w - b.x) < 0.5 || Math.abs(b.x + b.w - a.x) < 0.5
  const touchY = Math.abs(a.y + a.h - b.y) < 0.5 || Math.abs(b.y + b.h - a.y) < 0.5
  if (touchX) return Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y)
  if (touchY) return Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)
  return 0
}

function overlaps(a: Screen, b: Screen) {
  return a.x < b.x + b.w - 0.5 && b.x < a.x + a.w - 0.5 && a.y < b.y + b.h - 0.5 && b.y < a.y + a.h - 0.5
}

function neighbours(s: Screen, list: Screen[] = screens) {
  return list.filter((t) => t !== s && sharedLength(s, t) >= MIN_SHARED - 0.5)
}

function connected(list: Screen[]) {
  const seen = new Set([list[0]])
  const queue = [list[0]]
  while (queue.length) {
    for (const n of neighbours(queue.shift()!, list)) {
      if (!seen.has(n)) {
        seen.add(n)
        queue.push(n)
      }
    }
  }
  return seen.size === list.length
}

/** 松手后贴到最近的可用位置：不重叠、至少相接 MIN_SHARED，并且整组屏幕仍然连成一片 */
function snap(d: Screen, start: Point) {
  const candidates: Point[] = []
  for (const o of screens) {
    if (o === d) continue
    const y = clamp(d.y, o.y - d.h + MIN_SHARED, o.y + o.h - MIN_SHARED)
    const x = clamp(d.x, o.x - d.w + MIN_SHARED, o.x + o.w - MIN_SHARED)
    candidates.push({ x: o.x - d.w, y }, { x: o.x + o.w, y }, { x, y: o.y - d.h }, { x, y: o.y + o.h })
  }
  candidates.sort((a, b) => Math.hypot(a.x - d.x, a.y - d.y) - Math.hypot(b.x - d.x, b.y - d.y))

  for (const c of candidates) {
    const moved = { ...d, ...c }
    const rest = screens.filter((s) => s !== d)
    if (rest.some((s) => overlaps(moved, s))) continue
    if (!connected([moved, ...rest])) continue
    d.x = c.x
    d.y = c.y
    return
  }
  d.x = start.x
  d.y = start.y
}

/** 视图变换：整组屏幕居中，排得太长时整体缩小，屏幕自身的逻辑坐标不动 */
const view = reactive({ k: 1, ox: 0, oy: 0 })
const PAD = 16

function fitView() {
  const minX = Math.min(...screens.map((s) => s.x))
  const minY = Math.min(...screens.map((s) => s.y))
  const bw = Math.max(...screens.map((s) => s.x + s.w)) - minX
  const bh = Math.max(...screens.map((s) => s.y + s.h)) - minY
  view.k = Math.min(1, (W - PAD * 2) / bw, (H - PAD * 2) / bh)
  view.ox = (W - bw * view.k) / 2 - minX * view.k
  view.oy = (H - bh * view.k) / 2 - minY * view.k
}

/** 指针位置换算成屏幕的逻辑坐标 */
function toStage(event: PointerEvent): Point {
  const rect = stage.value!.getBoundingClientRect()
  return {
    x: (((event.clientX - rect.left) / rect.width) * W - view.ox) / view.k,
    y: (((event.clientY - rect.top) / rect.height) * H - view.oy) / view.k,
  }
}

// —— 自动演示：沿相邻关系从一块屏幕走到下一块 ——

let raf = 0
let resumeTimer: ReturnType<typeof setTimeout> | undefined
let route: Point[] = []
let segment: { from: Point; to: Point; start: number; duration: number } | null = null
let pauseUntil = 0
let previousId = ''
let visible = true
const reducedMotion =
  typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

function planRoute() {
  const s = byId(cursor.id)
  const options = neighbours(s)
  if (!options.length) return
  const next = options.find((n) => n.id !== previousId) ?? options[0]
  previousId = s.id

  // 穿过两块屏幕相接那段边的中点
  let m: Point
  let normal: Point
  if (Math.abs(s.x + s.w - next.x) < 0.5 || Math.abs(next.x + next.w - s.x) < 0.5) {
    const y = (Math.max(s.y, next.y) + Math.min(s.y + s.h, next.y + next.h)) / 2
    const x = next.x > s.x ? next.x : s.x
    m = { x, y }
    normal = { x: next.x > s.x ? 1 : -1, y: 0 }
  } else {
    const x = (Math.max(s.x, next.x) + Math.min(s.x + s.w, next.x + next.w)) / 2
    const y = next.y > s.y ? next.y : s.y
    m = { x, y }
    normal = { x: 0, y: next.y > s.y ? 1 : -1 }
  }
  route = [
    { x: m.x - normal.x * 14, y: m.y - normal.y * 14 },
    { x: m.x + normal.x * 14, y: m.y + normal.y * 14 },
    { x: next.x + next.w * (0.35 + Math.random() * 0.3), y: next.y + next.h * (0.35 + Math.random() * 0.3) },
  ]
}

const ease = (t: number) => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2)

function tick(now: number) {
  raf = requestAnimationFrame(tick)
  if (!visible || dragging.value || now < pauseUntil) return
  if (!segment) {
    if (!route.length) {
      planRoute()
      if (!route.length) return
    }
    const from = cursorPoint()
    const to = route.shift()!
    const distance = Math.hypot(to.x - from.x, to.y - from.y)
    segment = { from, to, start: now, duration: Math.max(160, distance * 3.4) }
  }
  const t = Math.min(1, (now - segment.start) / segment.duration)
  const k = ease(t)
  moveCursor({
    x: segment.from.x + (segment.to.x - segment.from.x) * k,
    y: segment.from.y + (segment.to.y - segment.from.y) * k,
  })
  if (t >= 1) {
    segment = null
    if (!route.length) pauseUntil = now + 900
  }
}

function startAutoplay() {
  if (reducedMotion || raf) return
  route = []
  segment = null
  raf = requestAnimationFrame(tick)
}

function stopAutoplay() {
  cancelAnimationFrame(raf)
  raf = 0
  clearTimeout(resumeTimer)
}

// —— 指针与键盘 ——

function onPointerEnter(event: PointerEvent) {
  if (event.pointerType !== 'mouse') return
  live.value = true
  stopAutoplay()
}

function onPointerLeave(event: PointerEvent) {
  if (event.pointerType !== 'mouse' || dragging.value) return
  live.value = false
  clearTimeout(resumeTimer)
  resumeTimer = setTimeout(startAutoplay, 2400)
}

function onPointerMove(event: PointerEvent) {
  const p = toStage(event)
  const drag = dragging.value
  if (drag && drag.pointerId === event.pointerId) {
    const s = byId(drag.id)
    const left = -view.ox / view.k
    const top = -view.oy / view.k
    s.x = clamp(p.x - drag.offset.x, left, left + W / view.k - s.w)
    s.y = clamp(p.y - drag.offset.y, top, top + H / view.k - s.h)
    return
  }
  if (event.pointerType === 'mouse') moveCursor(p)
}

function onScreenDown(event: PointerEvent, s: Screen) {
  if (event.button !== 0) return
  event.preventDefault()
  const p = toStage(event)
  stage.value!.setPointerCapture(event.pointerId)
  dragging.value = { id: s.id, offset: { x: p.x - s.x, y: p.y - s.y }, start: { x: s.x, y: s.y }, pointerId: event.pointerId }
}

function onPointerUp(event: PointerEvent) {
  const drag = dragging.value
  if (!drag || drag.pointerId !== event.pointerId) return
  snap(byId(drag.id), drag.start)
  fitView()
  settling.value = true
  clearTimeout(settleTimer)
  settleTimer = setTimeout(() => (settling.value = false), 340)
  dragging.value = null
  route = []
  segment = null
}

function onKeydown(event: KeyboardEvent) {
  const step = event.shiftKey ? 40 : 14
  const delta: Record<string, Point> = {
    ArrowLeft: { x: -step, y: 0 },
    ArrowRight: { x: step, y: 0 },
    ArrowUp: { x: 0, y: -step },
    ArrowDown: { x: 0, y: step },
  }
  const d = delta[event.key]
  if (!d) return
  event.preventDefault()
  stopAutoplay()
  const c = cursorPoint()
  moveCursor({ x: c.x + d.x, y: c.y + d.y })
}

let observer: IntersectionObserver | undefined

onMounted(() => {
  canHover.value = window.matchMedia('(hover: hover)').matches
  fitView()
  observer = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting
  })
  if (stage.value) observer.observe(stage.value)
  startAutoplay()
})

onBeforeUnmount(() => {
  stopAutoplay()
  clearTimeout(settleTimer)
  observer?.disconnect()
})

const pct = (v: number, total: number) => `${(v / total) * 100}%`
const screenStyle = (s: Screen) => ({
  left: pct(s.x * view.k + view.ox, W),
  top: pct(s.y * view.k + view.oy, H),
  width: pct(s.w * view.k, W),
  height: pct(s.h * view.k, H),
})
const cursorStyle = computed(() => {
  const p = cursorPoint()
  return { left: pct(p.x * view.k + view.ox, W), top: pct(p.y * view.k + view.oy, H) }
})
</script>

<template>
  <figure class="desk">
    <div
      ref="stage"
      class="desk__stage"
      :class="{ 'is-live': live, 'is-dragging': dragging, 'is-settling': settling }"
      tabindex="0"
      role="application"
      aria-label="屏幕排布演示。三台电脑的屏幕拼在一起，光标只能从相接的边移到另一台。用方向键移动光标。"
      @pointerenter="onPointerEnter"
      @pointerleave="onPointerLeave"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @keydown="onKeydown"
    >
      <div
        v-for="s in screens"
        :key="s.id"
        class="screen"
        :class="{ 'is-active': cursor.id === s.id, 'is-held': dragging?.id === s.id }"
        :style="screenStyle(s)"
        @pointerdown="onScreenDown($event, s)"
      >
        <div class="screen__face">
          <span class="screen__bar" aria-hidden="true"></span>
          <span class="screen__name">{{ s.name }}</span>
        </div>
      </div>

      <svg class="desk__cursor" :style="cursorStyle" viewBox="0 0 20 28" aria-hidden="true">
        <path
          d="M2 2 L2 22 L7.2 17.4 L10.6 25.2 L14 23.8 L10.7 16.2 L17.4 16.2 Z"
          fill="#111"
          stroke="#fff"
          stroke-width="1.6"
          stroke-linejoin="round"
        />
      </svg>
    </div>

    <figcaption class="desk__caption">
      <span class="desk__now">
        键鼠正在 <b>{{ activeName }}</b> 上
      </span>
      <span class="desk__hint">{{ canHover ? '移动鼠标试试，拖动屏幕可以换位置' : '拖动屏幕可以换位置' }}</span>
    </figcaption>
  </figure>
</template>

<style scoped>
.desk {
  margin: 0;
}

.desk__stage {
  position: relative;
  aspect-ratio: 540 / 330;
  border-radius: var(--r-card);
  background: var(--c-paper);
  box-shadow: inset 0 0 0 1px var(--c-line);
  touch-action: pan-y;
  user-select: none;
  -webkit-user-select: none;
}

.desk__stage.is-live {
  cursor: none;
}

.desk__stage.is-dragging {
  cursor: grabbing;
}

.screen {
  position: absolute;
  padding: clamp(3px, 0.7%, 6px);
  transition:
    left 0.32s var(--ease),
    top 0.32s var(--ease),
    width 0.32s var(--ease),
    height 0.32s var(--ease);
  touch-action: none;
}

.is-dragging .screen {
  transition: none;
}

.screen__face {
  position: relative;
  height: 100%;
  overflow: hidden;
  border-radius: clamp(5px, 1.3vw, 10px);
  background: linear-gradient(180deg, var(--c-graphite-2), var(--c-graphite));
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.08) inset,
    0 6px 14px -8px rgba(30, 30, 30, 0.5);
  transition:
    background 0.18s var(--ease),
    transform 0.18s var(--ease),
    box-shadow 0.18s var(--ease);
}

.screen.is-active .screen__face {
  background: linear-gradient(180deg, #ff7442, var(--c-signal) 70%);
}

.screen.is-held {
  z-index: 2;
}

.screen.is-held .screen__face {
  transform: scale(1.02);
  box-shadow: 0 18px 30px -16px rgba(30, 30, 30, 0.6);
}

.screen__bar {
  position: absolute;
  inset: 0 0 auto;
  height: 7%;
  min-height: 4px;
  background: rgba(255, 255, 255, 0.1);
}

.screen__name {
  position: absolute;
  left: 9%;
  bottom: 9%;
  color: rgba(255, 255, 255, 0.82);
  font-size: clamp(0.6875rem, 1.3vw, 0.8125rem);
  font-weight: 600;
  letter-spacing: 0.01em;
}

.desk__cursor {
  position: absolute;
  width: clamp(14px, 2.6%, 20px);
  margin: -1px 0 0 -1px;
  pointer-events: none;
  filter: drop-shadow(0 1px 1.5px rgba(0, 0, 0, 0.35));
  z-index: 3;
}

.is-settling .desk__cursor {
  transition:
    left 0.32s var(--ease),
    top 0.32s var(--ease);
}

.is-dragging .desk__cursor {
  opacity: 0;
}

.desk__caption {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 4px 16px;
  margin-top: 14px;
  font-size: var(--t-small);
  color: var(--c-muted);
}

.desk__now b {
  color: var(--c-ink);
  font-weight: 600;
}

</style>

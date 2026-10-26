<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, type Component } from 'vue'
import HeroCopy from './HeroCopy.vue'
import InputScreen from './screens/InputScreen.vue'
import RemoteScreen from './screens/RemoteScreen.vue'
import MicScreen from './screens/MicScreen.vue'
import ClipboardScreen from './screens/ClipboardScreen.vue'
import FilesScreen from './screens/FilesScreen.vue'

/*
 * 整页的主体是一张放大的应用图标：几块屏幕拼成一张桌面，橙色是键鼠所在的那块。
 * 宽屏下镜头随滚动推进：先看到整张桌面，再一块一块推近，光标跨过相接的边，
 * 下一块屏幕亮起，里面是对应功能的演示。窄屏或减弱动态效果时，屏幕依次纵向排开。
 *
 * 屏幕位置以「屏幕宽 / 屏幕高」为单位，相邻屏幕必须有一段相接的边，光标从那里过去。
 */

interface DeskScreen {
  id: string
  label: string
  gx: number
  gy: number
  /** 光标停靠点，相对屏幕的比例坐标 */
  rest: { u: number; v: number }
  component: Component
}

const screens: DeskScreen[] = [
  { id: 'input', label: '键鼠共享', gx: 0, gy: 0, rest: { u: 0.56, v: 0.62 }, component: InputScreen },
  { id: 'remote', label: '远程应用', gx: 1, gy: 0.2, rest: { u: 0.9, v: 0.86 }, component: RemoteScreen },
  { id: 'mic', label: '麦克风共享', gx: 0.6, gy: 1.2, rest: { u: 0.1, v: 0.86 }, component: MicScreen },
  { id: 'clipboard', label: '剪贴板', gx: -0.4, gy: 1, rest: { u: 0.5, v: 0.9 }, component: ClipboardScreen },
  { id: 'files', label: '文件传输', gx: -0.5, gy: 2, rest: { u: 0.9, v: 0.88 }, component: FilesScreen },
]

const NAV = 60
const GAP = 18

const mode = ref<'camera' | 'stack'>('stack')
const vp = reactive({ w: 1280, h: 800 })
const progress = ref(0)
const root = ref<HTMLElement | null>(null)
let rootTop = 0

const size = computed(() => {
  const w = Math.min(1180, vp.w - 240)
  return { w, h: Math.min(vp.h - NAV - 96, Math.round(w * 0.64)) }
})

const rects = computed(() =>
  screens.map((s) => ({ x: s.gx * size.value.w, y: s.gy * size.value.h, w: size.value.w, h: size.value.h })),
)

const bounds = computed(() => {
  const r = rects.value
  const x = Math.min(...r.map((a) => a.x))
  const y = Math.min(...r.map((a) => a.y))
  return { x, y, w: Math.max(...r.map((a) => a.x + a.w)) - x, h: Math.max(...r.map((a) => a.y + a.h)) - y }
})

interface Cam {
  cx: number
  cy: number
  k: number
  fx: number
  fy: number
}

/** 镜头站位：0 是整张桌面（放在首屏右侧），之后每块屏幕一个 */
const cams = computed<Cam[]>(() => {
  const b = bounds.value
  const left = vp.w * 0.47
  const right = vp.w - 56
  const top = NAV + 48
  const bottom = vp.h - 48
  const overview: Cam = {
    cx: b.x + b.w / 2,
    cy: b.y + b.h / 2,
    k: Math.min((right - left) / b.w, (bottom - top) / b.h),
    fx: (left + right) / 2,
    fy: (top + bottom) / 2,
  }
  return [
    overview,
    ...rects.value.map((r) => ({ cx: r.x + r.w / 2, cy: r.y + r.h / 2, k: 1, fx: vp.w / 2, fy: NAV + (vp.h - NAV) / 2 })),
  ]
})

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi)
const lerp = (a: number, b: number, t: number) => a + (b - a) * t
const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2)

/** 每段滚动前 25% 和后 20% 停住，方便阅读和操作 */
const motion = computed(() => {
  const last = cams.value.length - 1
  const seg = Math.min(Math.floor(progress.value), last - 1)
  const t = progress.value - seg
  const u = ease(clamp((t - 0.25) / 0.55, 0, 1))
  return { seg, u }
})

const cam = computed<Cam>(() => {
  const { seg, u } = motion.value
  const a = cams.value[seg]
  const b = cams.value[seg + 1]
  return {
    cx: lerp(a.cx, b.cx, u),
    cy: lerp(a.cy, b.cy, u),
    k: Math.exp(lerp(Math.log(a.k), Math.log(b.k), u)),
    fx: lerp(a.fx, b.fx, u),
    fy: lerp(a.fy, b.fy, u),
  }
})

function restPoint(i: number) {
  const r = rects.value[i]
  const s = screens[i]
  return { x: r.x + r.w * s.rest.u, y: r.y + r.h * s.rest.v }
}

/** 两块相邻屏幕相接那段边的中点 */
function edgeMid(a: (typeof rects.value)[number], b: (typeof rects.value)[number]) {
  const near = (m: number, n: number) => Math.abs(m - n) < 1
  if (near(a.x + a.w, b.x) || near(b.x + b.w, a.x)) {
    const x = near(a.x + a.w, b.x) ? b.x : a.x
    return { x, y: (Math.max(a.y, b.y) + Math.min(a.y + a.h, b.y + b.h)) / 2 }
  }
  const y = near(a.y + a.h, b.y) ? b.y : a.y
  return { x: (Math.max(a.x, b.x) + Math.min(a.x + a.w, b.x + b.w)) / 2, y }
}

/** 光标的世界坐标：停在当前屏幕，换屏时经过相接边的中点 */
const cursor = computed(() => {
  const { seg, u } = motion.value
  if (seg === 0) return restPoint(0)
  const from = restPoint(seg - 1)
  const to = restPoint(seg)
  const mid = edgeMid(rects.value[seg - 1], rects.value[seg])
  return u < 0.5
    ? { x: lerp(from.x, mid.x, u * 2), y: lerp(from.y, mid.y, u * 2) }
    : { x: lerp(mid.x, to.x, u * 2 - 1), y: lerp(mid.y, to.y, u * 2 - 1) }
})

const activeIndex = computed(() => {
  const c = cursor.value
  const hit = rects.value.findIndex((r) => c.x >= r.x - 1 && c.x <= r.x + r.w + 1 && c.y >= r.y - 1 && c.y <= r.y + r.h + 1)
  return hit < 0 ? 0 : hit
})

/** 1 = 还在整张桌面，0 = 已推近到屏幕 */
const overview = computed(() => (motion.value.seg === 0 ? 1 - motion.value.u : 0))
/** 推近时先保持整块橙色，最后一段才露出屏幕内容，避免长时间半透明的粉色 */
const veil = computed(() => clamp(overview.value * 2.5, 0, 1))
const atRest = computed(() => motion.value.u === 0 || motion.value.u === 1)
const interactive = computed(() => atRest.value && overview.value === 0)

const worldStyle = computed(() => {
  const c = cam.value
  return { transform: `translate(${c.fx - c.cx * c.k}px, ${c.fy - c.cy * c.k}px) scale(${c.k})` }
})

/** 停在屏幕里时访客用自己的鼠标操作演示，镜头的光标淡出，只在换屏时出现 */
const cursorStyle = computed(() => {
  const c = cam.value
  const p = cursor.value
  return {
    transform: `translate(${c.fx + (p.x - c.cx) * c.k}px, ${c.fy + (p.y - c.cy) * c.k}px)`,
    opacity: interactive.value ? 0 : 1,
  }
})

const pageLeft = computed(() => (vp.w - Math.min(1160, vp.w - 48)) / 2)

// —— 小地图：图标形状的导航，点一下跳到那块屏幕 ——
const MAP_W = 64
const mapScale = computed(() => MAP_W / bounds.value.w)
const mapRects = computed(() =>
  rects.value.map((r) => ({
    left: `${(r.x - bounds.value.x) * mapScale.value + 1.5}px`,
    top: `${(r.y - bounds.value.y) * mapScale.value + 1.5}px`,
    width: `${r.w * mapScale.value - 3}px`,
    height: `${r.h * mapScale.value - 3}px`,
  })),
)

function goTo(step: number) {
  window.scrollTo({ top: rootTop + step * vp.h, behavior: 'smooth' })
}

// —— 布局与滚动 ——
let raf = 0
let query: MediaQueryList | undefined

function measure() {
  vp.w = window.innerWidth
  vp.h = window.innerHeight
  if (root.value) rootTop = root.value.getBoundingClientRect().top + window.scrollY
  onScroll()
}

function onScroll() {
  if (mode.value !== 'camera' || raf) return
  raf = requestAnimationFrame(() => {
    raf = 0
    progress.value = clamp((window.scrollY - rootTop) / vp.h, 0, cams.value.length - 1)
  })
}

function updateMode() {
  mode.value = query?.matches ? 'camera' : 'stack'
  requestAnimationFrame(measure)
}

onMounted(() => {
  query = window.matchMedia('(min-width: 1100px) and (min-height: 680px) and (prefers-reduced-motion: no-preference)')
  query.addEventListener('change', updateMode)
  window.addEventListener('resize', measure)
  window.addEventListener('scroll', onScroll, { passive: true })
  updateMode()
})

onBeforeUnmount(() => {
  query?.removeEventListener('change', updateMode)
  window.removeEventListener('resize', measure)
  window.removeEventListener('scroll', onScroll)
  cancelAnimationFrame(raf)
})
</script>

<template>
  <section
    v-if="mode === 'camera'"
    id="top"
    ref="root"
    class="tour"
    :style="{ height: `${cams.length * vp.h}px` }"
  >
    <span
      v-for="(s, i) in screens"
      :id="`screen-${s.id}`"
      :key="`mark-${s.id}`"
      class="tour__mark"
      :style="{ top: `${(i + 1) * vp.h}px` }"
    ></span>

    <div class="tour__viewport">
      <div class="tour__world" :class="{ 'is-interactive': interactive }" :style="worldStyle">
        <article
          v-for="(s, i) in screens"
          :key="s.id"
          class="tscreen"
          :class="{ 'is-active': activeIndex === i }"
          :inert="!interactive || activeIndex !== i"
          :aria-label="s.label"
          :style="{
            left: `${rects[i].x}px`,
            top: `${rects[i].y}px`,
            width: `${rects[i].w}px`,
            height: `${rects[i].h}px`,
            padding: `${GAP}px`,
          }"
        >
          <div class="tscreen__face">
            <div class="tscreen__bar" aria-hidden="true">
              <span class="tscreen__apple"></span><b>{{ s.label }}</b>
            </div>
            <div class="tscreen__body">
              <component :is="s.component" :active="interactive && activeIndex === i" />
            </div>
            <div
              class="tscreen__veil"
              :class="{ 'is-hot': activeIndex === i }"
              :style="{ opacity: activeIndex === i ? veil : 1 }"
              aria-hidden="true"
            >
              <span :style="{ fontSize: `${15 / cam.k}px`, opacity: veil }">{{ s.label }}</span>
            </div>
          </div>
        </article>
      </div>

      <svg class="tour__cursor" :style="cursorStyle" viewBox="0 0 20 28" aria-hidden="true">
        <path
          d="M2 2 L2 22 L7.2 17.4 L10.6 25.2 L14 23.8 L10.7 16.2 L17.4 16.2 Z"
          fill="#111"
          stroke="#fff"
          stroke-width="1.6"
          stroke-linejoin="round"
        />
      </svg>

      <div
        class="tour__hero"
        :style="{
          left: `${pageLeft}px`,
          width: `${vp.w * 0.47 - pageLeft - 40}px`,
          opacity: Math.max(0, 1 - (1 - overview) * 2.4),
          transform: `translateY(${(overview - 1) * 60}px)`,
          visibility: overview > 0.6 ? 'visible' : 'hidden',
        }"
      >
        <HeroCopy cue />
      </div>

      <nav
        class="tour__map"
        aria-label="屏幕导航"
        :style="{
          left: `${Math.max(12, (vp.w - size.w) / 2 - MAP_W - 40)}px`,
          opacity: 1 - overview,
          visibility: overview < 0.9 ? 'visible' : 'hidden',
        }"
      >
        <div class="tour__map-desk" :style="{ width: `${MAP_W}px`, height: `${bounds.h * mapScale}px` }">
          <button
            v-for="(s, i) in screens"
            :key="s.id"
            type="button"
            :class="{ 'is-active': activeIndex === i }"
            :style="mapRects[i]"
            :aria-label="s.label"
            :title="s.label"
            :aria-current="activeIndex === i ? 'true' : undefined"
            @click="goTo(i + 1)"
          ></button>
        </div>
      </nav>
    </div>
  </section>

  <section v-else id="top" class="stack">
    <div class="page stack__hero">
      <HeroCopy />
      <div class="stack__icon" :style="{ width: `${MAP_W}px`, height: `${bounds.h * mapScale}px` }" aria-hidden="true">
        <span v-for="(s, i) in screens" :key="s.id" :class="{ 'is-hot': i === 0 }" :style="mapRects[i]"></span>
      </div>
    </div>
    <div class="page stack__list">
      <article v-for="s in screens" :id="`screen-${s.id}`" :key="s.id" class="sscreen" :aria-label="s.label">
        <div class="tscreen__bar" aria-hidden="true">
          <span class="tscreen__apple"></span><b>{{ s.label }}</b>
        </div>
        <div class="sscreen__body">
          <component :is="s.component" :active="true" />
        </div>
      </article>
    </div>
  </section>
</template>

<style scoped>
.tour {
  position: relative;
}

.tour__mark {
  position: absolute;
  left: 0;
  height: 1px;
  scroll-margin-top: 0;
}

.tour__viewport {
  position: sticky;
  top: 0;
  height: 100vh;
  overflow: hidden;
}

.tour__world {
  position: absolute;
  left: 0;
  top: 0;
  transform-origin: 0 0;
  pointer-events: none;
}

.tour__world.is-interactive {
  pointer-events: auto;
}

.tscreen {
  position: absolute;
}

.tscreen__face {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  border-radius: 26px;
  background: var(--c-paper);
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.06),
    0 30px 60px -40px rgba(30, 30, 30, 0.55);
}

.tscreen__bar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 30px;
  padding: 0 18px;
  flex: none;
  background: rgba(255, 255, 255, 0.7);
  box-shadow: 0 1px 0 rgba(0, 0, 0, 0.06);
  font-size: 0.8125rem;
}

.tscreen__bar b {
  font-weight: 600;
}

.tscreen__apple {
  width: 11px;
  height: 8px;
  border-radius: 2px;
  background: var(--c-graphite);
  box-shadow: 4px -3px 0 -1px var(--c-signal);
}

.tscreen__body {
  flex: 1;
  min-height: 0;
  isolation: isolate;
}

.tscreen__veil {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: flex-end;
  padding: 7% 8%;
  background: linear-gradient(180deg, var(--c-graphite-2), var(--c-graphite));
  transition:
    opacity 0.35s var(--ease),
    background 0.35s var(--ease);
  pointer-events: none;
  z-index: 1;
}

.tscreen__veil.is-hot {
  background: linear-gradient(180deg, #ff7442, var(--c-signal) 70%);
}

.tscreen__veil::before {
  content: '';
  position: absolute;
  inset: 0 0 auto;
  height: 7%;
  background: rgba(255, 255, 255, 0.1);
}

.tscreen__veil span {
  color: rgba(255, 255, 255, 0.85);
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;
}

.tour__cursor {
  position: absolute;
  left: 0;
  top: 0;
  width: 22px;
  margin: -2px 0 0 -2px;
  pointer-events: none;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.35));
  transition: opacity 0.25s;
  z-index: 3;
}

.tour__hero {
  position: absolute;
  top: 50%;
  margin-top: 30px;
  translate: 0 -50%;
  z-index: 2;
}

.tour__map {
  position: absolute;
  top: calc(50% + 30px);
  translate: 0 -50%;
  padding: 10px;
  border-radius: 12px;
  background: rgba(248, 247, 244, 0.92);
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.06),
    0 10px 24px -14px rgba(30, 30, 30, 0.5);
  z-index: 2;
  font-size: var(--t-small);
  font-weight: 600;
  transition: opacity 0.2s;
}

.tour__map-desk {
  position: relative;
}

.tour__map button {
  position: absolute;
  padding: 0;
  border: 0;
  border-radius: 2px;
  background: var(--c-graphite);
  cursor: pointer;
  transition: background 0.2s;
}

.tour__map button:hover {
  background: var(--c-graphite-2);
}

.tour__map button.is-active {
  background: var(--c-signal);
}

/* 纵向排列 */
.stack__hero {
  display: grid;
  grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
  align-items: center;
  gap: 48px;
  padding-block: clamp(48px, 8vw, 96px);
}

.stack__icon {
  position: relative;
  justify-self: center;
  transform: scale(3.4);
}

.stack__icon span {
  position: absolute;
  border-radius: 3px;
  background: var(--c-graphite);
}

.stack__icon span.is-hot {
  background: var(--c-signal);
}

.stack__list {
  display: grid;
  gap: 32px;
  padding-bottom: clamp(48px, 8vw, 96px);
}

.sscreen {
  overflow: hidden;
  border-radius: 22px;
  background: var(--c-paper);
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.06),
    0 24px 48px -36px rgba(30, 30, 30, 0.5);
  scroll-margin-top: 76px;
}

@media (max-width: 760px) {
  .stack__hero {
    grid-template-columns: minmax(0, 1fr);
  }

  .stack__icon {
    display: none;
  }
}
</style>

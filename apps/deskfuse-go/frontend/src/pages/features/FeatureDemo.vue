<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

/**
 * 功能关闭页的 SVG 引导动画。
 *
 * 这些演示原先在原生 FeatureDemos.swift 里；MyGo Web UI 迁移时只保留了文案和
 * 图标，动画没有一起迁过来。这里按原来的画面结构和时间轴用 SVG 重建：
 * 两台设备的舞台、统一的 DemoCurve 节奏、鼠标穿越、剪贴板飞行、文件落地、
 * 远程窗口迁移、麦克风声波和鼠标侧键切换桌面。
 */
type DemoPage = 'input' | 'clipboard' | 'files' | 'remoteApplications' | 'microphone' | 'mouse'

const props = defineProps<{ page: DemoPage }>()

const SCREEN = { width: 132, height: 84, gap: 40, right: 172, total: 304, captionY: 102 }
const uid = `feature-demo-${Math.random().toString(36).slice(2, 8)}`

const demoMeta: Record<DemoPage, { period: number; still: number; label: string }> = {
  input: {
    period: 4.2,
    still: 0.72,
    label: '演示：鼠标移到此 Mac 屏幕右侧边缘后，进入另一台电脑的屏幕',
  },
  clipboard: {
    period: 4.6,
    still: 0.8,
    label: '演示：在此 Mac 复制的内容出现在另一台电脑的剪贴板历史中，并可直接粘贴',
  },
  files: {
    period: 5.4,
    still: 0.85,
    label: '演示：手机和电脑浏览器把图片与数据经局域网直接发送到这台电脑',
  },
  remoteApplications: {
    period: 4.6,
    still: 0.8,
    label: '演示：另一台电脑上的应用窗口出现在此 Mac 上，像本地窗口一样使用',
  },
  microphone: {
    period: 2.4,
    still: 0.3,
    label: '演示：另一台 Mac 的麦克风声音实时传到此 Mac',
  },
  mouse: {
    period: 4,
    still: 0.6,
    label: '演示：按下鼠标侧键即可切换到下一个桌面',
  },
}

const progress = ref(0)
const reduceMotion = ref(false)
let raf = 0
let last = 0
let media: MediaQueryList | null = null

const clamp = (value: number) => Math.min(Math.max(value, 0), 1)
const mix = (from: number, to: number, t: number) => from + (to - from) * t
const smooth = (value: number) => {
  const t = clamp(value)
  return t * t * (3 - 2 * t)
}
const easeOut = (value: number) => 1 - (1 - clamp(value)) ** 3
const segment = (value: number, start: number, end: number) => clamp((value - start) / (end - start))
const pulse = (value: number, start: number, end: number) => {
  const t = segment(value, start, end)
  return t > 0 && t < 1 ? Math.sin(Math.PI * t) : 0
}
const fade = (value: number, edge = 0.1) =>
  value < edge ? clamp(value / edge) : value > 1 - edge ? clamp((1 - value) / edge) : 1

function leg(value: number, start: number, end: number, from: { x: number; y: number }, to: { x: number; y: number }) {
  const t = smooth(segment(value, start, end))
  const edge = 0.03
  return {
    x: mix(from.x, to.x, t),
    y: mix(from.y, to.y, t) - 10 * Math.sin(Math.PI * t),
    opacity: fade(value) * segment(value, start, start + edge) * (1 - segment(value, end - edge, end)),
    landed: easeOut(segment(value, end, end + 0.06)) * fade(value),
  }
}

function buildFrame(page: DemoPage, raw: number): any {
  const p = clamp(raw)
  const base = {
    leftLabel: '此 Mac',
    rightLabel: '另一台电脑',
    leftActive: false,
    rightActive: false,
    leftTint: 'var(--accent)',
    rightTint: 'var(--demo-remote)',
  }

  if (page === 'input') {
    const y = SCREEN.height * (0.62 - 0.17 * smooth(p / 0.78))
    let cursorX = 0
    let onRemote = false
    if (p < 0.48) {
      const t = smooth(segment(p, 0.1, 0.48))
      cursorX = mix(SCREEN.width * 0.3, SCREEN.width - 2, t)
    } else {
      const t = easeOut(segment(p, 0.48, 0.78))
      cursorX = mix(SCREEN.width + SCREEN.gap + 2, SCREEN.width + SCREEN.gap + SCREEN.width * 0.55, t)
      onRemote = true
    }
    const glow = p < 0.42 ? 0 : p < 0.48 ? (p - 0.42) / 0.06 : Math.max(0, 1 - (p - 0.48) / 0.22)
    return {
      ...base,
      leftActive: !onRemote,
      rightActive: onRemote,
      cursor: { x: cursorX, y },
      cursorOpacity: fade(p),
      glow,
    }
  }

  if (page === 'clipboard') {
    const historyTop = SCREEN.height * 0.36
    const start = { x: SCREEN.width * 0.42, y: SCREEN.height * 0.5 }
    const end = { x: SCREEN.width + SCREEN.gap + SCREEN.width / 2, y: historyTop }
    const t = smooth(segment(p, 0.24, 0.6))
    const landed = easeOut(segment(p, 0.58, 0.7))
    const chipWidth = mix(56, 92, landed)
    return {
      ...base,
      leftActive: t < 0.5,
      rightActive: t >= 0.5,
      chip: {
        x: mix(start.x, end.x, t),
        y: mix(start.y, end.y, t) - 26 * Math.sin(Math.PI * t),
        width: chipWidth,
        opacity: fade(p) * segment(p, 0.06, 0.14),
      },
      copy: pulse(p, 0.08, 0.24),
      paste: pulse(p, 0.7, 0.86),
      landed,
      historyTop,
    }
  }

  if (page === 'files') {
    const browser = { width: 84, height: 58 }
    const phone = { width: 42, height: 76 }
    const tile = 40
    const tileGap = 8
    const midY = SCREEN.height / 2
    const browserCenter = { x: browser.width / 2, y: midY }
    const phoneCenter = { x: SCREEN.width - phone.width / 2, y: midY }
    const macCenter = SCREEN.width + SCREEN.gap + SCREEN.width / 2
    const tileOffset = (tile + tileGap) / 2
    const tileY = midY + 4
    return {
      ...base,
      leftLabel: '电脑浏览器 · 手机',
      rightLabel: '这台电脑',
      leftActive: (p >= 0.04 && p < 0.38) || (p >= 0.42 && p < 0.76),
      rightActive: p >= 0.2 && p < 0.9,
      rightTint: 'var(--accent)',
      image: leg(p, 0.08, 0.34, phoneCenter, { x: macCenter - tileOffset, y: tileY }),
      data: leg(p, 0.46, 0.72, browserCenter, { x: macCenter + tileOffset, y: tileY }),
      phoneActive: p >= 0.04 && p < 0.38,
      browserActive: p >= 0.42 && p < 0.76,
      receiving: p >= 0.2 && p < 0.9,
    }
  }

  if (page === 'remoteApplications') {
    const t = smooth(segment(p, 0.22, 0.62))
    const opacity = fade(p)
    return {
      ...base,
      leftActive: t >= 0.5,
      rightActive: t < 0.5,
      center: {
        x: mix(SCREEN.width + SCREEN.gap + SCREEN.width / 2, SCREEN.width / 2, t),
        y: SCREEN.height / 2 + 5 - 18 * Math.sin(Math.PI * t),
      },
      lift: pulse(p, 0.14, 0.7),
      scale: 0.85 + 0.25 * easeOut(segment(p, 0.6, 0.74)),
      ghost: segment(p, 0.24, 0.4) * fade(p),
      opacity,
    }
  }

  if (page === 'microphone') {
    return {
      ...base,
      leftLabel: '另一台 Mac',
      rightLabel: '此 Mac',
      leftActive: true,
      rightActive: true,
      leftTint: 'var(--demo-remote)',
      rightTint: 'var(--accent)',
      waves: [0, 1, 2].map((index) => (p * 2 + index / 3) % 1),
      levels: [0, 1, 2, 3, 4].map((index) => 0.25 + 0.75 * Math.abs(Math.sin(2 * Math.PI * (p * 2 + index * 0.19)))),
      ring: (p * 2) % 1,
    }
  }

  return {
    ...base,
    leftLabel: '侧键',
    rightLabel: '此 Mac',
    leftActive: pulse(p, 0.1, 0.3) > 0.3,
    rightActive: true,
    rightTint: 'var(--accent)',
    press: pulse(p, 0.1, 0.3),
    chip: easeOut(segment(p, 0.2, 0.32)) * (1 - segment(p, 0.78, 0.9)),
    desktop: smooth(segment(p, 0.3, 0.58)) * (1 - smooth(segment(p, 0.86, 1))),
  }
}

const frame = computed(() => buildFrame(props.page, progress.value))
const meta = computed(() => demoMeta[props.page])
const screens = computed(() => {
  const f = frame.value
  if (props.page === 'files' || props.page === 'mouse') {
    return [{ id: 'right', x: SCREEN.right, active: props.page === 'files' ? f.receiving : true, tint: 'var(--accent)' }]
  }
  return [
    { id: 'left', x: 0, active: f.leftActive, tint: f.leftTint },
    { id: 'right', x: SCREEN.right, active: f.rightActive, tint: f.rightTint },
  ]
})
const captions = computed(() => [
  { x: 66, label: frame.value.leftLabel, active: frame.value.leftActive },
  { x: 238, label: frame.value.rightLabel, active: frame.value.rightActive },
])
const clip = (side: string) => `url(#${uid}-${side})`

function tick(now: number) {
  if (!document.hidden) {
    if (!last) last = now
    const delta = Math.min(0.05, (now - last) / 1000)
    last = now
    if (reduceMotion.value) progress.value = meta.value.still
    else progress.value = (progress.value + delta / meta.value.period) % 1
  } else {
    last = 0
  }
  raf = requestAnimationFrame(tick)
}

function onMotionChange(event: MediaQueryListEvent) {
  reduceMotion.value = event.matches
  if (event.matches) progress.value = meta.value.still
  last = 0
}

onMounted(() => {
  media = window.matchMedia('(prefers-reduced-motion: reduce)')
  reduceMotion.value = media.matches
  if (reduceMotion.value) progress.value = meta.value.still
  media.addEventListener('change', onMotionChange)
  raf = requestAnimationFrame(tick)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  media?.removeEventListener('change', onMotionChange)
})

watch(() => props.page, () => {
  progress.value = reduceMotion.value ? meta.value.still : 0
  last = 0
})
</script>

<template>
  <svg
    class="feature-demo"
    viewBox="0 0 304 112"
    role="img"
    :aria-label="meta.label"
    preserveAspectRatio="xMidYMid meet"
  >
    <defs>
      <clipPath v-for="side in ['left', 'right']" :id="`${uid}-${side}`" :key="side">
        <rect width="132" height="84" rx="8" />
      </clipPath>
      <filter :id="`${uid}-shadow`" x="-40%" y="-40%" width="180%" height="180%">
        <feDropShadow dx="0" dy="1" stdDeviation="2" flood-opacity="0.18" />
      </filter>
    </defs>

    <g v-for="side in screens" :key="side.id" :transform="`translate(${side.x} 0)`" class="screen-group">
      <rect class="screen" :class="{ active: side.active }" :style="{ '--tint': side.tint }" width="132" height="84" rx="8" />
      <g :clip-path="clip(side.id)">
        <rect :style="{ fill: side.tint, opacity: side.active ? 0.9 : 0.22 }" width="132" height="8" />

        <template v-if="page === 'input' || page === 'clipboard'">
          <template v-if="side.id === 'left'">
            <rect v-for="(line, index) in [92, 68, 56]" :key="line" x="20" :y="28 + index * 11" :width="line" height="5" rx="2.5" fill="var(--control)" />
          </template>
          <template v-else>
            <template v-if="page === 'input'">
              <rect v-for="(line, index) in [72, 92, 48]" :key="line" x="20" :y="28 + index * 11" :width="line" height="5" rx="2.5" fill="var(--control)" />
            </template>
            <template v-else>
              <rect
                v-for="row in 3"
                :key="row"
                x="20"
                :y="frame.historyTop + (row - 1) * 16 + 16 * frame.landed"
                width="92"
                height="12"
                rx="2"
                fill="var(--control)"
              />
            </template>
          </template>
        </template>

        <template v-else-if="page === 'remoteApplications'">
          <rect
            v-if="side.id === 'right'"
            x="35"
            y="26"
            width="62"
            height="42"
            rx="4"
            fill="none"
            stroke="var(--line)"
            stroke-dasharray="3 3"
            :opacity="frame.ghost"
          />
        </template>

        <template v-else-if="page === 'microphone'">
          <template v-if="side.id === 'left'">
            <circle cx="66" cy="42" :r="14 + 13 * frame.ring" fill="none" stroke="var(--demo-remote)" stroke-width="1.5" :opacity="0.5 * (1 - frame.ring)" />
            <rect x="62" y="31" width="8" height="14" rx="4" fill="none" stroke="var(--demo-remote)" stroke-width="1.5" />
            <path d="M60 41c0 6 12 6 12 0M66 48v5M62 53h8" fill="none" stroke="var(--demo-remote)" stroke-width="1.2" stroke-linecap="round" />
          </template>
          <template v-else>
            <rect
              v-for="(level, index) in frame.levels"
              :key="index"
              :x="46 + index * 9"
              :y="62 - (6 + 30 * level)"
              width="5"
              :height="6 + 30 * level"
              rx="2.5"
              fill="var(--accent)"
            />
          </template>
        </template>

        <template v-else-if="page === 'files'">
          <g v-for="(item, index) in [
            { key: 'image', x: 42, landed: frame.image.landed },
            { key: 'data', x: 90, landed: frame.data.landed },
          ]" :key="item.key" :transform="`translate(${item.x} 46) scale(${0.7 + 0.3 * item.landed})`" :opacity="item.landed">
            <rect x="-20" y="-20" width="40" height="40" rx="4" fill="var(--control)" />
            <g v-if="item.key === 'image'" :opacity="0.85">
              <rect x="-11" y="-11" width="22" height="18" rx="2" fill="none" stroke="var(--accent)" stroke-width="1.2" />
              <path d="m-9 4 6-6 4 4 3-3 4 5" fill="none" stroke="var(--accent)" stroke-width="1.2" stroke-linejoin="round" />
              <circle cx="-4" cy="-5" r="2" fill="var(--accent)" />
            </g>
            <g v-else :opacity="0.85">
              <path d="M-9-12h12l6 6v18H-9z" fill="none" stroke="var(--accent)" stroke-width="1.2" stroke-linejoin="round" />
              <path d="M3-12v6h6M-5-2H5M-5 2H5M-5 6H2" fill="none" stroke="var(--accent)" stroke-width="1.1" stroke-linecap="round" />
            </g>
            <g transform="translate(14 -14)">
              <circle r="5" fill="var(--success)" />
              <path d="m-2 0 1.5 2L3-3" fill="none" stroke="white" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" />
            </g>
          </g>
        </template>

        <template v-else-if="page === 'mouse'">
          <g :transform="`translate(${-132 * frame.desktop} 8)`">
            <g>
              <rect x="20" y="18" width="54" height="34" rx="3" fill="var(--accent)" opacity="0.18" />
              <rect x="24" y="22" width="46" height="26" rx="2" fill="none" stroke="var(--accent)" opacity="0.45" />
              <rect x="84" y="12" width="40" height="26" rx="3" fill="var(--demo-remote)" opacity="0.18" />
              <rect x="88" y="16" width="32" height="18" rx="2" fill="none" stroke="var(--demo-remote)" opacity="0.45" />
            </g>
            <g transform="translate(132 0)">
              <rect x="24" y="14" width="70" height="40" rx="3" fill="var(--accent)" opacity="0.18" />
              <rect x="28" y="18" width="62" height="32" rx="2" fill="none" stroke="var(--accent)" opacity="0.45" />
            </g>
          </g>
        </template>
      </g>
    </g>

    <template v-if="page === 'files'">
      <g transform="translate(0 13)">
        <rect width="84" height="58" rx="4" fill="var(--surface)" :style="{ stroke: frame.browserActive ? 'var(--accent)' : 'var(--line)' }" stroke-width="1" />
        <circle cx="8" cy="8" r="1.6" fill="var(--demo-remote)" :opacity="frame.browserActive ? 0.9 : 0.25" />
        <circle cx="14" cy="8" r="1.6" fill="var(--demo-remote)" :opacity="frame.browserActive ? 0.9 : 0.25" />
        <circle cx="20" cy="8" r="1.6" fill="var(--demo-remote)" :opacity="frame.browserActive ? 0.9 : 0.25" />
        <rect x="27" y="5" width="30" height="4" rx="2" fill="var(--demo-remote)" :opacity="frame.browserActive ? 0.9 : 0.25" />
        <g transform="translate(42 34)" :style="{ stroke: frame.browserActive ? 'var(--accent)' : 'var(--secondary)' }" fill="none" stroke-width="1.2">
          <path d="M-7-9h9l4 4v14H-7z" />
          <path d="M2-9v4h4M-4 0h6M-4 4h6" />
        </g>
      </g>
      <g transform="translate(90 4)">
        <rect width="42" height="76" rx="5" fill="var(--surface)" :style="{ stroke: frame.phoneActive ? 'var(--accent)' : 'var(--line)' }" stroke-width="1" />
        <rect x="15" y="5" width="12" height="3" rx="1.5" fill="var(--demo-remote)" :opacity="frame.phoneActive ? 0.9 : 0.25" />
        <g transform="translate(21 38)" :style="{ stroke: frame.phoneActive ? 'var(--accent)' : 'var(--secondary)' }" fill="none" stroke-width="1.2">
          <rect x="-8" y="-8" width="16" height="16" rx="2" />
          <path d="m-6 3 4-4 3 3 2-2 3 4" />
          <circle cx="-2" cy="-3" r="1.5" />
        </g>
      </g>
      <path d="M136 42h32" fill="none" stroke="var(--accent)" stroke-width="1.5" stroke-linecap="round" stroke-dasharray="2 4" opacity="0.45" />
      <g v-if="frame.image.opacity > 0" :transform="`translate(${frame.image.x} ${frame.image.y})`" :opacity="frame.image.opacity">
        <rect x="-7" y="-7" width="14" height="14" rx="2" fill="var(--accent)" />
        <path d="m-5 3 4-4 3 3 2-2 2 3" fill="none" stroke="white" stroke-width="1" />
        <circle cx="-2" cy="-3" r="1.2" fill="white" />
      </g>
      <g v-if="frame.data.opacity > 0" :transform="`translate(${frame.data.x} ${frame.data.y})`" :opacity="frame.data.opacity">
        <rect x="-7" y="-8" width="14" height="16" rx="2" fill="var(--accent)" />
        <path d="M-3-5h6M-3-1h6M-3 3h4" stroke="white" stroke-width="1" stroke-linecap="round" />
      </g>
    </template>

    <template v-if="page === 'mouse'">
      <g transform="translate(43 6)">
        <rect width="46" height="72" rx="23" fill="var(--surface)" stroke="var(--line)" stroke-width="1" />
        <path d="M8 30h30" stroke="var(--line)" stroke-width="1" />
        <rect x="21" y="11" width="4" height="10" rx="2" fill="var(--secondary)" opacity="0.5" />
        <rect
          x="2"
          y="29"
          width="5"
          height="16"
          rx="2.5"
          fill="var(--accent)"
          :opacity="0.25 + 0.75 * frame.press"
          :transform="`translate(${1.5 * frame.press} 0)`"
        />
      </g>
      <g :transform="`translate(210 62)`" :opacity="frame.chip">
        <rect x="0" y="0" width="56" height="16" rx="8" fill="var(--surface)" stroke="var(--line)" stroke-width="1" />
        <text x="28" y="11" text-anchor="middle" class="chip-text">→ 切换桌面</text>
      </g>
    </template>

    <template v-if="page === 'input'">
      <rect x="128" y="12" width="3" height="60" rx="1.5" fill="var(--accent)" :opacity="frame.glow" />
      <rect x="171" y="12" width="3" height="60" rx="1.5" fill="var(--accent)" :opacity="frame.glow" />
      <g class="cursor" :transform="`translate(${frame.cursor.x} ${frame.cursor.y})`" :opacity="frame.cursorOpacity">
        <path d="M0 0v13l4-3.6 3 6.8 2.8-1.3-3-6.6h5.4z" fill="var(--text)" stroke="var(--surface)" stroke-width="0.8" stroke-linejoin="round" />
      </g>
    </template>

    <template v-if="page === 'clipboard'">
      <g :transform="`translate(${frame.chip.x} ${frame.chip.y})`" :opacity="frame.chip.opacity">
        <rect :x="-frame.chip.width / 2" y="-6" :width="frame.chip.width" height="12" rx="2" fill="var(--accent)" opacity="0.16" stroke="var(--accent)" stroke-width="1" />
      </g>
      <g :transform="`translate(110 68) scale(${1 + 0.12 * frame.copy})`" :opacity="frame.landed < 1 ? 1 : 0.4">
        <rect x="-16" y="-8" width="32" height="16" rx="3" fill="var(--control)" stroke="var(--accent)" :stroke-opacity="frame.copy" />
        <text y="3" text-anchor="middle" class="keycap-text" :class="{ pressed: frame.copy > 0.5 }">⌘C</text>
      </g>
      <g :transform="`translate(282 68) scale(${1 + 0.12 * frame.paste})`" :opacity="frame.landed > 0 ? 1 : 0.4">
        <rect x="-16" y="-8" width="32" height="16" rx="3" fill="var(--control)" stroke="var(--accent)" :stroke-opacity="frame.paste" />
        <text y="3" text-anchor="middle" class="keycap-text" :class="{ pressed: frame.paste > 0.5 }">⌘V</text>
      </g>
    </template>

    <template v-if="page === 'remoteApplications'">
      <g
        :transform="`translate(${frame.center.x} ${frame.center.y}) scale(${frame.scale}) translate(-31 -21)`"
        :opacity="frame.opacity"
        :filter="`url(#${uid}-shadow)`"
      >
        <rect width="62" height="42" rx="4" fill="var(--surface)" stroke="var(--line)" stroke-width="1" />
        <circle cx="7" cy="7" r="1.7" fill="#ff5f57" />
        <circle cx="12" cy="7" r="1.7" fill="#febc2e" />
        <circle cx="17" cy="7" r="1.7" fill="#28c840" />
        <rect x="10" y="17" width="42" height="4" rx="2" fill="var(--control)" />
        <rect x="10" y="25" width="30" height="4" rx="2" fill="var(--control)" />
      </g>
    </template>

    <template v-if="page === 'microphone'">
      <circle
        v-for="(wave, index) in frame.waves"
        :key="index"
        :cx="mix(95, 205, wave)"
        cy="46"
        r="2.5"
        fill="var(--accent)"
        :opacity="Math.sin(Math.PI * wave)"
      />
    </template>

    <text
      v-for="caption in captions"
      :key="caption.x"
      :x="caption.x"
      :y="SCREEN.captionY"
      text-anchor="middle"
      class="caption"
      :class="{ active: caption.active }"
    >
      {{ caption.label }}
    </text>
  </svg>
</template>

<style scoped>
.feature-demo {
  display: block;
  width: min(304px, 100%);
  height: auto;
  overflow: visible;
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", sans-serif;
  --demo-remote: #af52de;
}

@media (prefers-color-scheme: dark) {
  .feature-demo {
    --demo-remote: #bf5af2;
  }
}

.screen {
  fill: var(--surface);
  stroke: var(--line);
  stroke-width: 1;
}

.screen.active {
  stroke: var(--tint);
}

.caption {
  fill: var(--secondary);
  font-size: 9px;
  font-weight: 400;
}

.caption.active {
  fill: var(--text);
  font-weight: 600;
}

.keycap-text {
  fill: var(--secondary);
  font-size: 7px;
  font-weight: 600;
}

.keycap-text.pressed {
  fill: var(--accent);
}

.chip-text {
  fill: var(--accent);
  font-size: 7px;
  font-weight: 600;
}
</style>

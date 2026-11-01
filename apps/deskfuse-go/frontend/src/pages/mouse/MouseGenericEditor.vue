<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { actionLabel, buttonName } from './model'

const props = defineProps<{
  bindings: any[]
  modifiers: number
  recording: number | null
  activeMenuButton?: number | null
  saveStatus: string
  lastExecution: string
  hidStatus: string
  modelName: string | null
  deviceName: string | null
  extras: number[]
  loaded: boolean
}>()

const emit = defineEmits<{
  menu: [button: number, event: MouseEvent]
  record: [button: number]
  cancelRecord: []
}>()

interface SpotSpec {
  key: string
  button: number
  codeNum: string
  column: 'left' | 'right'
  title: string
  detail: string
  x: number
  y: number
}

interface LeaderLine {
  key: string
  d: string
  sx: number
  sy: number
  configured: boolean
}

// 左列 y: 74 -> 139 -> 179；右列 y: 58 -> 84，纵坐标严格单调，折线零交叉。
const SPOTS: SpotSpec[] = [
  { key: 'b0', button: 0, codeNum: '01', column: 'left', title: '左键', detail: '食指主键', x: 118, y: 74 },
  { key: 'b4', button: 4, codeNum: '02', column: 'left', title: '前进侧键', detail: '按钮 5 · 左侧靠前', x: 75, y: 139 },
  { key: 'b3', button: 3, codeNum: '03', column: 'left', title: '后退侧键', detail: '按钮 4 · 左侧靠后', x: 75, y: 179 },
  { key: 'b2', button: 2, codeNum: '04', column: 'right', title: '中键 / 滚轮按下', detail: '按下滚轮', x: 160, y: 58 },
  { key: 'b1', button: 1, codeNum: '05', column: 'right', title: '右键', detail: '中指副键', x: 202, y: 84 },
]

const hoveredKey = ref<string | null>(null)
const canvasEl = ref<HTMLElement | null>(null)
const lines = ref<LeaderLine[]>([])

let observer: ResizeObserver | null = null
let rafId = 0

function effectiveModifiers(button: number): number {
  return button >= 195 ? 0 : props.modifiers
}

function bindingFor(button: number): any {
  const mods = effectiveModifiers(button)
  return props.bindings.find(
    (entry) => Number(entry.button) === button && Number(entry.modifiers) === mods,
  )?.action ?? null
}

function configured(button: number): boolean {
  return Boolean(bindingFor(button))
}

function labelFor(button: number): string {
  return actionLabel(bindingFor(button), effectiveModifiers(button))
}

function shortExtraName(button: number): string {
  return buttonName(button).replace(/（.*?）/g, '')
}

const layoutBadge = computed(() => {
  if (props.modelName) return `${props.modelName} · 标准五键`
  if (props.deviceName) return '未适配 · 标准五键布局'
  return '离线预配置 · 标准五键'
})

const boundaryHint = computed(() => {
  if (props.deviceName && !props.modelName) {
    return '未在适配列表，仅映射标准五键；其余按键保持鼠标原有功能'
  }
  if (!props.modelName && !props.deviceName) {
    return '未检测到鼠标，当前设置将在连接鼠标后自动生效'
  }
  return '标准五键之外，鼠标原有功能保持不变'
})

const leftSpots = computed(() => SPOTS.filter((spot) => spot.column === 'left'))
const rightSpots = computed(() => SPOTS.filter((spot) => spot.column === 'right'))

function isHighlighted(spot: SpotSpec): boolean {
  if (hoveredKey.value === spot.key) return true
  if (props.recording !== null && props.recording === spot.button) return true
  if (props.activeMenuButton !== null && props.activeMenuButton !== undefined && props.activeMenuButton === spot.button) return true
  return false
}

function isPrimaryLeftRebound(button: number): boolean {
  if (button !== 0) return false
  const action = bindingFor(0)
  return Boolean(action && !(action.kind === 'mouse' && Number(action.button) === 0))
}

function openButtonMenu(button: number, event: MouseEvent) {
  if (!props.loaded || props.recording !== null) return
  emit('menu', button, event)
}

function onSpotClick(spot: SpotSpec, event: MouseEvent) {
  const cardBtn = canvasEl.value?.querySelector<HTMLElement>(`[data-callout-key="${spot.key}"] .callout-action`)
  if (cardBtn) {
    cardBtn.focus({ preventScroll: true })
    openButtonMenu(spot.button, { currentTarget: cardBtn } as unknown as MouseEvent)
    return
  }
  openButtonMenu(spot.button, event)
}

function toggleButtonRecord(button: number) {
  if (!props.loaded) return
  if (props.recording === button) {
    emit('cancelRecord')
    return
  }
  emit('record', button)
}

function measureLines() {
  const root = canvasEl.value
  if (!root) return
  const rootRect = root.getBoundingClientRect()
  if (rootRect.width <= 660) {
    lines.value = []
    return
  }
  const next: LeaderLine[] = []
  for (const spot of SPOTS) {
    const card = root.querySelector<HTMLElement>(`[data-callout-key="${spot.key}"]`)
    const pin = root.querySelector<HTMLElement>(`[data-spot-key="${spot.key}"]`)
    if (!card || !pin) continue
    const cRect = card.getBoundingClientRect()
    const pRect = pin.getBoundingClientRect()
    if (cRect.width === 0 || pRect.width === 0) continue
    const isLeft = spot.column === 'left'
    const sx = (isLeft ? cRect.right : cRect.left) - rootRect.left
    const sy = cRect.top + cRect.height / 2 - rootRect.top
    const ex = pRect.left + pRect.width / 2 - rootRect.left
    const ey = pRect.top + pRect.height / 2 - rootRect.top
    const kx = sx + (ex - sx) * 0.44
    next.push({
      key: spot.key,
      d: `M${sx.toFixed(1)},${sy.toFixed(1)} L${kx.toFixed(1)},${sy.toFixed(1)} L${ex.toFixed(1)},${ey.toFixed(1)}`,
      sx: Number(sx.toFixed(1)),
      sy: Number(sy.toFixed(1)),
      configured: configured(spot.button),
    })
  }
  lines.value = next
}

function scheduleLines() {
  if (rafId) cancelAnimationFrame(rafId)
  rafId = requestAnimationFrame(() => {
    rafId = 0
    measureLines()
  })
}

watch(
  () => [props.bindings, props.modifiers, props.extras] as const,
  async () => {
    await nextTick()
    scheduleLines()
  },
  { deep: true },
)

onMounted(() => {
  scheduleLines()
  observer = new ResizeObserver(scheduleLines)
  if (canvasEl.value) observer.observe(canvasEl.value)
  window.addEventListener('resize', scheduleLines)
})

onBeforeUnmount(() => {
  if (rafId) cancelAnimationFrame(rafId)
  observer?.disconnect()
  window.removeEventListener('resize', scheduleLines)
})
</script>

<template>
  <section class="callout-sheet">
    <header class="sheet-head">
      <div class="head-left">
        <slot name="device" />
      </div>
      <span class="layout-tag">{{ layoutBadge }}</span>
    </header>

    <div ref="canvasEl" class="callout-canvas">
      <svg class="leader-svg" aria-hidden="true">
        <g v-for="line in lines" :key="line.key">
          <path :d="line.d" class="leader-halo" />
          <path
            :d="line.d"
            class="leader-path"
            :class="{
              'is-configured': line.configured,
              'is-highlighted': hoveredKey === line.key,
            }"
          />
          <circle
            :cx="line.sx"
            :cy="line.sy"
            r="2.4"
            class="leader-origin"
            :class="{
              'is-configured': line.configured,
              'is-highlighted': hoveredKey === line.key,
            }"
          />
        </g>
      </svg>

      <!-- 左侧标注列 -->
      <div class="callout-col is-left">
        <article
          v-for="spot in leftSpots"
          :key="spot.key"
          class="callout-card"
          :class="{
            'is-configured': configured(spot.button),
            'is-highlighted': isHighlighted(spot),
            'is-recording': recording === spot.button,
          }"
          :data-callout-key="spot.key"
          @mouseenter="hoveredKey = spot.key"
          @mouseleave="hoveredKey = null"
          @focusin="hoveredKey = spot.key"
          @focusout="hoveredKey = null"
        >
          <div class="callout-top">
            <div class="callout-titles">
              <strong>{{ spot.title }}</strong>
              <small>{{ spot.detail }}</small>
            </div>
            <span class="callout-index">{{ spot.codeNum }}</span>
          </div>

          <div class="callout-controls">
            <button
              type="button"
              class="callout-action"
              :class="{ 'is-recording': recording === spot.button }"
              :disabled="!loaded || recording !== null"
              @click="openButtonMenu(spot.button, $event)"
            >
              <span class="action-text">{{ recording === spot.button ? '请按键盘快捷键…' : labelFor(spot.button) }}</span>
              <span class="action-chev" aria-hidden="true">⌄</span>
            </button>
            <button
              type="button"
              class="record-btn"
              :class="{ 'is-recording': recording === spot.button }"
              :disabled="!loaded || (recording !== null && recording !== spot.button)"
              :title="recording === spot.button ? '按 Esc 或点击取消录制' : '直接录制键盘快捷键'"
              @click="toggleButtonRecord(spot.button)"
            >
              {{ recording === spot.button ? '取消' : '录制' }}
            </button>
          </div>
          <span v-if="isPrimaryLeftRebound(spot.button)" class="sub-warn">左键已改绑，请保留触控板或辅助点击方式</span>
        </article>
      </div>

      <!-- 中央标准五键线稿示意图 -->
      <div class="schem-center">
        <div class="schem-frame">
          <svg class="schematic" viewBox="0 0 320 300" role="img" aria-label="标准五键鼠标示意图">
            <defs>
              <linearGradient id="ge-body" x1="0" y1="0" x2="0.4" y2="1">
                <stop class="ge-stop-a" offset="0" />
                <stop class="ge-stop-b" offset="1" />
              </linearGradient>
              <linearGradient id="ge-wheel" x1="0" y1="0" x2="0" y2="1">
                <stop class="ge-wheel-a" offset="0" />
                <stop class="ge-wheel-b" offset="1" />
              </linearGradient>
            </defs>

            <rect class="tab" :class="{ on: configured(4) }" x="66" y="122" width="18" height="34" rx="7" />
            <rect class="tab" :class="{ on: configured(3) }" x="66" y="162" width="18" height="34" rx="7" />

            <path
              class="body"
              d="M160 24 C202 24 240 52 240 96 L240 206 C240 254 214 276 160 276 C106 276 80 254 80 206 L80 96 C80 52 118 24 160 24 Z"
            />
            <path class="split" d="M160 26 L160 98" />
            <rect class="wheel" :class="{ on: configured(2) }" x="147" y="42" width="26" height="52" rx="10" />
          </svg>

          <button
            v-for="spot in SPOTS"
            :key="`hot-${spot.key}`"
            type="button"
            class="hotspot"
            :class="{
              configured: configured(spot.button),
              selected: isHighlighted(spot),
              recording: recording === spot.button,
            }"
            :data-spot-key="spot.key"
            :style="{ left: `${(spot.x / 320) * 100}%`, top: `${(spot.y / 300) * 100}%` }"
            :title="`${spot.title} · ${labelFor(spot.button)}`"
            :aria-label="`${buttonName(spot.button)}，当前 ${labelFor(spot.button)}`"
            @mouseenter="hoveredKey = spot.key"
            @mouseleave="hoveredKey = null"
            @focus="hoveredKey = spot.key"
            @blur="hoveredKey = null"
            @click.stop="onSpotClick(spot, $event)"
          >
            <span class="spot-num">{{ spot.codeNum }}</span>
          </button>
        </div>
      </div>

      <!-- 右侧标注列 -->
      <div class="callout-col is-right">
        <article
          v-for="spot in rightSpots"
          :key="spot.key"
          class="callout-card"
          :class="{
            'is-configured': configured(spot.button),
            'is-highlighted': isHighlighted(spot),
            'is-recording': recording === spot.button,
          }"
          :data-callout-key="spot.key"
          @mouseenter="hoveredKey = spot.key"
          @mouseleave="hoveredKey = null"
          @focusin="hoveredKey = spot.key"
          @focusout="hoveredKey = null"
        >
          <div class="callout-top">
            <div class="callout-titles">
              <strong>{{ spot.title }}</strong>
              <small>{{ spot.detail }}</small>
            </div>
            <span class="callout-index">{{ spot.codeNum }}</span>
          </div>

          <div class="callout-controls">
            <button
              type="button"
              class="callout-action"
              :class="{ 'is-recording': recording === spot.button }"
              :disabled="!loaded || recording !== null"
              @click="openButtonMenu(spot.button, $event)"
            >
              <span class="action-text">{{ recording === spot.button ? '请按键盘快捷键…' : labelFor(spot.button) }}</span>
              <span class="action-chev" aria-hidden="true">⌄</span>
            </button>
            <button
              type="button"
              class="record-btn"
              :class="{ 'is-recording': recording === spot.button }"
              :disabled="!loaded || (recording !== null && recording !== spot.button)"
              :title="recording === spot.button ? '按 Esc 或点击取消录制' : '直接录制键盘快捷键'"
              @click="toggleButtonRecord(spot.button)"
            >
              {{ recording === spot.button ? '取消' : '录制' }}
            </button>
          </div>
        </article>
      </div>
    </div>

    <!-- 无固定示意图坐标的扩展键（如 MX Anywhere / Ergo / Vertical）就地陈列，不伪造坐标 -->
    <div v-if="extras.length" class="extras-strip">
      <span class="extras-title">机身扩展键</span>
      <div class="extras-list">
        <div
          v-for="extraBtn in extras"
          :key="extraBtn"
          class="extra-card"
          :class="{
            'is-configured': configured(extraBtn),
            'is-recording': recording === extraBtn,
          }"
        >
          <span class="extra-name">{{ shortExtraName(extraBtn) }}</span>
          <button
            type="button"
            class="callout-action"
            :class="{ 'is-recording': recording === extraBtn }"
            :disabled="!loaded || recording !== null"
            @click="openButtonMenu(extraBtn, $event)"
          >
            <span class="action-text">{{ recording === extraBtn ? '请按键盘快捷键…' : labelFor(extraBtn) }}</span>
            <span class="action-chev" aria-hidden="true">⌄</span>
          </button>
          <button
            type="button"
            class="record-btn"
            :class="{ 'is-recording': recording === extraBtn }"
            :disabled="!loaded || (recording !== null && recording !== extraBtn)"
            @click="toggleButtonRecord(extraBtn)"
          >
            {{ recording === extraBtn ? '取消' : '录制' }}
          </button>
        </div>
      </div>
    </div>

    <footer class="sheet-foot">
      <div class="foot-left">
        <slot name="modifiers" />
        <span class="foot-boundary">{{ boundaryHint }}</span>
      </div>
      <div class="foot-right">
        <slot name="foot-actions" />
      </div>
    </footer>
  </section>
</template>

<style scoped>
.callout-sheet {
  container-type: inline-size;
  display: flex;
  flex-direction: column;
  background: var(--surface);
  border-radius: var(--radius-panel, 16px);
  box-shadow: 0 0 0 1px var(--line), 0 2px 6px rgba(0, 0, 0, .035);
  overflow: hidden;
}

.sheet-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 13px 18px;
  border-bottom: 0.5px solid var(--line);
  background: var(--surface);
  flex-wrap: wrap;
}

.head-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex-wrap: wrap;
}

.layout-tag {
  flex: none;
  padding: 4px 11px;
  border-radius: 999px;
  font-size: 11.5px;
  font-weight: 500;
  color: var(--secondary);
  background: var(--control);
}

.callout-canvas {
  position: relative;
  isolation: isolate;
  min-height: 368px;
  display: grid;
  grid-template-columns: minmax(196px, 260px) minmax(168px, 1fr) minmax(196px, 260px);
  align-items: center;
  gap: 12px;
  padding: 20px 18px;
  background-color: var(--surface);
  background-image:
    radial-gradient(ellipse 44% 58% at 50% 50%, var(--surface) 38%, transparent 82%),
    radial-gradient(color-mix(in srgb, var(--text, #222) 8%, transparent) 0.85px, transparent 0.85px);
  background-size: 100% 100%, 16px 16px;
}

.leader-svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: 2;
}

.leader-halo {
  fill: none;
  stroke: color-mix(in srgb, var(--surface) 64%, transparent);
  stroke-width: 3.2;
  stroke-linejoin: round;
}

.leader-path {
  fill: none;
  stroke: color-mix(in srgb, var(--text, #222) 26%, var(--surface));
  stroke-width: 1.3;
  stroke-linejoin: round;
  transition: stroke var(--dur-fast, 120ms) var(--ease-out, ease), stroke-width var(--dur-fast, 120ms) var(--ease-out, ease);
}

.leader-path.is-configured {
  stroke: color-mix(in srgb, var(--accent) 72%, var(--surface));
}

.leader-path.is-highlighted {
  stroke: var(--accent);
  stroke-width: 1.9;
}

.leader-origin {
  fill: color-mix(in srgb, var(--text, #222) 28%, var(--surface));
  transition: fill var(--dur-fast, 120ms) var(--ease-out, ease);
}

.leader-origin.is-configured,
.leader-origin.is-highlighted {
  fill: var(--accent);
}

.callout-col {
  position: relative;
  z-index: 3;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 12px;
  min-width: 0;
}

.callout-card {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 10px 12px;
  border-radius: 11px;
  background: color-mix(in srgb, var(--surface) 96%, var(--canvas));
  box-shadow: 0 0 0 1px var(--line), 0 2px 5px rgba(0, 0, 0, .035);
  transition: box-shadow var(--dur-fast, 120ms) var(--ease-out, ease), background-color var(--dur-fast, 120ms) var(--ease-out, ease), transform var(--dur-fast, 120ms) var(--ease-out, ease);
}

.callout-card.is-configured {
  background: color-mix(in srgb, var(--accent) 3.5%, var(--surface));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 28%, var(--line)), 0 2px 6px rgba(0, 0, 0, .04);
}

.callout-card.is-highlighted {
  box-shadow: 0 0 0 1px var(--accent), 0 0 0 4px color-mix(in srgb, var(--accent) 12%, transparent);
}

.callout-card.is-recording {
  box-shadow: 0 0 0 1px var(--warning, #ed8b23), 0 0 0 4px color-mix(in srgb, var(--warning, #ed8b23) 14%, transparent);
}

@media (hover: hover) and (pointer: fine) {
  .callout-card:hover {
    transform: translateY(-1px);
  }
}

.callout-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}

.callout-titles {
  min-width: 0;
  display: grid;
  gap: 1px;
}

.callout-titles strong {
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
  color: var(--text);
}

.callout-titles small {
  font-size: 11px;
  line-height: 1.35;
  color: var(--secondary);
}

.callout-index {
  flex: none;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace);
  font-size: 10px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  color: var(--secondary);
  opacity: .72;
  padding-top: 1px;
}

.callout-card.is-configured .callout-index {
  color: var(--accent);
  opacity: 1;
}

.callout-controls {
  display: flex;
  align-items: center;
  gap: 6px;
}

.callout-action {
  flex: 1;
  min-width: 0;
  min-height: 30px;
  padding: 4px 9px;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  text-align: left;
  font-size: 12px;
  background: var(--control);
  border: 1px solid transparent;
  color: var(--secondary);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), border-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.callout-card.is-configured .callout-action,
.extra-card.is-configured .callout-action {
  background: color-mix(in srgb, var(--accent) 9%, var(--surface));
  border-color: color-mix(in srgb, var(--accent) 22%, transparent);
  color: var(--accent);
  font-weight: 500;
}

@media (hover: hover) and (pointer: fine) {
  .callout-action:not(:disabled):hover {
    background: color-mix(in srgb, var(--text, #222) 7%, var(--surface));
    color: var(--text);
  }
  .callout-card.is-configured .callout-action:not(:disabled):hover,
  .extra-card.is-configured .callout-action:not(:disabled):hover {
    background: color-mix(in srgb, var(--accent) 14%, var(--surface));
    color: var(--accent);
  }
}

.callout-action.is-recording:disabled {
  opacity: 1;
  background: color-mix(in srgb, var(--warning, #ed8b23) 12%, var(--surface));
  border-color: color-mix(in srgb, var(--warning, #ed8b23) 34%, transparent);
  color: var(--text);
  font-weight: 500;
}

.action-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.action-chev {
  flex: none;
  font-size: 11px;
  color: var(--secondary);
}

.record-btn {
  flex: none;
  min-height: 30px;
  padding: 0 9px;
  border-radius: 7px;
  font-size: 11.5px;
  font-weight: 500;
  color: var(--secondary);
  background: var(--surface);
  border: 1px solid var(--line);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), border-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

@media (hover: hover) and (pointer: fine) {
  .record-btn:not(:disabled):hover {
    background: var(--control);
    color: var(--text);
  }
}

.record-btn.is-recording {
  background: color-mix(in srgb, var(--warning, #ed8b23) 14%, var(--surface));
  border-color: color-mix(in srgb, var(--warning, #ed8b23) 36%, transparent);
  color: var(--text);
}

.sub-warn {
  font-size: 11px;
  line-height: 1.4;
  color: var(--warning, #ed8b23);
}

.schem-center {
  position: relative;
  display: grid;
  place-items: center;
  min-width: 0;
  user-select: none;
}

.schem-frame {
  position: relative;
  width: min(100%, 248px);
  aspect-ratio: 320 / 300;
}

.schematic {
  position: relative;
  z-index: 1;
  display: block;
  width: 100%;
  height: 100%;
}

/* 示意图跟随浅色与深色主题，深色下为石墨机身，不产生刺眼白板。 */
.ge-stop-a { stop-color: color-mix(in srgb, var(--surface) 96%, var(--text)); }
.ge-stop-b { stop-color: color-mix(in srgb, var(--surface) 86%, var(--text)); }
.ge-wheel-a { stop-color: color-mix(in srgb, var(--secondary) 32%, var(--surface)); }
.ge-wheel-b { stop-color: color-mix(in srgb, var(--secondary) 58%, var(--surface)); }

.body {
  fill: url(#ge-body);
  stroke: color-mix(in srgb, var(--text, #222) 24%, transparent);
  stroke-width: 1.7;
}

.split {
  stroke: color-mix(in srgb, var(--text, #222) 15%, transparent);
  stroke-width: 1.3;
  fill: none;
}

.wheel {
  fill: url(#ge-wheel);
  stroke: color-mix(in srgb, var(--text, #222) 24%, transparent);
  stroke-width: 1.3;
  transition: fill var(--dur-fast, 120ms) var(--ease-out, ease);
}

.wheel.on {
  fill: var(--accent);
  stroke: color-mix(in srgb, var(--accent) 62%, #fff);
}

.tab {
  fill: url(#ge-body);
  stroke: color-mix(in srgb, var(--text, #222) 24%, transparent);
  stroke-width: 1.4;
  transition: fill var(--dur-fast, 120ms) var(--ease-out, ease), stroke var(--dur-fast, 120ms) var(--ease-out, ease);
}

.tab.on {
  fill: color-mix(in srgb, var(--accent) 24%, var(--surface));
  stroke: var(--accent);
}

.hotspot {
  position: absolute;
  width: 22px;
  height: 22px;
  margin: -11px 0 0 -11px;
  padding: 0;
  border-radius: 50%;
  background: var(--surface);
  border: 2px solid var(--accent);
  color: var(--accent);
  box-shadow: 0 2px 5px rgba(0, 0, 0, .18);
  display: grid;
  place-items: center;
  z-index: 3;
  transition: transform var(--dur-fast, 120ms) var(--ease-out, ease), box-shadow var(--dur-fast, 120ms) var(--ease-out, ease), background-color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.hotspot.configured {
  background: var(--accent);
  border-color: var(--surface);
  color: #fff;
}

.hotspot.selected {
  transform: scale(1.14);
  box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 24%, transparent), 0 2px 5px rgba(0, 0, 0, .2);
}

.hotspot.recording {
  background: var(--warning, #ed8b23);
  border-color: var(--surface);
  color: #fff;
}

.spot-num {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace);
  font-size: 9px;
  font-weight: 600;
  line-height: 1;
  font-variant-numeric: tabular-nums;
}

.extras-strip {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 11px 18px;
  border-top: 0.5px solid var(--line);
  background: var(--surface);
  flex-wrap: wrap;
}

.extras-title {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--secondary);
}

.extras-list {
  flex: 1;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.extra-card {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px 5px 10px;
  border-radius: 9px;
  background: var(--control);
  box-shadow: 0 0 0 1px var(--line);
}

.extra-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text);
  white-space: nowrap;
}

.sheet-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 11px 18px;
  border-top: 0.5px solid var(--line);
  background: color-mix(in srgb, var(--surface) 88%, var(--canvas));
  font-size: 11.5px;
  color: var(--secondary);
  flex-wrap: wrap;
}

.foot-left,
.foot-right {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.foot-boundary {
  color: var(--secondary);
}

@container (max-width: 660px) {
  .leader-svg {
    display: none;
  }
  .callout-canvas {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-template-rows: auto auto;
    align-items: start;
    min-height: 0;
    padding: 16px 14px;
  }
  .schem-center {
    grid-column: 1 / -1;
    grid-row: 1;
    padding-bottom: 4px;
  }
  .schem-frame {
    width: min(100%, 208px);
  }
  .callout-col {
    gap: 10px;
  }
}

@container (max-width: 440px) {
  .callout-canvas {
    grid-template-columns: 1fr;
    padding: 12px 10px;
  }
  .sheet-head,
  .sheet-foot,
  .extras-strip {
    padding: 10px 12px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .callout-card,
  .callout-action,
  .record-btn,
  .hotspot,
  .wheel,
  .tab,
  .leader-path,
  .leader-origin {
    transition: none;
  }
}
</style>

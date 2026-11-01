<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  G502_FACTORY, G502_PHOTO, G502_POSITIONS, G502_WHEEL_POSITIONS, MX_PHOTO, MX_POSITIONS,
  actionLabel, photoName, photoURL,
} from './model'

const props = defineProps<{
  model: string
  bindings: any[]
  locations: Record<string, number>
  availableButtons: number[]
  modifiers: number
  recording: number | null
  activeMenuButton?: number | null
  detecting: boolean
  captureReleased: boolean
  lastButton: number | null
  saveStatus: string
  lastExecution: string
  hidStatus: string
  loaded?: boolean
}>()

const emit = defineEmits<{
  record: [button: number]
  cancelRecord: []
  menu: [button: number, event: MouseEvent]
  beginCapture: [position: string]
  cancelCapture: []
  locate: [button: number, position: string]
  detectExtension: []
}>()

interface CalloutItem {
  key: string
  codeNum: string
  column: 'left' | 'right'
  title: string
  detail: string
  nx: number
  ny: number
  button: number
  configured: boolean
  actionText: string
  isWheelGroup: boolean
  extensionLocked: boolean
  extensionMissing: boolean
}

interface LeaderLine {
  key: string
  d: string
  sx: number
  sy: number
  configured: boolean
}

const isG502 = computed(() => props.model === 'g502Lightspeed')
const side = ref<'top' | 'side'>('top')
const wheelChoice = ref<'middle' | 'tiltLeft' | 'tiltRight'>('middle')
const locating = ref<string | null>(null)
const candidate = ref<number | null>(null)
const hoveredKey = ref<string | null>(null)
const canvasEl = ref<HTMLElement | null>(null)
const lines = ref<LeaderLine[]>([])

let observer: ResizeObserver | null = null
let rafId = 0

const photoDims = computed(() => (isG502.value ? G502_PHOTO : MX_PHOTO))
const imageURL = computed(() => photoURL(photoName(props.model, side.value)))

function locations(): Record<string, number> {
  return props.locations ?? {}
}

function g502Button(position: string): number {
  return Number(locations()[position] ?? G502_FACTORY[position])
}

function effectiveModifiers(button: number): number {
  if (isG502.value || button >= 195) return 0
  return props.modifiers
}

function bindingFor(button: number): any {
  const mods = effectiveModifiers(button)
  return props.bindings.find(
    (entry) => Number(entry.button) === button && Number(entry.modifiers) === mods,
  )?.action ?? null
}

function configuredAtCurrentLayer(button: number): boolean {
  return Boolean(bindingFor(button))
}

function configuredAnyLayer(button: number): boolean {
  return props.bindings.some((entry) => Number(entry.button) === button && entry.action)
}

function g502PositionConfigured(position: string): boolean {
  const buttons = (G502_WHEEL_POSITIONS as readonly string[]).includes(position)
    ? G502_WHEEL_POSITIONS.map((id) => g502Button(id))
    : [g502Button(position)]
  return buttons.some((btn) => configuredAnyLayer(btn))
}

function labelFor(button: number): string {
  return actionLabel(bindingFor(button), effectiveModifiers(button))
}

const topCustomCount = computed(() => {
  if (isG502.value) {
    return ['left', 'right', 'middle', 'dpiUp', 'dpiDown', 'top'].filter(g502PositionConfigured).length
  }
  return [0, 1, 2, 196].filter(configuredAnyLayer).length
})

const sideCustomCount = computed(() => {
  if (isG502.value) {
    return ['forward', 'back', 'thumb'].filter(g502PositionConfigured).length
  }
  return [4, 3, 195].filter(configuredAnyLayer).length
})

const MX_META: Record<number, { shortTitle: string; detail: string; column: 'left' | 'right'; order: number }> = {
  // top: 左列 (0.16 -> 0.23)，右列 (0.18 -> 0.40)，纵坐标严格单调且贴近真实高度
  0: { shortTitle: '左键', detail: '食指主键', column: 'left', order: 1 },
  2: { shortTitle: '中键 / 滚轮按下', detail: '按下滚轮', column: 'left', order: 2 },
  1: { shortTitle: '右键', detail: '中指副键', column: 'right', order: 3 },
  196: { shortTitle: '滚轮模式切换键', detail: 'HID++ 模式键', column: 'right', order: 4 },
  // side: 左列 (0.47, 0.485)，右列 (0.58, 0.445 -> 0.61, 0.685)，折线零交叉
  4: { shortTitle: '前进侧键', detail: '按钮 5 · 拇指靠前', column: 'left', order: 1 },
  3: { shortTitle: '后退侧键', detail: '按钮 4 · 拇指靠后', column: 'right', order: 2 },
  195: { shortTitle: '拇指手势键', detail: 'HID++ 拇指托', column: 'right', order: 3 },
}

const G502_COLUMN_ORDER: Record<string, { column: 'left' | 'right'; order: number }> = {
  // top: 左列 (190 -> 240 -> 325)，右列 (205 -> 270 -> 490)
  left: { column: 'left', order: 1 },
  dpiUp: { column: 'left', order: 2 },
  dpiDown: { column: 'left', order: 3 },
  right: { column: 'right', order: 4 },
  middle: { column: 'right', order: 5 },
  top: { column: 'right', order: 6 },
  // side: 左列 (515 -> 620)，右列 (480)
  forward: { column: 'left', order: 1 },
  thumb: { column: 'left', order: 2 },
  back: { column: 'right', order: 3 },
}

const currentItems = computed<CalloutItem[]>(() => {
  if (isG502.value) {
    return G502_POSITIONS
      .filter((pos) => pos.side === side.value && pos.id !== 'tiltLeft' && pos.id !== 'tiltRight')
      .sort((a, b) => (G502_COLUMN_ORDER[a.id]?.order ?? 99) - (G502_COLUMN_ORDER[b.id]?.order ?? 99))
      .map((pos, idx) => {
        const isWheel = pos.id === 'middle'
        const activePos = isWheel ? wheelChoice.value : pos.id
        const activeMeta = isWheel ? (G502_POSITIONS.find((p) => p.id === wheelChoice.value) ?? pos) : pos
        const button = g502Button(activePos)
        const colInfo = G502_COLUMN_ORDER[pos.id] ?? { column: 'right' as const, order: idx + 1 }
        return {
          key: pos.id,
          codeNum: String(idx + 1).padStart(2, '0'),
          column: colInfo.column,
          title: activeMeta.title,
          detail: activeMeta.detail,
          nx: pos.x / G502_PHOTO.width,
          ny: pos.y / G502_PHOTO.height,
          button,
          configured: isWheel ? g502PositionConfigured('middle') : configuredAtCurrentLayer(button),
          actionText: labelFor(button),
          isWheelGroup: isWheel,
          extensionLocked: false,
          extensionMissing: false,
        }
      })
  }

  return MX_POSITIONS
    .filter((pos) => pos.side === side.value)
    .sort((a, b) => (MX_META[a.button]?.order ?? 99) - (MX_META[b.button]?.order ?? 99))
    .map((pos, idx) => {
      const meta = MX_META[pos.button] ?? { shortTitle: pos.title, detail: '', column: 'right' as const, order: idx + 1 }
      const isExt = pos.button >= 195
      const available = !isExt || props.availableButtons.includes(pos.button)
      return {
        key: String(pos.button),
        codeNum: String(idx + 1).padStart(2, '0'),
        column: meta.column,
        title: meta.shortTitle,
        detail: meta.detail,
        nx: pos.x,
        ny: pos.y,
        button: pos.button,
        configured: configuredAtCurrentLayer(pos.button),
        actionText: labelFor(pos.button),
        isWheelGroup: false,
        extensionLocked: isExt && props.modifiers !== 0,
        extensionMissing: !available,
      }
    })
})

const leftItems = computed(() => currentItems.value.filter((item) => item.column === 'left'))
const rightItems = computed(() => currentItems.value.filter((item) => item.column === 'right'))

function isHighlighted(item: CalloutItem): boolean {
  if (hoveredKey.value === item.key || locating.value === item.key) return true
  if (props.recording !== null && props.recording === item.button) return true
  if (props.activeMenuButton !== null && props.activeMenuButton !== undefined && props.activeMenuButton === item.button) return true
  return false
}

function isPrimaryLeftRebound(item: CalloutItem): boolean {
  if (item.button !== 0) return false
  const action = bindingFor(0)
  return Boolean(action && !(action.kind === 'mouse' && Number(action.button) === 0))
}

function wheelDirectionConfigured(dir: 'middle' | 'tiltLeft' | 'tiltRight'): boolean {
  return configuredAnyLayer(g502Button(dir))
}

function setSide(next: 'top' | 'side') {
  if (side.value === next) return
  if (locating.value) cancelLocate()
  side.value = next
}

function openItemMenu(item: CalloutItem, event: MouseEvent) {
  if (props.loaded === false || props.recording !== null || item.extensionMissing) return
  emit('menu', item.button, event)
}

function onSpotClick(item: CalloutItem, event: MouseEvent) {
  const cardBtn = canvasEl.value?.querySelector<HTMLElement>(`[data-callout-key="${item.key}"] .callout-action`)
  if (cardBtn && !item.extensionMissing && locating.value !== item.key) {
    cardBtn.focus({ preventScroll: true })
    emit('menu', item.button, { currentTarget: cardBtn } as unknown as MouseEvent)
    return
  }
  openItemMenu(item, event)
}

function beginItemRecord(item: CalloutItem) {
  if (props.loaded === false || item.extensionMissing) return
  if (props.recording === item.button) {
    emit('cancelRecord')
    return
  }
  emit('record', item.button)
}

function startCapture(posKey: string) {
  const resolved = posKey === 'middle' ? wheelChoice.value : posKey
  candidate.value = null
  locating.value = posKey
  emit('beginCapture', resolved)
}

function confirmCandidate(posKey: string) {
  if (candidate.value === null) return
  const resolved = posKey === 'middle' ? wheelChoice.value : posKey
  emit('locate', candidate.value, resolved)
  candidate.value = null
  locating.value = null
}

function cancelLocate() {
  locating.value = null
  candidate.value = null
  emit('cancelCapture')
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
  for (const item of currentItems.value) {
    const card = root.querySelector<HTMLElement>(`[data-callout-key="${item.key}"]`)
    const spot = root.querySelector<HTMLElement>(`[data-spot-key="${item.key}"]`)
    if (!card || !spot) continue
    const cRect = card.getBoundingClientRect()
    const sRect = spot.getBoundingClientRect()
    if (cRect.width === 0 || sRect.width === 0) continue
    const isLeft = item.column === 'left'
    const sx = (isLeft ? cRect.right : cRect.left) - rootRect.left
    const sy = cRect.top + cRect.height / 2 - rootRect.top
    const ex = sRect.left + sRect.width / 2 - rootRect.left
    const ey = sRect.top + sRect.height / 2 - rootRect.top
    const kx = sx + (ex - sx) * 0.44
    next.push({
      key: item.key,
      d: `M${sx.toFixed(1)},${sy.toFixed(1)} L${kx.toFixed(1)},${sy.toFixed(1)} L${ex.toFixed(1)},${ey.toFixed(1)}`,
      sx: Number(sx.toFixed(1)),
      sy: Number(sy.toFixed(1)),
      configured: item.configured,
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
  () => [props.detecting, props.captureReleased, props.lastButton] as const,
  ([, released, last]) => {
    if (locating.value && released && last !== null && last >= 208 && last <= 218) {
      candidate.value = last
    }
  },
)

watch(
  () => [props.model, side.value, wheelChoice.value, locating.value, candidate.value, props.bindings, props.modifiers] as const,
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
  if (locating.value) emit('cancelCapture')
})
</script>

<template>
  <section class="callout-sheet">
    <header class="sheet-head">
      <div class="head-left">
        <slot name="device" />
      </div>
      <div class="angle-picker" role="group" aria-label="鼠标视角">
        <button
          type="button"
          :class="{ active: side === 'top' }"
          :aria-pressed="side === 'top'"
          @click="setSide('top')"
        >
          <span>顶视</span>
          <b v-if="topCustomCount > 0" class="angle-count">{{ topCustomCount }}</b>
        </button>
        <button
          type="button"
          :class="{ active: side === 'side' }"
          :aria-pressed="side === 'side'"
          @click="setSide('side')"
        >
          <span>侧视</span>
          <b v-if="sideCustomCount > 0" class="angle-count">{{ sideCustomCount }}</b>
        </button>
      </div>
    </header>

    <div ref="canvasEl" class="callout-canvas" :class="{ 'is-side': side === 'side', 'is-g502': isG502 }">
      <svg class="leader-svg" aria-hidden="true">
        <g v-for="line in lines" :key="line.key">
          <path :d="line.d" class="leader-halo" />
          <path
            :d="line.d"
            class="leader-path"
            :class="{
              'is-configured': line.configured,
              'is-highlighted': hoveredKey === line.key || locating === line.key,
            }"
          />
          <circle
            :cx="line.sx"
            :cy="line.sy"
            r="2.4"
            class="leader-origin"
            :class="{
              'is-configured': line.configured,
              'is-highlighted': hoveredKey === line.key || locating === line.key,
            }"
          />
        </g>
      </svg>

      <!-- 左侧就地标注列 -->
      <div class="callout-col is-left">
        <article
          v-for="item in leftItems"
          :key="item.key"
          class="callout-card"
          :class="{
            'is-configured': item.configured,
            'is-highlighted': isHighlighted(item),
            'is-recording': recording === item.button,
          }"
          :data-callout-key="item.key"
          @mouseenter="hoveredKey = item.key"
          @mouseleave="hoveredKey = null"
          @focusin="hoveredKey = item.key"
          @focusout="hoveredKey = null"
        >
          <div class="callout-top">
            <div class="callout-titles">
              <strong>{{ item.title }}</strong>
              <small v-if="item.detail">{{ item.detail }}</small>
            </div>
            <span class="callout-index">{{ item.codeNum }}</span>
          </div>

          <template v-if="isG502 && locating === item.key">
            <div class="locate-box">
              <template v-if="candidate !== null">
                <p class="locate-status">已识别实体键 #{{ candidate - 207 }}</p>
                <div class="locate-actions">
                  <button type="button" class="mini-btn is-primary" @click="confirmCandidate(item.key)">确认此位置</button>
                  <button type="button" class="mini-link" @click="startCapture(item.key)">重测</button>
                  <button type="button" class="mini-link" @click="cancelLocate">取消</button>
                </div>
              </template>
              <template v-else>
                <p class="locate-status">
                  {{ detecting ? `请按下并松开「${item.title}」…` : captureReleased ? '未识别，请重试' : lastButton !== null ? '请松开按键…' : '等待按键输入…' }}
                </p>
                <div class="locate-actions">
                  <button type="button" class="mini-btn" @click="startCapture(item.key)">重新监听</button>
                  <button type="button" class="mini-link" @click="cancelLocate">取消</button>
                </div>
              </template>
            </div>
          </template>

          <template v-else-if="item.extensionMissing">
            <div class="ext-missing">
              <span>尚未检测到此扩展键</span>
              <button type="button" class="mini-btn" @click="emit('detectExtension')">检测扩展键</button>
            </div>
          </template>

          <template v-else>
            <div class="callout-controls">
              <button
                type="button"
                class="callout-action"
                :class="{ 'is-recording': recording === item.button }"
                :disabled="loaded === false || recording !== null"
                @click="openItemMenu(item, $event)"
              >
                <span class="action-text">{{ recording === item.button ? '请按键盘快捷键…' : item.actionText }}</span>
                <span class="action-chev" aria-hidden="true">⌄</span>
              </button>
              <button
                type="button"
                class="record-btn"
                :class="{ 'is-recording': recording === item.button }"
                :disabled="loaded === false || (recording !== null && recording !== item.button)"
                :title="recording === item.button ? '按 Esc 或点击取消录制' : '直接录制键盘快捷键'"
                @click="beginItemRecord(item)"
              >
                {{ recording === item.button ? '取消' : '录制' }}
              </button>
            </div>
            <div v-if="isG502 || item.extensionLocked || isPrimaryLeftRebound(item)" class="callout-sub">
              <span v-if="item.extensionLocked" class="sub-hint">此扩展键仅单层生效，不受修饰键影响</span>
              <span v-else-if="isPrimaryLeftRebound(item)" class="sub-warn">左键已改绑，请保留触控板或辅助点击方式</span>
              <button v-if="isG502" type="button" class="mini-link" @click="startCapture(item.key)">重认键位</button>
            </div>
          </template>
        </article>
      </div>

      <!-- 中央鼠标实物与锚点 -->
      <div class="photo-center">
        <div class="photo-frame" :class="{ 'is-g502': isG502, 'is-side': side === 'side' }">
          <img
            :src="imageURL"
            alt=""
            draggable="false"
            class="photo"
            :width="photoDims.width"
            :height="photoDims.height"
            @load="scheduleLines"
          />
          <button
            v-for="item in currentItems"
            :key="`spot-${item.key}`"
            type="button"
            class="hotspot"
            :class="{
              configured: item.configured,
              selected: isHighlighted(item),
              recording: recording === item.button,
            }"
            :data-spot-key="item.key"
            :style="{ left: `${item.nx * 100}%`, top: `${item.ny * 100}%` }"
            :title="`${item.title} · ${item.actionText}`"
            :aria-label="`${item.title}，当前 ${item.actionText}`"
            @mouseenter="hoveredKey = item.key"
            @mouseleave="hoveredKey = null"
            @focus="hoveredKey = item.key"
            @blur="hoveredKey = null"
            @click.stop="onSpotClick(item, $event)"
          >
            <span class="spot-num">{{ item.codeNum }}</span>
          </button>
        </div>
      </div>

      <!-- 右侧就地标注列 -->
      <div class="callout-col is-right">
        <article
          v-for="item in rightItems"
          :key="item.key"
          class="callout-card"
          :class="{
            'is-configured': item.configured,
            'is-highlighted': isHighlighted(item),
            'is-recording': recording === item.button,
          }"
          :data-callout-key="item.key"
          @mouseenter="hoveredKey = item.key"
          @mouseleave="hoveredKey = null"
          @focusin="hoveredKey = item.key"
          @focusout="hoveredKey = null"
        >
          <div class="callout-top">
            <div class="callout-titles">
              <strong>{{ item.title }}</strong>
              <small v-if="item.detail">{{ item.detail }}</small>
            </div>
            <span class="callout-index">{{ item.codeNum }}</span>
          </div>

          <div v-if="item.isWheelGroup" class="wheel-dirs" role="group" aria-label="滚轮触发方向">
            <button
              type="button"
              :class="{ active: wheelChoice === 'middle', 'has-dot': wheelDirectionConfigured('middle') }"
              @click="wheelChoice = 'middle'"
            >按下</button>
            <button
              type="button"
              :class="{ active: wheelChoice === 'tiltLeft', 'has-dot': wheelDirectionConfigured('tiltLeft') }"
              @click="wheelChoice = 'tiltLeft'"
            >左拨</button>
            <button
              type="button"
              :class="{ active: wheelChoice === 'tiltRight', 'has-dot': wheelDirectionConfigured('tiltRight') }"
              @click="wheelChoice = 'tiltRight'"
            >右拨</button>
          </div>

          <template v-if="isG502 && locating === item.key">
            <div class="locate-box">
              <template v-if="candidate !== null">
                <p class="locate-status">已识别实体键 #{{ candidate - 207 }}</p>
                <div class="locate-actions">
                  <button type="button" class="mini-btn is-primary" @click="confirmCandidate(item.key)">确认此位置</button>
                  <button type="button" class="mini-link" @click="startCapture(item.key)">重测</button>
                  <button type="button" class="mini-link" @click="cancelLocate">取消</button>
                </div>
              </template>
              <template v-else>
                <p class="locate-status">
                  {{ detecting ? `请按下并松开「${item.title}」…` : captureReleased ? '未识别，请重试' : lastButton !== null ? '请松开按键…' : '等待按键输入…' }}
                </p>
                <div class="locate-actions">
                  <button type="button" class="mini-btn" @click="startCapture(item.key)">重新监听</button>
                  <button type="button" class="mini-link" @click="cancelLocate">取消</button>
                </div>
              </template>
            </div>
          </template>

          <template v-else-if="item.extensionMissing">
            <div class="ext-missing">
              <span>尚未检测到此扩展键</span>
              <button type="button" class="mini-btn" @click="emit('detectExtension')">检测扩展键</button>
            </div>
          </template>

          <template v-else>
            <div class="callout-controls">
              <button
                type="button"
                class="callout-action"
                :class="{ 'is-recording': recording === item.button }"
                :disabled="loaded === false || recording !== null"
                @click="openItemMenu(item, $event)"
              >
                <span class="action-text">{{ recording === item.button ? '请按键盘快捷键…' : item.actionText }}</span>
                <span class="action-chev" aria-hidden="true">⌄</span>
              </button>
              <button
                type="button"
                class="record-btn"
                :class="{ 'is-recording': recording === item.button }"
                :disabled="loaded === false || (recording !== null && recording !== item.button)"
                :title="recording === item.button ? '按 Esc 或点击取消录制' : '直接录制键盘快捷键'"
                @click="beginItemRecord(item)"
              >
                {{ recording === item.button ? '取消' : '录制' }}
              </button>
            </div>
            <div v-if="isG502 || item.extensionLocked" class="callout-sub">
              <span v-if="item.extensionLocked" class="sub-hint">此扩展键仅单层生效，不受修饰键影响</span>
              <button v-if="isG502" type="button" class="mini-link" @click="startCapture(item.key)">重认键位</button>
            </div>
          </template>
        </article>
      </div>
    </div>

    <footer class="sheet-foot">
      <div class="foot-left">
        <template v-if="isG502">
          <span class="foot-note">保留鼠标原有 DPI 与板载配置，只附加快捷键</span>
          <span v-if="lastExecution || saveStatus" class="foot-status">{{ lastExecution || saveStatus }}</span>
        </template>
        <template v-else>
          <slot name="modifiers" />
        </template>
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

.angle-picker {
  display: inline-flex;
  padding: 3px;
  gap: 2px;
  border-radius: 999px;
  background: var(--control);
  flex: none;
}

.angle-picker button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 13px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  color: var(--secondary);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.angle-picker button.active {
  background: var(--accent);
  color: #fff;
}

.angle-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  border-radius: 999px;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace);
  font-size: 10px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  background: color-mix(in srgb, var(--accent) 20%, transparent);
  color: var(--accent);
}

.angle-picker button.active .angle-count {
  background: rgba(255, 255, 255, .24);
  color: #fff;
}

.callout-canvas {
  position: relative;
  isolation: isolate;
  min-height: 388px;
  display: grid;
  grid-template-columns: minmax(196px, 260px) minmax(176px, 1fr) minmax(196px, 260px);
  align-items: center;
  gap: 12px;
  padding: 20px 18px;
  background-color: var(--surface);
  background-image:
    radial-gradient(ellipse 46% 58% at 50% 50%, var(--surface) 38%, transparent 82%),
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
  stroke: color-mix(in srgb, var(--surface) 58%, transparent);
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

.wheel-dirs {
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  border-radius: 7px;
  background: var(--control);
}

.wheel-dirs button {
  flex: 1;
  position: relative;
  padding: 3px 8px;
  border-radius: 5px;
  font-size: 11px;
  font-weight: 500;
  color: var(--secondary);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.wheel-dirs button.active {
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 0 0 0.5px var(--line);
}

.wheel-dirs button.has-dot::after {
  content: '';
  display: inline-block;
  width: 4px;
  height: 4px;
  margin-left: 4px;
  vertical-align: middle;
  border-radius: 50%;
  background: var(--accent);
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

.callout-card.is-configured .callout-action {
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
  .callout-card.is-configured .callout-action:not(:disabled):hover {
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

.callout-sub {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
}

.sub-hint {
  font-size: 11px;
  line-height: 1.4;
  color: var(--secondary);
}

.sub-warn {
  font-size: 11px;
  line-height: 1.4;
  color: var(--warning, #ed8b23);
}

.ext-missing,
.locate-box {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 2px;
}

.ext-missing span,
.locate-status {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.45;
  color: var(--secondary);
}

.locate-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.mini-btn {
  align-self: flex-start;
  min-height: 26px;
  padding: 0 10px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--surface);
  font-size: 11.5px;
  font-weight: 500;
}

.mini-btn.is-primary {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}

.mini-link {
  font-size: 11px;
  color: var(--accent);
}

.mini-link:hover {
  text-decoration: underline;
}

.photo-center {
  position: relative;
  display: grid;
  place-items: center;
  min-width: 0;
  user-select: none;
}

.photo-frame {
  position: relative;
  width: min(100%, 340px);
  aspect-ratio: 2400 / 2061;
}

.photo-frame.is-g502 {
  width: min(100%, 360px);
  aspect-ratio: 1384 / 1038;
}

.photo {
  position: relative;
  z-index: 1;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  pointer-events: none;
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
  box-shadow: 0 2px 5px rgba(0, 0, 0, .2);
  display: grid;
  place-items: center;
  z-index: 3;
  transition: transform var(--dur-fast, 120ms) var(--ease-out, ease), box-shadow var(--dur-fast, 120ms) var(--ease-out, ease), background-color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.hotspot.configured {
  background: var(--accent);
  border-color: #fff;
  color: #fff;
}

.hotspot.selected {
  transform: scale(1.14);
  box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 24%, transparent), 0 2px 5px rgba(0, 0, 0, .22);
}

.hotspot.recording {
  background: var(--warning, #ed8b23);
  border-color: #fff;
  color: #fff;
}

.spot-num {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace);
  font-size: 9px;
  font-weight: 600;
  line-height: 1;
  font-variant-numeric: tabular-nums;
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
  gap: 10px;
  flex-wrap: wrap;
}

.foot-status {
  color: var(--accent);
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
  .photo-center {
    grid-column: 1 / -1;
    grid-row: 1;
    padding-bottom: 4px;
  }
  .photo-frame,
  .photo-frame.is-g502 {
    width: min(100%, 236px);
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
  .sheet-foot {
    padding: 10px 12px;
  }
  .angle-picker {
    max-width: 100%;
  }
  .angle-picker button {
    padding: 4px 8px;
    gap: 4px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .callout-card,
  .callout-action,
  .record-btn,
  .hotspot,
  .leader-path,
  .leader-origin,
  .angle-picker button,
  .wheel-dirs button {
    transition: none;
  }
}
</style>

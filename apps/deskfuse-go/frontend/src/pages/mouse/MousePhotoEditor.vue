<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  G502_FACTORY, G502_PHOTO, G502_POSITIONS, G502_WHEEL_POSITIONS, MX_PHOTO, MX_POSITIONS,
  actionLabel, buttonName, photoName, photoURL,
} from './model'

const props = defineProps<{
  model: string
  bindings: any[]
  locations: Record<string, number>
  availableButtons: number[]
  modifiers: number
  recording: number | null
  detecting: boolean
  captureReleased: boolean
  lastButton: number | null
  saveStatus: string
  lastExecution: string
  hidStatus: string
}>()

const emit = defineEmits<{
  record: [button: number]
  menu: [button: number, event: MouseEvent]
  beginCapture: [position: string]
  cancelCapture: []
  locate: [button: number, position: string]
  detectExtension: []
}>()

const isG502 = computed(() => props.model === 'g502Lightspeed')
const side = ref<'top' | 'side'>('top')
const activePosition = ref<string | null>(null)
const activeButton = ref<number | null>(null)
const wheelChoice = ref<'middle' | 'tiltLeft' | 'tiltRight'>('middle')
const locating = ref<string | null>(null)
const candidate = ref<number | null>(null)
const stage = ref<HTMLElement | null>(null)
const size = ref({ width: 0, height: 0 })

let observer: ResizeObserver | null = null
function measure() {
  if (stage.value) size.value = { width: stage.value.clientWidth, height: stage.value.clientHeight }
}
onMounted(() => {
  measure()
  observer = new ResizeObserver(measure)
  if (stage.value) observer.observe(stage.value)
})
onBeforeUnmount(() => {
  observer?.disconnect()
  if (locating.value) emit('cancelCapture')
})

const imageURL = computed(() => photoURL(photoName(props.model, side.value)))
const inspectorOpen = computed(() => activePosition.value !== null || activeButton.value !== null)

const layout = computed(() => {
  const width = Math.max(1, size.value.width)
  const height = Math.max(1, size.value.height)
  let photoWidth: number
  let photoHeight: number
  let scale = 0
  if (isG502.value) {
    scale = side.value === 'top' ? (height - 35) / G502_PHOTO.height : Math.min(width - 40, 740) / G502_PHOTO.width
    scale = Math.max(0.05, scale)
    photoWidth = G502_PHOTO.width * scale
    photoHeight = G502_PHOTO.height * scale
  } else if (side.value === 'top') {
    photoWidth = height * MX_PHOTO.width / MX_PHOTO.height
    photoHeight = photoWidth * MX_PHOTO.height / MX_PHOTO.width
  } else {
    photoWidth = Math.min(width - 40, 740)
    photoHeight = photoWidth * MX_PHOTO.height / MX_PHOTO.width
  }
  const x = (width - photoWidth) / 2
  const y = (height - photoHeight) / 2
  const spots = isG502.value
    ? G502_POSITIONS.filter((position) => position.side === side.value && position.id !== 'tiltLeft' && position.id !== 'tiltRight')
      .map((position) => ({
        key: position.id, title: position.title,
        x: x + position.x * scale, y: y + position.y * scale,
        selected: activePosition.value === position.id,
        configured: g502Configured(position.id),
      }))
    : MX_POSITIONS.filter((position) => position.side === side.value).map((position) => ({
      key: String(position.button), title: position.title,
      x: x + position.x * photoWidth, y: y + position.y * photoHeight,
      selected: activeButton.value === position.button,
      configured: configuredFor(position.button),
    }))
  return { width, height, x, y, photoWidth, photoHeight, spots }
})

function locations(): Record<string, number> {
  return props.locations ?? {}
}
function g502Button(position: string): number {
  return Number(locations()[position] ?? G502_FACTORY[position])
}
function sharedWheelButtons(): number[] {
  return G502_WHEEL_POSITIONS.map((position) => g502Button(position))
}
function g502Configured(position: string): boolean {
  const buttons = (G502_WHEEL_POSITIONS as readonly string[]).includes(position)
    ? sharedWheelButtons()
    : [g502Button(position)]
  return props.bindings.some((binding) => buttons.includes(Number(binding.button)))
}
function configuredFor(button: number): boolean {
  return props.bindings.some((binding) => Number(binding.button) === button)
}
function bindingFor(button: number): any {
  return props.bindings.find(
    (binding) => Number(binding.button) === button && Number(binding.modifiers) === props.modifiers,
  )?.action ?? null
}
function labelFor(button: number): string {
  return actionLabel(bindingFor(button), props.modifiers)
}

const target = computed(() => {
  if (isG502.value) {
    const position = activePosition.value ?? 'middle'
    const key = (G502_WHEEL_POSITIONS as readonly string[]).includes(position) ? wheelChoice.value : position
    return { position: key, button: g502Button(key), wheel: (G502_WHEEL_POSITIONS as readonly string[]).includes(position) }
  }
  const button = activeButton.value ?? 0
  return { position: '', button, wheel: false }
})

const positionInfo = computed(() => G502_POSITIONS.find((position) => position.id === target.value.position) ?? null)
const visibleAvailable = computed(() => activeButton.value === null ? true : activeButton.value < 195 || props.availableButtons.includes(activeButton.value))

const inspectorStyle = computed(() => {
  const spot = layout.value.spots.find((entry) => entry.key === (isG502.value ? activePosition.value : String(activeButton.value)))
  if (!spot) return {}
  const width = 280
  const left = Math.max(10, Math.min(spot.x + 26, layout.value.width - width - 12))
  const top = Math.max(10, Math.min(spot.y - 40, layout.value.height - 320))
  return { left: `${left}px`, top: `${top}px`, width: `${width}px` }
})

function selectSpot(key: string) {
  if (locating.value) cancelLocate()
  if (isG502.value) {
    activePosition.value = key
    wheelChoice.value = (G502_WHEEL_POSITIONS as readonly string[]).includes(key)
      ? (key as 'middle' | 'tiltLeft' | 'tiltRight')
      : 'middle'
  } else {
    activeButton.value = Number(key)
  }
}
function closeInspector() {
  if (locating.value) cancelLocate()
  activePosition.value = null
  activeButton.value = null
}
function setSide(value: 'top' | 'side') {
  if (side.value === value) return
  closeInspector()
  side.value = value
}
function openMenu(event: MouseEvent) {
  emit('menu', target.value.button, event)
}
function beginRecord() {
  emit('record', target.value.button)
}
function startCapture() {
  if (!activePosition.value) return
  candidate.value = null
  locating.value = activePosition.value
  emit('beginCapture', activePosition.value)
}
function confirmCandidate() {
  if (candidate.value === null || !activePosition.value) return
  emit('locate', candidate.value, activePosition.value)
  candidate.value = null
  locating.value = null
  activePosition.value = null
}
function cancelLocate() {
  locating.value = null
  candidate.value = null
  emit('cancelCapture')
}

watch(
  () => [props.detecting, props.captureReleased, props.lastButton] as const,
  ([, released, last]) => {
    if (locating.value && released && last !== null && last >= 208 && last <= 218) {
      candidate.value = last
    }
  },
)
watch(side, () => closeInspector())
</script>

<template>
  <section class="photo-editor">
    <header class="editor-head">
      <span class="hint"><span class="cursor" aria-hidden="true">▸</span>点击按键，直接分配功能</span>
      <div class="angle-picker" role="group" aria-label="视角">
        <button type="button" :class="{ active: side === 'top' }" @click="setSide('top')">顶视</button>
        <button type="button" :class="{ active: side === 'side' }" @click="setSide('side')">侧视</button>
      </div>
    </header>

    <div ref="stage" class="stage" :class="{ side: side === 'side' }">
      <div class="photo-layer">
        <img :src="imageURL" alt="" draggable="false" class="photo" />
        <button
          v-for="spot in layout.spots" :key="spot.key" type="button"
          class="hotspot" :class="{ configured: spot.configured, selected: spot.selected }"
          :style="{ left: `${spot.x}px`, top: `${spot.y}px` }"
          :title="spot.title" :aria-label="spot.title"
          @click.stop="selectSpot(spot.key)"
        >
          <span v-if="spot.configured" class="check">✓</span>
        </button>
        <div class="view-caption">{{ side === 'side' ? (isG502 ? '拇指侧 · 3 个按键' : '侧视 · 手势键与侧键') : (isG502 ? '滚轮支持按下、左拨与右拨' : '顶视 · 主键与滚轮') }}</div>
      </div>

      <div v-if="inspectorOpen" class="dismiss" @click="closeInspector" />
      <div v-if="inspectorOpen" class="inspector" :style="inspectorStyle" role="dialog">
        <div class="inspector-head">
          <div>
            <strong>{{ isG502 ? positionInfo?.title : (activeButton !== null ? buttonName(activeButton) : '') }}</strong>
            <small v-if="isG502 && positionInfo">{{ positionInfo.detail }}</small>
          </div>
          <button type="button" class="icon-button" aria-label="关闭按键设置" @click="closeInspector">✕</button>
        </div>

        <template v-if="isG502">
          <div v-if="target.wheel" class="segmented" role="group" aria-label="滚轮动作">
            <button type="button" :class="{ active: wheelChoice === 'middle' }" @click="wheelChoice = 'middle'">按下</button>
            <button type="button" :class="{ active: wheelChoice === 'tiltLeft' }" @click="wheelChoice = 'tiltLeft'">左拨</button>
            <button type="button" :class="{ active: wheelChoice === 'tiltRight' }" @click="wheelChoice = 'tiltRight'">右拨</button>
          </div>

          <template v-if="locating">
            <template v-if="candidate !== null">
              <p class="accent-line">✓ 已收到按键</p>
              <p class="muted">刚才按下的是「{{ positionInfo?.title }}」吗？确认后才会记住位置。</p>
              <button type="button" class="button primary" @click="confirmCandidate">确认这个位置</button>
              <button type="button" class="link-button" @click="startCapture">重新识别</button>
              <button type="button" class="link-button" @click="cancelLocate">取消</button>
            </template>
            <template v-else>
              <p class="accent-line">
                {{ detecting ? '正在监听…' : captureReleased ? '请重试识别' : lastButton !== null ? '请松开按键' : '未收到按键' }}
              </p>
              <p class="muted">按下并松开「{{ positionInfo?.title }}」。只确认位置，不改变鼠标原有设置。</p>
              <div class="inline-actions">
                <button type="button" class="button" @click="startCapture">重新监听</button>
                <button type="button" class="link-button" @click="cancelLocate">取消</button>
              </div>
            </template>
          </template>

          <template v-else>
            <small class="section-label">附加动作</small>
            <button type="button" class="action-choice" :disabled="recording !== null" @click="openMenu">
              <span>{{ recording === target.button ? '请按快捷键…' : labelFor(target.button) }}</span>
              <span class="chevron">⌃⌄</span>
            </button>
            <small class="muted">{{ saveStatus }}</small>
            <small class="muted">{{ lastExecution }}</small>
            <small class="muted">自动清除该位置的通用按键映射，避免重复执行动作。</small>
            <button type="button" class="link-button" @click="startCapture">按键不对应？重新识别</button>
          </template>
        </template>

        <template v-else>
          <template v-if="visibleAvailable">
            <button
              type="button" class="action-choice" :disabled="recording !== null"
              @click="openMenu"
            >
              <span>{{ recording === activeButton ? '请按快捷键…' : labelFor(activeButton ?? 0) }}</span>
              <span class="chevron">⌃⌄</span>
            </button>
            <small class="muted">{{ saveStatus }}</small>
            <small v-if="(activeButton ?? 0) >= 195" class="muted">设置动作会接管此扩展键；选择保持原始功能可恢复原行为。</small>
          </template>
          <template v-else>
            <p class="muted">尚未检测到此扩展键。请连接 MX Master 3，并允许输入监控权限。</p>
            <button type="button" class="button" @click="emit('detectExtension')">检测扩展键</button>
            <small class="muted">{{ hidStatus }}</small>
          </template>
        </template>
      </div>
    </div>

    <footer class="editor-foot">
      {{ isG502
        ? '保留原生功能，只附加快捷键'
        : '普通按键直接设置 · 手势键与模式键需检测到 HID++ 支持 · 横向滚轮保留原功能' }}
    </footer>
  </section>
</template>

<style scoped>
.photo-editor {
  display: flex; flex-direction: column; background: var(--surface); border-radius: 16px;
  border: 1px solid color-mix(in srgb, var(--text, #222) 6%, transparent); overflow: hidden;
}
.editor-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 18px; }
.hint { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--secondary); }
.cursor { color: var(--tertiary, var(--secondary)); }
.angle-picker { display: inline-flex; padding: 3px; border-radius: 999px; background: color-mix(in srgb, var(--text, #222) 6%, transparent); }
.angle-picker button { padding: 5px 14px; border-radius: 999px; font-size: 12px; font-weight: 500; color: var(--text); }
.angle-picker button.active { background: var(--accent); color: #fff; }
.stage { position: relative; height: 420px; user-select: none; }
.photo-layer {
  position: absolute; inset: 0; overflow: hidden;
  background: linear-gradient(135deg, color-mix(in srgb, var(--text, #222) 1.5%, transparent), color-mix(in srgb, var(--accent) 4.5%, transparent));
}
.photo { position: absolute; left: v-bind('layout.x + "px"'); top: v-bind('layout.y + "px"'); width: v-bind('layout.photoWidth + "px"'); height: v-bind('layout.photoHeight + "px"'); object-fit: contain; pointer-events: none; }
.hotspot {
  position: absolute; width: 24px; height: 24px; margin: -12px 0 0 -12px; padding: 0; border-radius: 50%;
  background: #fff; border: 2px solid var(--accent); box-shadow: 0 2px 4px rgba(0, 0, 0, 0.22);
  display: flex; align-items: center; justify-content: center; z-index: 3;
}
.hotspot.configured { background: var(--accent); border-color: #fff; color: #fff; }
.hotspot.selected { box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 22%, transparent), 0 2px 4px rgba(0, 0, 0, 0.22); transform: scale(1.08); }
.check { font-size: 11px; font-weight: 700; line-height: 1; }
.view-caption {
  position: absolute; left: 50%; bottom: 12px; transform: translateX(-50%);
  padding: 5px 12px; border-radius: 999px; font-size: 11px; color: var(--secondary);
  background: color-mix(in srgb, var(--surface) 82%, transparent); backdrop-filter: blur(10px); white-space: nowrap;
}
.dismiss { position: absolute; inset: 0; z-index: 2; }
.inspector {
  position: absolute; z-index: 4; max-height: calc(100% - 20px); overflow: auto; padding: 16px;
  display: flex; flex-direction: column; gap: 9px; border-radius: 12px;
  background: var(--surface); border: 0.5px solid var(--line);
  box-shadow: 0 14px 40px rgba(0, 0, 0, 0.24);
}
.inspector-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
.inspector-head strong { display: block; font-size: 14px; font-weight: 600; }
.inspector-head small { display: block; margin-top: 3px; font-size: 11px; color: var(--secondary); line-height: 1.4; }
.icon-button { width: 22px; height: 22px; border-radius: 6px; color: var(--secondary); font-size: 12px; }
.icon-button:hover { background: var(--control); }
.segmented { display: inline-flex; padding: 2px; border-radius: 7px; background: var(--control); }
.segmented button { flex: 1; padding: 4px 8px; border-radius: 5px; font-size: 11px; color: var(--secondary); }
.segmented button.active { background: var(--surface); color: var(--text); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12); }
.section-label { font-size: 11px; color: var(--secondary); }
.action-choice {
  display: flex; align-items: center; justify-content: space-between; gap: 8px; width: 100%;
  min-height: 32px; padding: 6px 10px; border-radius: 8px; font-size: 12px; text-align: left;
  background: color-mix(in srgb, var(--accent) 6%, transparent); border: 1px solid color-mix(in srgb, var(--accent) 14%, transparent);
}
.action-choice span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chevron { flex: none; color: var(--secondary); font-size: 10px; }
.muted { font-size: 11px; line-height: 1.45; color: var(--secondary); }
.accent-line { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--accent); }
.inline-actions { display: flex; align-items: center; gap: 10px; }
.link-button { align-self: flex-start; font-size: 11px; color: var(--accent); }
.link-button:hover { text-decoration: underline; }
.editor-foot { padding: 14px 18px; border-top: 0.5px solid var(--line); font-size: 11px; color: var(--secondary); }
</style>

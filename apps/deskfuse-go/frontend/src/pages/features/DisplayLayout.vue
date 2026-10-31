<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import Icon from './Icon.vue'

// 对应 DisplayLayout：屏幕坐标全部来自引擎，画布不编造拓扑。
// 虚线屏幕表示已保留但未就绪；拖动松手后提交 input.moveDevice，快照确认后才算生效。
interface Display {
  id: string
  deviceId: string
  name: string
  x: number
  y: number
  width: number
  height: number
  ready: boolean
  local: boolean
  wallpaperJpeg?: string | null
}

const props = defineProps<{
  displays: Display[]
  devices: any[]
  participants: string[]
  localId: string
  localName: string
  reconnecting: Set<string>
  editingDisabled?: boolean
  enabled?: boolean
}>()
const emit = defineEmits<{
  (e: 'move', payload: { deviceId: string; displayId: string; x: number; y: number }): void
  (e: 'reconnect', deviceId: string): void
  (e: 'remove', deviceId: string): void
  (e: 'error', message: string): void
}>()

const palette = ['var(--accent)', '#af52de', '#30b0c7', '#ff9500', '#ff2d55', '#5856d6', '#34c759', '#a2845e']
const canvas = ref<HTMLElement | null>(null)
const size = ref({ width: 0, height: 0 })
const observer = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(() => measure()) : null
function measure() {
  const el = canvas.value
  if (el) size.value = { width: el.clientWidth, height: el.clientHeight }
}
watch(canvas, (el, old) => {
  if (old) observer?.unobserve(old)
  if (el) {
    observer?.observe(el)
    measure()
  }
})
onBeforeUnmount(() => observer?.disconnect())

const sorted = computed(() => [...props.displays].sort((a, b) => layoutId(a).localeCompare(layoutId(b))))
const layoutId = (d: Display) => `${d.deviceId}:${d.id}`

interface Viewport { scale: number; ox: number; oy: number; lx: number; ly: number }
function viewportFor(displays: Display[]): Viewport {
  const minX = Math.min(...displays.map((d) => d.x), 0)
  const minY = Math.min(...displays.map((d) => d.y), 0)
  const lx = displays.length ? Math.min(...displays.map((d) => d.x)) : minX
  const ly = displays.length ? Math.min(...displays.map((d) => d.y)) : minY
  const maxX = displays.length ? Math.max(...displays.map((d) => d.x + d.width)) : 1
  const maxY = displays.length ? Math.max(...displays.map((d) => d.y + d.height)) : 1
  const scale = Math.max(0.001, Math.min((size.value.width - 64) / Math.max(1, maxX - lx), (size.value.height - 64) / Math.max(1, maxY - ly)))
  return {
    scale,
    lx,
    ly,
    ox: (size.value.width - (maxX - lx) * scale) / 2,
    oy: (size.value.height - (maxY - ly) * scale) / 2,
  }
}

// 拖动期间冻结视口与屏幕集合，避免快照到达或窗口缩放让屏幕在手下跳动。
const dragging = ref<string | null>(null)
const frozen = ref<{ displays: Display[]; viewport: Viewport } | null>(null)
const delta = ref({ x: 0, y: 0 })
const snapped = ref(false)
const guideX = ref<number | null>(null)
const guideY = ref<number | null>(null)
const pending = ref<{ token: number; before: Display[]; deviceId: string; dx: number; dy: number } | null>(null)
const hovered = ref<string | null>(null)

const shown = computed(() => (frozen.value ? frozen.value.displays : sorted.value))
const view = computed(() => frozen.value?.viewport ?? viewportFor(shown.value))

function pos(d: Display) {
  const v = view.value
  const moving = dragging.value === d.deviceId ? delta.value : { x: 0, y: 0 }
  return { x: v.ox + (d.x - v.lx) * v.scale + moving.x, y: v.oy + (d.y - v.ly) * v.scale + moving.y }
}

function deviceOf(d: Display) {
  return props.devices.find((x) => x.id === d.deviceId)
}
function deviceName(d: Display): string {
  if (d.local) return props.localName
  return deviceOf(d)?.name ?? `设备 ${d.deviceId.slice(0, 8)}`
}
function isOffline(d: Display): boolean {
  if (d.local) return !d.ready
  return !(deviceOf(d)?.connected === true) || !d.ready
}
function statusOf(d: Display): { label: string; cls: string } {
  if (d.local) return d.ready ? { label: '本机', cls: 'local' } : { label: '已断开', cls: 'off' }
  return !isOffline(d) ? { label: '已连接', cls: 'on' } : { label: '已断开', cls: 'off' }
}
const sizeLabel = (d: Display) => `${Math.trunc(d.width)} × ${Math.trunc(d.height)} pt`
function color(d: Display): string {
  const ids = [...new Set(props.displays.map((x) => x.deviceId))].sort()
  const slots = Math.max(palette.length, ids.length)
  const used = new Set<number>()
  for (const id of ids) {
    // 与 DisplayPalette 相同的 64 位 FNV-1a，保证 Web 与原生给设备分配同一身份色。
    let h = 14695981039346656037n
    for (let i = 0; i < id.length; i++) h = ((h ^ BigInt(id.charCodeAt(i))) * 1099511628211n) & 0xffffffffffffffffn
    let index = Number(h % BigInt(slots))
    while (used.has(index)) index = (index + 1) % slots
    used.add(index)
    if (id === d.deviceId) {
      const hue = ((index * 0.61803398875) % 1) * 360
      return index < palette.length ? palette[index] : `hsl(${hue} 75% 56%)`
    }
  }
  return 'var(--secondary)'
}
function wallpaper(d: Display): string | null {
  const data = d.wallpaperJpeg
  if (!data || typeof data !== 'string') return null
  return data.startsWith('data:') ? data : `data:image/jpeg;base64,${data}`
}
// 与原生 showsReconnect 一致：未就绪的远端屏幕在共享开启或设备未连接时提供重连入口。
const canReconnect = (d: Display) => {
  if (d.ready || d.local) return false
  const connected = deviceOf(d)?.connected === true
  return Boolean(props.enabled) || !connected
}
const isReconnecting = (id: string) => props.reconnecting.has(id) || String(props.devices.find((d) => d.id === id)?.detail ?? '').includes('正在连接')

// 等待对端上报屏幕的参与设备。
const awaiting = computed(() => {
  const present = new Set(props.displays.map((d) => d.deviceId))
  return props.participants
    .filter((id) => id !== props.localId && !present.has(id))
    .map((id) => props.devices.find((d) => d.id === id))
    .filter(Boolean)
})

let origin = { x: 0, y: 0 }
function down(event: PointerEvent, d: Display) {
  if (props.editingDisabled || pending.value || event.button !== 0) return
  origin = { x: event.clientX, y: event.clientY }
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture(event.pointerId)
  armed = { deviceId: d.deviceId }
}
let armed: { deviceId: string } | null = null

function move(event: PointerEvent) {
  if (!armed) return
  const dx = event.clientX - origin.x
  const dy = event.clientY - origin.y
  if (!dragging.value) {
    if (Math.hypot(dx, dy) < 4) return
    frozen.value = { displays: sorted.value.map((d) => ({ ...d })), viewport: viewportFor(sorted.value) }
    dragging.value = armed.deviceId
  }
  latest = { dx, dy }
  if (!frame) {
    frame = requestAnimationFrame(() => {
      frame = 0
      if (latest) void update(latest.dx, latest.dy, false)
    })
  }
}
let latest: { dx: number; dy: number } | null = null
let frame = 0
let generation = 0

/** 吸附与对齐由引擎计算；页面只把冻结的屏幕集合和逻辑位移交给它。 */
async function snapOffset(mx: number, my: number, snapDistance: number, attach: boolean): Promise<{ x: number; y: number }> {
  const f = frozen.value
  if (!f || !dragging.value) return { x: mx, y: my }
  const result = await backend.invoke('input.previewDisplayMove', {
    displays: f.displays.map((d) => ({
      id: d.id, device_id: d.deviceId, name: d.name,
      x: d.x, y: d.y, width: d.width, height: d.height, local: d.local, ready: d.ready,
    })),
    device_id: dragging.value,
    delta_x: mx, delta_y: my,
    snap_distance: snapDistance, attach,
  })
  const x = Number(result?.x)
  const y = Number(result?.y)
  return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : { x: mx, y: my }
}

async function update(dx: number, dy: number, attach: boolean) {
  const f = frozen.value
  if (!f || !dragging.value) return
  const request = ++generation
  const device = dragging.value
  const v = f.viewport
  let offset = { x: dx / v.scale, y: dy / v.scale }
  try {
    offset = await snapOffset(offset.x, offset.y, 14 / v.scale, attach)
  } catch (error) {
    if (request === generation) emit('error', error instanceof Error ? error.message : String(error))
  }
  if (request !== generation || !frozen.value || dragging.value !== device) return
  delta.value = { x: offset.x * v.scale, y: offset.y * v.scale }
  snapped.value = Math.abs(delta.value.x - dx) > 0.1 || Math.abs(delta.value.y - dy) > 0.1
  updateSnapGuides(offset.x, offset.y)
}

/** 与原生 updateSnapGuides 相同的贴合线：只画真正贴到静止屏幕边缘的那一条。 */
function updateSnapGuides(mx: number, my: number) {
  const f = frozen.value
  guideX.value = null
  guideY.value = null
  if (!f || !snapped.value) return
  const v = f.viewport
  const movers = f.displays.filter((d) => d.deviceId === dragging.value)
  const fixed = f.displays.filter((d) => d.deviceId !== dragging.value)
  for (const m of movers) {
    for (const s of fixed) {
      const left = m.x + mx
      const right = left + m.width
      const seam = Math.abs(left - (s.x + s.width)) < 0.5 ? left : Math.abs(right - s.x) < 0.5 ? right : null
      if (seam !== null) guideX.value = v.ox + (seam - v.lx) * v.scale
      const top = m.y + my
      const bottom = top + m.height
      const vertical = Math.abs(top - (s.y + s.height)) < 0.5 ? top : Math.abs(bottom - s.y) < 0.5 ? bottom : null
      if (vertical !== null) guideY.value = v.oy + (vertical - v.ly) * v.scale
    }
  }
}

let counter = 0
let timer: ReturnType<typeof setTimeout> | undefined
function up(event: PointerEvent, d: Display) {
  const wasArmed = armed
  armed = null
  if (!wasArmed || dragging.value !== d.deviceId || !frozen.value) {
    return
  }
  const v = frozen.value.viewport
  const anchor = frozen.value.displays.find((x) => layoutId(x) === layoutId(d))
  if (!anchor || props.editingDisabled) return reset()
  // 松手时用引擎的 attach 模式再吸附一次，避免最后一帧还在动画队列里。
  const fallback = { x: delta.value.x / v.scale, y: delta.value.y / v.scale }
  const token = ++counter
  pending.value = { token, before: frozen.value.displays, deviceId: d.deviceId, dx: fallback.x, dy: fallback.y }
  void (async () => {
    let dxl = fallback.x
    let dyl = fallback.y
    try {
      const offset = await snapOffset(fallback.x, fallback.y, 14 / v.scale, true)
      dxl = offset.x
      dyl = offset.y
    } catch { /* 引擎不可用时保留当前位移，提交动作会给出真实错误 */ }
    if (pending.value?.token !== token) return
    pending.value = { token, before: pending.value.before, deviceId: d.deviceId, dx: dxl, dy: dyl }
    emit('move', { deviceId: d.deviceId, displayId: d.id, x: anchor.x + dxl, y: anchor.y + dyl })
  })()
  clearTimeout(timer)
  timer = setTimeout(() => {
    if (pending.value?.token !== token) return
    reset()
    emit('error', '屏幕布局更新尚未确认，请重试')
  }, 3000)
}

function reset() {
  generation += 1
  latest = null
  if (frame) { cancelAnimationFrame(frame); frame = 0 }
  dragging.value = null
  frozen.value = null
  delta.value = { x: 0, y: 0 }
  snapped.value = false
  guideX.value = null
  guideY.value = null
  pending.value = null
  armed = null
  clearTimeout(timer)
}
defineExpose({ reset })

// 提交回执或仅就绪变化不算布局生效：比较整套排布，允许引擎重新平移原点。
watch(
  () => props.displays,
  (after) => {
    const p = pending.value
    if (!p) return
    const before = [...p.before].sort((a, b) => layoutId(a).localeCompare(layoutId(b)))
    const next = [...after].sort((a, b) => layoutId(a).localeCompare(layoutId(b)))
    if (!before.length || before.length !== next.length) return
    const same = before.every((b, i) => layoutId(b) === layoutId(next[i]) && b.width === next[i].width && b.height === next[i].height)
    if (!same) return
    const expected = (d: Display) => ({ x: d.x + (d.deviceId === p.deviceId ? p.dx : 0), y: d.y + (d.deviceId === p.deviceId ? p.dy : 0) })
    const base = expected(before[0])
    const ok = before.every((b, i) => {
      const want = expected(b)
      return Math.abs(want.x - base.x - (next[i].x - next[0].x)) < 0.5 && Math.abs(want.y - base.y - (next[i].y - next[0].y)) < 0.5
    })
    if (ok) reset()
  },
  { deep: true },
)
onBeforeUnmount(reset)
</script>

<template>
  <div ref="canvas" class="canvas" role="group" aria-label="屏幕排布">
    <div v-if="shown.length === 0" class="empty-state">
      <Icon name="display" :size="40" />
      <h3 class="df-title">等待显示器布局</h3>
      <p class="df-secondary">设备连接后显示真实布局。</p>
    </div>

    <div v-if="guideX !== null && dragging" class="guide v" :style="{ left: `${guideX}px` }" />
    <div v-if="guideY !== null && dragging" class="guide h" :style="{ top: `${guideY}px` }" />

    <div
      v-for="(d, index) in shown"
      :key="layoutId(d)"
      class="tile"
      :class="{ offline: isOffline(d), dragging: dragging === d.deviceId, notready: !d.ready, hover: hovered === layoutId(d) && !dragging }"
      :style="{ left: `${pos(d).x}px`, top: `${pos(d).y}px`, width: `${d.width * view.scale}px`, height: `${d.height * view.scale}px`, '--outline': color(d) }"
      role="img"
      :aria-label="`屏幕 ${index + 1}，${d.name}，${sizeLabel(d)}，${d.ready ? '已就绪' : '未就绪'}`"
      @pointerdown="down($event, d)"
      @pointermove="move"
      @pointerup="up($event, d)"
      @pointercancel="reset"
      @pointerenter="hovered = layoutId(d)"
      @pointerleave="hovered === layoutId(d) && (hovered = null)"
    >
      <img v-if="wallpaper(d)" class="wall" :src="wallpaper(d)!" alt="" draggable="false" />
      <div v-else class="wall fallback" />
      <span v-if="isOffline(d)" class="offline-tag"><Icon name="wifi-off" :size="12" /> {{ d.ready ? '已断开' : '等待连接' }}</span>

      <div class="info" :class="{ compact: d.width * view.scale < 180 || d.height * view.scale < 96 }">
        <Icon :name="/iphone|ipad|手机/i.test(deviceName(d)) ? 'phone' : /windows|desktop/i.test(deviceName(d)) ? 'monitor' : 'laptop'" :size="11" />
        <i class="dot" :class="statusOf(d).cls" />
        <span class="info-text">
          <span class="df-ellipsis">{{ deviceName(d) }}</span>
          <span v-if="hovered === layoutId(d) || !(d.width * view.scale < 180 || d.height * view.scale < 96)" class="res df-ellipsis">{{ sizeLabel(d) }}</span>
        </span>
        <span v-if="hovered === layoutId(d) && !(d.width * view.scale < 180 || d.height * view.scale < 96)" class="res">{{ statusOf(d).label }}</span>
      </div>

      <div v-if="hovered === layoutId(d) && !dragging" class="hover-card">
        <div class="df-caption strong">屏幕 {{ String(index + 1).padStart(2, '0') }} · {{ d.local ? '本机' : '远端' }}</div>
        <div class="df-detail strong">{{ d.name }}</div>
        <div class="df-caption">{{ sizeLabel(d) }} · {{ d.ready ? '已就绪' : '未就绪' }}</div>
      </div>

      <div v-if="canReconnect(d)" class="reconnect-wrap">
        <button type="button" class="reconnect" :disabled="editingDisabled || isReconnecting(d.deviceId)" title="重新连接此设备，并通知对端准备键鼠共享" @pointerdown.stop @click="emit('reconnect', d.deviceId)">
          <span v-if="isReconnecting(d.deviceId)" class="spinner" />
          <Icon v-else name="refresh" :size="12" />
          {{ isReconnecting(d.deviceId) ? '正在连接' : '重连' }}
        </button>
      </div>
      <button v-if="!d.local" type="button" class="remove" title="移除此设备的屏幕" :aria-label="`移除 ${deviceName(d)} 的屏幕`" :disabled="editingDisabled" @pointerdown.stop @click="emit('remove', d.deviceId)">
        <Icon name="x" :size="12" />
      </button>
    </div>

    <div v-if="awaiting.length && !dragging" class="awaiting">
      <div v-for="device in awaiting" :key="device.id" class="await-row">
        <Icon name="alert" :size="14" class="df-tone-warning" />
        <span class="await-text">
          <span class="df-detail strong">{{ device.name }} · {{ isReconnecting(device.id) ? '正在连接…' : '等待屏幕信息' }}</span>
          <span v-if="device.inputReason" class="df-caption df-secondary">{{ device.inputReason }}</span>
        </span>
        <button type="button" class="button" :disabled="editingDisabled || isReconnecting(device.id)" @click="emit('reconnect', device.id)"><Icon name="refresh" :size="12" /> 重新连接</button>
        <button type="button" class="icon-button" :disabled="editingDisabled" :aria-label="`移除 ${device.name} 的屏幕`" :title="`从屏幕排布中移除 ${device.name}`" @click="emit('remove', device.id)"><Icon name="x" :size="12" /></button>
      </div>
    </div>

    <div v-if="dragging" class="hint">{{ pending ? '正在保存布局…' : snapped ? '已吸附边缘' : '松手自动贴合相邻屏幕' }}</div>
  </div>
</template>

<style scoped>
.canvas {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 280px;
  overflow: hidden;
  border-radius: 12px;
  background-image: radial-gradient(circle, color-mix(in srgb, var(--secondary) 24%, transparent) 0.75px, transparent 1px);
  background-size: 24px 24px;
  background-position: 16px 16px;
  touch-action: none;
}
.canvas > .empty-state { position: absolute; inset: 0; display: grid; place-content: center; justify-items: center; gap: 8px; color: var(--secondary); text-align: center; }
.empty-state .df-title { color: var(--text); margin: 0; }
.empty-state p { margin: 0; }
.tile {
  position: absolute;
  border-radius: 4px;
  overflow: hidden;
  cursor: grab;
  box-shadow: 0 2px 3px rgba(0, 0, 0, 0.08);
  user-select: none;
  transition: box-shadow 0.15s ease, transform 0.15s ease;
}
.tile.hover { box-shadow: 0 2px 8px rgba(0, 0, 0, 0.18); transform: scale(1.01); }
.tile.dragging { cursor: grabbing; transition: none; z-index: 5; }
.tile::after {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: 4px;
  pointer-events: none;
  background: color-mix(in srgb, var(--outline) 2.5%, transparent);
  border: 1px solid color-mix(in srgb, var(--text) 12%, transparent);
}
.tile.notready::after { border-style: dashed; border-width: 1px; background: color-mix(in srgb, var(--outline) 6%, transparent); border-color: color-mix(in srgb, var(--text) 35%, transparent); }
.tile.dragging::after { border-width: 2px; border-color: color-mix(in srgb, var(--text) 35%, transparent); background: color-mix(in srgb, var(--outline) 9%, transparent); }
.wall { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; pointer-events: none; }
.wall.fallback { background: linear-gradient(135deg, color-mix(in srgb, var(--outline) 30%, transparent), color-mix(in srgb, var(--outline) 15%, transparent)); }
.tile.offline .wall { filter: grayscale(1); }
.tile.offline::before { content: ''; position: absolute; inset: 0; background: rgba(0, 0, 0, 0.24); z-index: 1; pointer-events: none; }
.offline-tag { position: absolute; top: 8px; right: 8px; z-index: 2; display: inline-flex; align-items: center; gap: 4px; padding: 5px 8px; border-radius: 999px; background: rgba(0, 0, 0, 0.48); color: #fff; font-size: 11px; font-weight: 600; pointer-events: none; }
.info { position: absolute; left: 7px; bottom: 7px; z-index: 3; display: flex; align-items: center; gap: 6px; max-width: calc(100% - 14px); padding: 5px 8px; border-radius: 999px; background: rgba(0, 0, 0, 0.58); border: 0.5px solid rgba(255, 255, 255, 0.18); color: #fff; font-size: 11px; font-weight: 600; pointer-events: none; }
.info-text { display: grid; min-width: 0; }
.res { opacity: 0.8; font-weight: 400; white-space: nowrap; }
.dot { width: 6px; height: 6px; border-radius: 50%; flex: none; background: var(--secondary); }
.dot.local { background: var(--accent); }
.dot.on { background: var(--success, #34c759); }
.hover-card { position: absolute; top: 8px; left: 8px; z-index: 3; padding: 7px 10px; border-radius: 8px; background: color-mix(in srgb, var(--surface) 92%, transparent); border: 1px solid color-mix(in srgb, var(--outline) 35%, transparent); pointer-events: none; display: grid; gap: 3px; backdrop-filter: blur(10px); }
.strong { font-weight: 600; }
.reconnect-wrap { position: absolute; inset: 0; z-index: 4; display: grid; place-items: center; pointer-events: none; }
.reconnect { pointer-events: auto; display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border: 0; border-radius: 999px; color: #fff; font: inherit; font-size: 12px; font-weight: 600; background: color-mix(in srgb, var(--outline) 90%, transparent); cursor: pointer; }
.reconnect:disabled { opacity: 0.78; cursor: default; }
.remove { position: absolute; top: 6px; left: 6px; z-index: 4; display: none; width: 22px; height: 22px; border: 0; border-radius: 50%; background: rgba(0, 0, 0, 0.55); color: #fff; place-items: center; cursor: pointer; }
.tile.hover .remove { display: grid; }
.guide { position: absolute; z-index: 6; background: color-mix(in srgb, var(--accent) 78%, transparent); pointer-events: none; }
.guide.v { top: 0; bottom: 0; width: 1px; }
.guide.h { left: 0; right: 0; height: 1px; }
.hint { position: absolute; top: 12px; left: 12px; z-index: 7; padding: 8px 12px; border-radius: 999px; font-size: 11px; background: color-mix(in srgb, var(--surface) 90%, transparent); border: 0.5px solid var(--line); pointer-events: none; }
.awaiting { position: absolute; left: 12px; bottom: 12px; z-index: 8; display: grid; gap: 6px; }
.await-row { display: flex; align-items: center; gap: 8px; max-width: 520px; padding: 7px 10px; border-radius: 8px; background: color-mix(in srgb, var(--surface) 92%, transparent); border: 0.5px solid var(--line); backdrop-filter: blur(10px); }
.await-text { display: grid; flex: 1; min-width: 0; }
.spinner { width: 12px; height: 12px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, 0.4); border-top-color: #fff; animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
</style>

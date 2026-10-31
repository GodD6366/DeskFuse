<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { backend, errorMessage } from '../services/backend'
import { friendlyError, invokeAction, milliseconds } from './features/core'
import Toggle from './features/Toggle.vue'
import Notice from './features/Notice.vue'
import DisplayLayout from './features/DisplayLayout.vue'
import ConfirmDialog from './features/ConfirmDialog.vue'
import FeatureGate from './features/FeatureGate.vue'
import Icon from './features/Icon.vue'
import InputLatencyChart from './features/InputLatencyChart.vue'

const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ (e: 'navigate', page: string): void }>()

const busy = ref(false)
const error = ref('')
const errorRaw = ref('')
const showAdd = ref(false)
const showWifi = ref(false)
const showQuality = ref(false)
const openSuggestionId = ref<string | null>(null)
const wifiAnchor = ref({ top: 0, left: 0 })
const qualityAnchor = ref({ top: 0, left: 0 })
const reconnecting = ref(new Set<string>())
const timers = new Map<string, ReturnType<typeof setTimeout>>()
let pendingTimer: ReturnType<typeof setTimeout> | undefined

// 提交乐观状态后最多等 5 秒快照确认；引擎拒绝时交回快照，避免界面长期显示未生效的值。
function startPendingWindow() {
  if (pendingTimer) clearTimeout(pendingTimer)
  pendingTimer = setTimeout(() => {
    pendingSharing.value = null
    pendingParticipants.value = null
  }, 5000)
}

const input = computed(() => props.snapshot?.input ?? {})
const settings = computed(() => input.value.settings ?? {})
const devices = computed<any[]>(() => props.snapshot?.devices ?? [])
const localId = computed(() => props.snapshot?.localId ?? props.snapshot?.local?.id ?? '')
const localName = computed(() => props.snapshot?.local?.name ?? '这台 Mac')
const featureEnabled = computed(() => props.snapshot?.features?.input !== false)
const pendingSharing = ref<{ enabled: boolean; controller: boolean } | null>(null)
const pendingParticipants = ref<string[] | null>(null)
const enabled = computed(() => pendingSharing.value?.enabled ?? Boolean(settings.value.enabled))
const controller = computed(() => pendingSharing.value?.controller ?? Boolean(settings.value.controller))
const managed = computed(() => Boolean(input.value.managed))
const managedPaused = computed(() => Boolean(input.value.managed_paused ?? input.value.managedPaused))
const canConfigure = computed(() => Boolean(input.value.can_configure ?? input.value.canConfigure))
const snapshotParticipants = computed<string[]>(
  () => settings.value.participants ?? input.value.participants ?? [],
)
const participants = computed<string[]>(() => pendingParticipants.value ?? snapshotParticipants.value)
const displays = computed<any[]>(() => input.value.displays ?? [])

watch(
  [() => settings.value.enabled, () => settings.value.controller],
  ([nextEnabled, nextController]) => {
    const pending = pendingSharing.value
    if (pending && pending.enabled === Boolean(nextEnabled) && pending.controller === Boolean(nextController)) {
      pendingSharing.value = null
    }
  },
)
watch(snapshotParticipants, (next) => {
  const pending = pendingParticipants.value
  if (pending && pending.length === next.length && pending.every((id) => next.includes(id))) {
    pendingParticipants.value = null
  }
})

const layoutDisplays = computed(() =>
  displays.value.map((d: any, i: number) => ({
    id: String(d.id ?? i),
    deviceId: String(d.device_id ?? d.deviceId ?? localId.value),
    name: d.name ?? `显示器 ${i + 1}`,
    x: Number(d.x ?? 0),
    y: Number(d.y ?? 0),
    width: Number(d.width ?? 1280),
    height: Number(d.height ?? 720),
    ready: d.ready !== false,
    local: Boolean(d.local ?? ((d.device_id ?? d.deviceId) === localId.value)),
    wallpaperJpeg: wallpapers.value[String(d.id ?? i)] ?? d.wallpaper_jpeg ?? d.wallpaperJpeg,
  })),
)

interface LatencyPoint {
  second: number
  rttMs: number
  series: number
}

interface HitchTick {
  second: number
  capture: boolean
}

interface QualityStatus {
  title: string
  tone: 'ok' | 'warn' | 'bad' | 'idle'
}

interface PeerRow {
  id: string
  name: string
  ssid: string
  matches: boolean | null
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

const quality = computed<any>(() => input.value.quality ?? {})
const qualitySamples = computed<any[]>(() => {
  const samples = quality.value?.samples
  return Array.isArray(samples) ? samples : []
})
const qualitySuggestions = computed<any[]>(() => {
  const suggestions = quality.value?.suggestions
  return Array.isArray(suggestions) ? suggestions : []
})
const abilityGrade = computed(() => String(quality.value?.grade ?? ''))
const rttP50 = computed(() => finiteNumber(quality.value?.rtt_p50_ms ?? quality.value?.rttP50Ms))
const rttP95 = computed(() => finiteNumber(quality.value?.rtt_p95_ms ?? quality.value?.rttP95Ms))
const rttMax = computed(() => finiteNumber(quality.value?.rtt_max_ms ?? quality.value?.rttMaxMs))
const hitchesPerMinute = computed(
  () => finiteNumber(quality.value?.hitches_per_minute ?? quality.value?.hitchesPerMinute) ?? 0,
)
const slowInjections = computed(
  () => finiteNumber(quality.value?.slow_injections ?? quality.value?.slowInjections) ?? 0,
)
const datagramReady = computed<boolean | null>(() => {
  const value = quality.value?.datagram_ready ?? quality.value?.datagramReady
  return typeof value === 'boolean' ? value : null
})
const datagramDowngraded = computed(() => datagramReady.value === false)

// 与原生 latencyPoints 一致：只画已结算的秒（<60），相邻采样间隔超过 2 秒换段。
const latencyPoints = computed<LatencyPoint[]>(() => {
  let series = 0
  let previous: number | null = null
  const points: LatencyPoint[] = []
  for (const sample of qualitySamples.value) {
    const second = finiteNumber(sample?.second)
    const rtt = finiteNumber(sample?.rtt_ms ?? sample?.rttMs)
    if (second === null || second >= 60 || rtt === null || rtt < 0) continue
    if (previous !== null && second - previous > 2) series += 1
    previous = second
    points.push({ second, rttMs: rtt, series })
  }
  return points
})
const hitchTicks = computed<HitchTick[]>(() =>
  qualitySamples.value.flatMap((sample) => {
    const second = finiteNumber(sample?.second)
    const hitches = finiteNumber(sample?.hitches) ?? 0
    if (second === null || hitches <= 0) return []
    return [
      {
        second,
        capture: (finiteNumber(sample?.hitch_capture ?? sample?.hitchCapture) ?? 0) > 0,
      },
    ]
  }),
)
const hasSignal = computed(
  () => rttP50.value !== null || rttP95.value !== null || hitchesPerMinute.value > 0,
)
const typical = computed<number | null>(
  () => rttP50.value ?? rttP95.value ?? latencyPoints.value.at(-1)?.rttMs ?? null,
)
const latencyPeak = computed(() => {
  const values = latencyPoints.value.map((point) => point.rttMs)
  if (rttMax.value !== null && rttMax.value >= 0) values.push(rttMax.value)
  return values.filter((value) => Number.isFinite(value) && value >= 0).reduce((max, value) => Math.max(max, value), 0)
})
const showsPeak = computed(
  () =>
    latencyPeak.value > 0 &&
    (typical.value === null ||
      (latencyPeak.value > typical.value + 0.2 && latencyPeak.value > typical.value * 1.25)),
)
const latencySeriesColor = computed(() => {
  const p95 = rttP95.value ?? typical.value
  if (p95 === null) return 'var(--accent)'
  if (p95 > 50) return 'var(--danger)'
  if (p95 > 20) return 'var(--warning)'
  return 'var(--accent)'
})
// 与原生 latencyCalloutSecond 一致：点很少时标出末点或明显峰值。
const latencyCalloutSecond = computed<number | null>(() => {
  const points = latencyPoints.value
  if (points.length === 0 || points.length > 4) return null
  const last = points[points.length - 1]
  const peak = points.reduce((best, point) => (point.rttMs > best.rttMs ? point : best), points[0])
  return last.rttMs >= peak.rttMs * 0.98 ? last.second : peak.second
})

const inputError = computed(() => String(input.value.error ?? '').trim())
const inputBlockedReason = computed(() => {
  if (inputError.value) return friendlyError(inputError.value)
  const summary = String(input.value.summary ?? '')
  return summary.includes('权限') ? summary : ''
})
const pointerModeLabel = computed(() => {
  const summary = String(input.value.summary ?? '')
  if (summary.includes('相对移动')) return '相对移动'
  if (summary.includes('绝对坐标')) return '绝对坐标'
  if (summary.includes('可靠路径')) return '可靠路径'
  return '等待会话'
})
const pointerFallback = computed(() => {
  const summary = String(input.value.summary ?? '')
  const marker = ['相对移动', '绝对坐标', '可靠路径'].find((value) => summary.includes(value))
  if (!marker) return ''
  const rest = summary.slice(summary.indexOf(marker) + marker.length).trim()
  if (!rest.startsWith('·')) return ''
  return rest.slice(1).trim()
})
const qualityStatus = computed<QualityStatus>(() => {
  if (inputBlockedReason.value) return { title: '暂不可用', tone: 'warn' }
  switch (abilityGrade.value) {
    case 'smooth':
      return { title: '流畅', tone: 'ok' }
    case 'fair':
      return { title: '一般', tone: 'warn' }
    case 'poor':
      return { title: '较差', tone: 'bad' }
    default:
      return { title: hasSignal.value ? '采样较少' : '空闲', tone: 'idle' }
  }
})
const qualityTone = computed(() => qualityStatus.value.tone)
const showsQuality = computed(() => controller.value && enabled.value && !managed.value)
// 还没有任何采样时，原生位置回退成屏幕数量，不显示没有内容的流畅度胶囊。
const showQualityBadge = computed(() => showsQuality.value && qualitySamples.value.length > 0)
watch(showQualityBadge, (visible) => {
  if (!visible) closeQuality()
})
const datagramChannelDescription = computed(() =>
  datagramReady.value === null ? '尚无采样' : datagramReady.value ? '就绪' : '不可用，运动走可靠传输',
)

function hitchRate(value: number): string {
  return value === 0 ? '0' : `${value.toFixed(1)} 次/分钟`
}

function suggestionIcon(priority: unknown): string {
  const value = String(priority ?? '')
  if (value === 'info') return 'info'
  if (value === 'ok') return 'check-circle'
  return 'lightbulb'
}

function suggestionTone(priority: unknown): string {
  const value = String(priority ?? '')
  if (value === 'info') return 'neutral'
  if (value === 'ok') return 'ok'
  return 'act'
}

// 屏幕预览由原生按显示器截图；快照只带 has_wallpaper，位图按需向桥接获取。
const wallpapers = ref<Record<string, string>>({})
const wallpaperAttempts = new Map<string, number>()
const wallpaperTimers = new Set<ReturnType<typeof setTimeout>>()
let wallpaperDisposed = false
// 原生截图要经过 ScreenCaptureKit，第一次请求只会触发采集；随后几秒内重试取图。
async function loadWallpapers(list: any[]) {
  if (wallpaperDisposed) return
  for (const display of list) {
    const id = String(display.id ?? '')
    if (!id || wallpapers.value[id]) continue
    const attempts = wallpaperAttempts.get(id) ?? 0
    if (attempts >= 5) continue
    wallpaperAttempts.set(id, attempts + 1)
    const result = await invokeAction<{ image?: string | null }>('input.wallpaper', { display_id: id })
    if (wallpaperDisposed) return
    if (result.ok && result.value?.image) wallpapers.value[id] = result.value.image
    else if (attempts < 4) {
      const timer = setTimeout(() => {
        wallpaperTimers.delete(timer)
        void loadWallpapers(list)
      }, 1200)
      wallpaperTimers.add(timer)
    }
  }
}
watch(displays, (list) => void loadWallpapers(list ?? []), { immediate: true })

const wifiGranted = computed(() => {
  const items = props.snapshot?.permissions?.items ?? []
  return items.find((item: any) => item.id === 'wifi')?.granted === true
})
const observedSsid = computed(() => String(input.value.observed_ssid ?? input.value.observedSsid ?? ''))
const effectiveSsid = computed(() => String(input.value.ssid ?? ''))
const wifiSummary = computed(() => {
  if (effectiveSsid.value) return effectiveSsid.value
  if (observedSsid.value) return observedSsid.value
  return wifiGranted.value ? '未连接 Wi-Fi' : '未读取 Wi-Fi'
})
const environmentMessage = computed(() => String(input.value.environment_message ?? input.value.environmentMessage ?? ''))
const wifiRules = computed<any[]>(() => (Array.isArray(input.value.rules) ? input.value.rules : []))
const peerSsids = computed<any[]>(() => {
  const value = input.value.peer_ssids ?? input.value.peerSsids
  return Array.isArray(value) ? value : []
})
const peerReference = computed(() => effectiveSsid.value || observedSsid.value)
const peerRows = computed<PeerRow[]>(() =>
  peerSsids.value
    .map((peer: any) => {
      const id = String(peer?.device_id ?? peer?.deviceId ?? '')
      const ssid = String(peer?.ssid ?? '')
      const name =
        id === localId.value
          ? '这台 Mac'
          : (devices.value.find((device: any) => device.id === id)?.name ?? (id ? id.slice(0, 8) : '未知设备'))
      return {
        id,
        name,
        ssid,
        matches: peerReference.value ? peerReference.value === ssid : null,
      }
    })
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id)),
)
const wifiNeedsAttention = computed(
  () =>
    !wifiGranted.value ||
    Boolean(input.value.environment_conflict ?? input.value.environmentConflict) ||
    peerRows.value.some((peer) => peer.matches === false),
)
const wifiIcon = computed(() =>
  !observedSsid.value && !effectiveSsid.value && wifiGranted.value ? 'wifi-off' : 'wifi',
)
const preferredController = computed<string | null>(() => {
  const ssid = effectiveSsid.value
  if (!ssid) return null
  const rule = wifiRules.value.find((item: any) => String(item.ssid ?? '') === ssid)
  const id = String(rule?.controller ?? '')
  if (!id) return null
  if (id === localId.value) return '这台 Mac'
  return devices.value.find((device: any) => device.id === id)?.name ?? id.slice(0, 8)
})
const adoptedFromPeer = computed(() => Boolean(effectiveSsid.value) && effectiveSsid.value !== observedSsid.value)
const candidates = computed(() =>
  devices.value.filter((d: any) => d.member && !d.blocked && d.id !== localId.value),
)
const receiverTitle = computed(() => {
  if (managedPaused.value) return '已停止被控'
  if (managed.value) return '被控状态'
  if (controller.value) return '主控已自动暂停'
  return '等待主控'
})

async function invoke(action: string, params: Record<string, unknown> = {}) {
  busy.value = true
  error.value = ''
  errorRaw.value = ''
  try {
    await backend.invoke(action, params)
    return true
  } catch (e) {
    errorRaw.value = errorMessage(e)
    error.value = friendlyError(errorRaw.value)
    return false
  } finally {
    busy.value = false
  }
}

function setSharing(value: boolean) {
  pendingSharing.value = { enabled: value, controller: value }
  startPendingWindow()
  void invoke('input.setSettings', { settings: { enabled: value, controller: value } }).then((ok) => {
    if (!ok) pendingSharing.value = null
  })
}

function enableAsController() {
  // 与原生「从等待页面启用主控」一致：一次提交同时打开共享与主控。
  pendingSharing.value = { enabled: true, controller: true }
  startPendingWindow()
  const alreadyController = input.value.settings?.controller === true
  const call = alreadyController
    // 主控已自动暂停：原生发 chooseController 重新接管，而不是重发同一组开关值。
    ? invoke('input.chooseController', { id: localId.value, remember: false })
    : invoke('input.setSettings', { settings: { enabled: true, controller: true } })
  void call.then((ok) => { if (!ok) pendingSharing.value = null })
}

function reclaim() {
  void invoke('input.reclaim')
}

function setParticipant(id: string, included: boolean) {
  const next = included
    ? [...new Set([...participants.value, id])]
    : participants.value.filter((value) => value !== id)
  pendingParticipants.value = next
  startPendingWindow()
  // 原生在这里同时维护「禁止自动加入」；只改 participants 会被下一帧快照加回来。
  void invoke('input.setParticipant', { device_id: id, included }).then((ok) => {
    if (!ok) pendingParticipants.value = null
  })
}

function move(payload: { deviceId: string; displayId: string; x: number; y: number }) {
  void invoke('input.moveDevice', {
    deviceId: payload.deviceId,
    displayId: payload.displayId,
    x: payload.x,
    y: payload.y,
  })
}

function reconnect(deviceId: string) {
  if (!deviceId || reconnecting.value.has(deviceId)) return
  reconnecting.value.add(deviceId)
  const timer = setTimeout(() => reconnecting.value.delete(deviceId), 12000)
  timers.set(deviceId, timer)
  // 原生同时重新拨号并重启输入激活；只重启激活在离线设备上不会重连。
  void (async () => {
    const connected = await invoke('devices.connect', { peer_id: deviceId })
    const restarted = await invoke('input.reconnect', { device_id: deviceId })
    if (!connected || !restarted) reconnecting.value.delete(deviceId)
  })()
}

function openWifi(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  wifiAnchor.value = {
    top: rect.bottom + 8,
    left: Math.max(12, Math.min(rect.right - 380, window.innerWidth - 392)),
  }
  showQuality.value = false
  openSuggestionId.value = null
  showWifi.value = !showWifi.value
}

function openQuality(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const width = Math.min(520, window.innerWidth - 24)
  qualityAnchor.value = {
    top: rect.bottom + 8,
    left: Math.max(12, Math.min(rect.left, window.innerWidth - width - 12)),
  }
  showWifi.value = false
  showQuality.value = !showQuality.value
  if (!showQuality.value) openSuggestionId.value = null
}

function closeQuality() {
  showQuality.value = false
  openSuggestionId.value = null
}

function toggleSuggestion(id: string) {
  openSuggestionId.value = openSuggestionId.value === id ? null : id
}

function authorizeWifi() {
  void invoke('permissions.open', { permission: 'wifi', mode: 'authorize' })
}

function goDevices() {
  showAdd.value = false
  emit('navigate', 'devices')
}

onMounted(() => {
  window.addEventListener('keydown', closeOnEscape)
})
onBeforeUnmount(() => {
  wallpaperDisposed = true
  for (const timer of wallpaperTimers) clearTimeout(timer)
  wallpaperTimers.clear()
  wallpaperAttempts.clear()
  wallpapers.value = {}
  window.removeEventListener('keydown', closeOnEscape)
  timers.forEach((timer) => clearTimeout(timer))
  if (pendingTimer) clearTimeout(pendingTimer)
})
function closeOnEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  showWifi.value = false
  showQuality.value = false
  openSuggestionId.value = null
  showAdd.value = false
}
</script>

<template>
  <div class="input-page">
    <FeatureGate v-if="snapshot && !featureEnabled" page="input" title="键鼠共享" />

    <div v-else-if="!snapshot" class="empty-state">
      <Icon name="keyboard" :size="40" />
      <h2>正在读取键鼠共享状态</h2>
      <p>连接原生服务后读取屏幕排布与设备。</p>
    </div>

    <template v-else-if="canConfigure">
      <div class="input-config">
        <div class="page-header">
          <h1 class="df-title">当前屏幕排布</h1>
          <div class="header-actions">
            <label class="master"
              ><span>此 Mac 作为主控</span
              ><Toggle
                :model-value="controller && enabled"
                label="此 Mac 作为主控"
                :disabled="busy || managed"
                @update:model-value="setSharing"
            /></label>
            <button
              type="button"
              class="wifi-pill"
              title="查看 Wi-Fi 标识详情"
              :aria-label="`Wi-Fi 标识：${wifiSummary}`"
              @click="openWifi"
            >
              <Icon
                :name="wifiIcon"
                :size="12"
                :class="{ warn: wifiNeedsAttention }"
              />
              <span>{{ wifiSummary }}</span>
              <Icon
                v-if="wifiNeedsAttention"
                name="alert"
                :size="11"
                class="warn"
              />
            </button>
            <button class="button primary add" :disabled="busy" @click="showAdd = true">
              <Icon name="plus" :size="12" />添加设备
            </button>
          </div>
        </div>

        <div v-if="input.locked" class="locked">
          <Icon name="lock" :size="12" />鼠标已锁定在当前设备
        </div>

        <Notice
          v-if="error"
          tone="danger"
          title="操作未完成"
          :message="error"
          :raw="errorRaw"
          dismissible
          @dismiss="error = ''"
        />

        <section class="layout-card">
          <header class="layout-head">
            <button
              v-if="showQualityBadge"
              type="button"
              class="quality-chip"
              :class="qualityTone"
              title="流畅度：最近 60 秒往返延迟趋势，点击查看详情"
              :aria-label="`流畅度 ${qualityStatus.title}，典型往返延迟 ${milliseconds(typical)}`"
              @click="openQuality"
            >
              <InputLatencyChart
                class="quality-spark"
                :points="latencyPoints"
                :width="72"
                :height="22"
                mini
                :color="latencySeriesColor"
              />
              <span class="quality-ms">{{ milliseconds(typical) }}</span>
              <span class="quality-status"><i />{{ qualityStatus.title }}</span>
              <span
                v-if="datagramDowngraded"
                class="datagram-badge"
                title="数据报运动通道不可用，移动改走可靠传输：延迟更高，但输入不会中断。检查网络质量或重新连接设备。"
              >
                <Icon name="alert" :size="10" />已降级到可靠通道
              </span>
            </button>
            <div v-else class="screens-summary">{{ displays.length }} 个屏幕</div>
            <div class="hint">拖动屏幕以匹配实际摆放位置 · pt 为 macOS 逻辑尺寸</div>
          </header>
          <div class="canvas-wrap">
            <DisplayLayout
              :displays="layoutDisplays"
              :devices="devices"
              :participants="participants"
              :local-id="localId"
              :local-name="localName"
              :reconnecting="reconnecting"
              :editing-disabled="busy"
              :enabled="enabled"
              @move="move"
              @remove="(id) => setParticipant(id, false)"
              @reconnect="reconnect"
              @error="(message) => { error = message; errorRaw = message }"
            />
          </div>
        </section>
      </div>
    </template>

    <div v-else class="receiver-page">
      <div class="receiver">
        <Icon
          :name="managedPaused ? 'pause-circle' : 'keyboard'"
          :size="40"
          class="receiver-icon"
        />
        <h2>{{ receiverTitle }}</h2>
        <p>{{ input.summary }}</p>
        <p v-if="!managed && controller && environmentMessage" class="environment">
          {{ environmentMessage }}
        </p>
        <button
          v-if="managed"
          class="button primary"
          :disabled="busy || managedPaused"
          @click="reclaim"
        >
          停止被控
        </button>
        <button v-else class="button primary" :disabled="busy" @click="enableAsController">
          {{ controller ? '本次使用此 Mac 作为主控' : '在此 Mac 启用主控' }}
        </button>
        <button
          type="button"
          class="wifi-pill"
          title="查看 Wi-Fi 标识详情"
          :aria-label="`Wi-Fi 标识：${wifiSummary}`"
          @click="openWifi"
        >
          <Icon :name="wifiIcon" :size="12" :class="{ warn: wifiNeedsAttention }" />
          <span>{{ wifiSummary }}</span>
          <Icon v-if="wifiNeedsAttention" name="alert" :size="11" class="warn" />
        </button>
        <Notice
          v-if="error"
          tone="danger"
          title="操作未完成"
          :message="error"
          :raw="errorRaw"
          dismissible
          @dismiss="error = ''"
        />
      </div>
    </div>

    <template v-if="showQuality">
      <div class="pop-backdrop" @click="closeQuality" />
      <div
        class="quality-pop"
        role="dialog"
        aria-label="流畅度详情"
        :style="{
          top: `${qualityAnchor.top}px`,
          left: `${qualityAnchor.left}px`,
          maxHeight: `calc(100vh - ${qualityAnchor.top + 12}px)`,
        }"
      >
        <header class="quality-head">
          <span class="quality-title"><Icon name="chart" :size="14" />流畅度</span>
          <div class="quality-head-right">
            <span
              v-if="datagramDowngraded"
              class="datagram-badge"
              title="数据报运动通道不可用，移动改走可靠传输：延迟更高，但输入不会中断。检查网络质量或重新连接设备。"
            >
              <Icon name="alert" :size="10" />已降级到可靠通道
            </span>
            <span class="quality-status-pill" :class="qualityTone"><i />{{ qualityStatus.title }}</span>
          </div>
        </header>

        <div class="quality-hero">
          <strong>{{ milliseconds(typical) }}</strong>
          <span>典型往返延迟</span>
          <span class="spacer" />
          <span v-if="showsPeak" class="quality-peak">峰值 {{ milliseconds(latencyPeak) }}</span>
        </div>

        <InputLatencyChart
          class="quality-detail-chart"
          :points="latencyPoints"
          :hitches="hitchTicks"
          :width="484"
          :height="168"
          :color="latencySeriesColor"
          :callout-second="latencyCalloutSecond"
        />
        <p class="chart-note">固定刻度 0–100 ms；超过上限的点显示在上限处，数值仍为实际延迟。</p>

        <hr class="df-divider" />

        <div class="quality-metrics">
          <div class="quality-metric"><span>P50</span><strong>{{ milliseconds(rttP50) }}</strong></div>
          <div class="quality-metric"><span>P95</span><strong>{{ milliseconds(rttP95) }}</strong></div>
          <div class="quality-metric">
            <span>网络卡顿</span><strong>{{ hitchRate(hitchesPerMinute) }}</strong>
          </div>
          <div class="quality-metric"><span>慢注入</span><strong>{{ slowInjections }}</strong></div>
        </div>

        <div v-if="hitchTicks.length" class="hitch-legend">
          <span><i class="capture" />注入慢</span>
          <span><i class="network" />网络卡顿</span>
        </div>

        <div v-if="pointerFallback" class="quality-fallback">
          <Icon name="info" :size="12" /><span>{{ pointerFallback }}</span>
        </div>

        <div v-if="qualitySuggestions.length" class="quality-advice">
          <h3>优化建议</h3>
          <div v-for="suggestion in qualitySuggestions" :key="suggestion.id" class="suggestion">
            <button
              type="button"
              class="suggestion-row"
              :aria-expanded="openSuggestionId === String(suggestion.id)"
              @click="toggleSuggestion(String(suggestion.id))"
            >
              <Icon
                :name="suggestionIcon(suggestion.priority)"
                :size="13"
                class="suggestion-icon"
                :class="`tone-${suggestionTone(suggestion.priority)}`"
              />
              <span>{{ suggestion.trigger }}</span>
              <Icon class="suggestion-info" name="info" :size="12" />
            </button>
            <div v-if="openSuggestionId === String(suggestion.id)" class="suggestion-detail">
              <p class="suggestion-detail-title">{{ suggestion.trigger }}</p>
              <div class="suggestion-field"><span>做法</span><p>{{ suggestion.action }}</p></div>
              <div class="suggestion-field"><span>预期</span><p>{{ suggestion.expected }}</p></div>
              <div class="suggestion-field"><span>复测</span><p>{{ suggestion.retest }}</p></div>
            </div>
          </div>
        </div>

        <details class="quality-diagnostics">
          <summary>
            <Icon class="diagnostics-chevron" name="chevron-right" :size="11" />查看诊断详情
          </summary>
          <div class="diagnostics-body">
            <p v-if="inputBlockedReason" class="warn-text">{{ inputBlockedReason }}</p>
            <p>每秒更新；空闲时显示设备连接延迟，没有可用延迟时显示 0。</p>
            <p>当前模式：{{ pointerModeLabel }}</p>
            <p>数据报通道：{{ datagramChannelDescription }}</p>
            <p>慢注入 {{ slowInjections }} 次</p>
            <p>平台没有光标呈现时间。端到端时间是采集排队、发送、往返和注入的估计，不是两台电脑的时钟同步。</p>
            <p v-if="inputError">{{ inputError }}</p>
          </div>
        </details>

        <p class="quality-footnote">最近 60 秒 · 3 秒平滑趋势 · 指标与峰值为原始采样</p>
      </div>
    </template>

    <template v-if="showWifi">
      <div class="pop-backdrop" @click="showWifi = false" />
      <div
        class="wifi-pop"
        role="dialog"
        aria-label="Wi-Fi 标识"
        :style="{ top: `${wifiAnchor.top}px`, left: `${wifiAnchor.left}px` }"
      >
        <div class="wifi-title"><Icon name="wifi" :size="14" />Wi-Fi 标识</div>
        <div class="wifi-grid">
          <span class="wifi-label">本机读取</span>
          <div class="wifi-value">
            <span v-if="observedSsid" class="mono">{{ observedSsid }}</span>
            <span v-else-if="wifiGranted" class="secondary"
              >未连接 Wi-Fi；有线网络会采用对端上报的标识</span
            >
            <span v-else class="warn-text"
              >未读取：需要定位授权才能读取 Wi-Fi 名称
              <button type="button" class="df-link" @click="authorizeWifi">前往授权</button></span
            >
          </div>
          <span class="wifi-label">场景判定</span>
          <div class="wifi-value">
            <template v-if="effectiveSsid">
              <span class="mono">{{ effectiveSsid }}</span>
              <span v-if="adoptedFromPeer" class="secondary note">采用对端上报</span>
            </template>
            <span v-else class="secondary">未确定</span>
          </div>
          <span class="wifi-label">首选主控</span>
          <div class="wifi-value">{{ preferredController ?? '该网络未保存主控' }}</div>
          <span class="wifi-label">对端上报</span>
          <div class="wifi-value peer-list">
            <template v-if="peerRows.length">
              <div v-for="peer in peerRows" :key="peer.id" class="peer-row">
                <span>{{ peer.name }}</span>
                <span class="mono">{{ peer.ssid }}</span>
                <span
                  v-if="peer.matches !== null"
                  class="match"
                  :class="peer.matches ? 'df-tone-success' : 'df-tone-warning'"
                >
                  <Icon :name="peer.matches ? 'check-circle' : 'alert'" :size="10" />
                  {{ peer.matches ? '一致' : '不一致' }}
                </span>
              </div>
            </template>
            <span v-else class="secondary">暂无；对端需与本机建立输入链路后才会上报</span>
          </div>
        </div>
        <p v-if="environmentMessage" class="wifi-message">{{ environmentMessage }}</p>
      </div>
    </template>

    <ConfirmDialog
      :open="showAdd"
      title="添加设备屏幕"
      :width="500"
      hide-confirm
      cancel-title="完成"
      @cancel="showAdd = false"
    >
      <p class="dialog-desc">
        选择已配对的电脑，添加它的全部屏幕。屏幕尺寸由对端提供，连接就绪后才能跨屏操作。
      </p>
      <Notice
        v-if="error"
        tone="danger"
        title="操作未完成"
        :message="error"
        :raw="errorRaw"
      />
      <div v-if="!candidates.length" class="dialog-empty">
        <Icon name="display" :size="36" />
        <h3>还没有可添加的设备</h3>
        <p>先到设备页配对另一台电脑，再回来添加屏幕。</p>
      </div>
      <div v-else class="candidate-list">
        <div v-for="device in candidates" :key="device.id" class="candidate">
          <Icon name="display" :size="18" class="candidate-icon" />
          <div class="candidate-text">
            <strong>{{ device.name || '未命名设备' }}</strong>
            <small>{{ device.input_reason || (device.connected ? '在线' : '离线') }}</small>
          </div>
          <button
            v-if="participants.includes(device.id)"
            class="button"
            :disabled="busy"
            @click="setParticipant(device.id, false)"
          >
            移除
          </button>
          <button
            v-else
            class="button"
            :disabled="busy"
            @click="setParticipant(device.id, true)"
          >
            添加
          </button>
        </div>
      </div>
      <div class="dialog-footer">
        <button type="button" class="button" @click="goDevices">
          <Icon name="display" :size="13" />前往设备页配对
        </button>
        <span v-if="busy" class="spinner" aria-label="正在保存" />
      </div>
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.input-page {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  position: relative;
}

.input-config {
  height: 100%;
  min-height: 0;
  overflow: auto;
  padding: 4px 24px 24px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 32px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.master {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
}

.wifi-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  max-width: 200px;
  padding: 0 10px;
  border-radius: 999px;
  background: var(--control);
  color: var(--text);
  font-size: 12px;
  font-weight: 500;
}

.wifi-pill > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 160px;
}

.wifi-pill > svg {
  color: var(--secondary);
}

.wifi-pill > svg.warn {
  color: var(--warning);
}

.warn-text .df-link {
  margin-left: 6px;
}

.add {
  min-height: 28px;
  height: 28px;
  padding: 0 12px;
  font-size: 12px;
}

.locked {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--warning);
}

.layout-card {
  flex: 1;
  min-height: 320px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 12px;
}

.layout-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
}

.quality-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 32px;
  padding: 5px 10px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text) 4%, transparent);
  color: var(--secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.quality-chip:hover {
  background: color-mix(in srgb, var(--text) 7%, transparent);
}
.quality-spark {
  width: 72px;
  height: 22px;
  flex: none;
}
.quality-ms {
  font-weight: 600;
  color: var(--text);
}
.quality-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-weight: 600;
}
.quality-status i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.quality-chip.ok .quality-status { color: var(--success); }
.quality-chip.warn .quality-status { color: var(--warning); }
.quality-chip.bad .quality-status { color: var(--danger); }
.quality-chip.idle .quality-status { color: var(--secondary); }

.datagram-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--warning) 12%, transparent);
  color: var(--warning);
  font-size: 11px;
  font-weight: 500;
  white-space: nowrap;
}

.screens-summary {
  font-size: 12px;
  font-weight: 500;
  color: var(--secondary);
  white-space: nowrap;
}

.quality-pop {
  position: fixed;
  z-index: 60;
  width: 520px;
  max-width: calc(100vw - 24px);
  overflow: auto;
  padding: 18px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 16px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.2);
  font-variant-numeric: tabular-nums;
}

.quality-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.quality-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
}

.quality-head-right {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 8px;
}

.quality-status-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 9px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  background: color-mix(in srgb, currentColor 12%, transparent);
  border: 0.5px solid color-mix(in srgb, currentColor 25%, transparent);
}
.quality-status-pill i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.quality-status-pill.ok { color: var(--success); }
.quality-status-pill.warn { color: var(--warning); }
.quality-status-pill.bad { color: var(--danger); }
.quality-status-pill.idle { color: var(--secondary); }

.quality-hero {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-height: 30px;
}
.quality-hero strong {
  font-size: 24px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.quality-hero > span:not(.spacer) {
  font-size: 11px;
  color: var(--secondary);
}
.quality-hero .spacer { flex: 1; }
.quality-peak { font-variant-numeric: tabular-nums; }

.quality-detail-chart {
  width: 100%;
  height: auto;
  flex: none;
}

.chart-note {
  margin-top: -6px;
  font-size: 11px;
  color: var(--secondary);
}

.quality-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 16px;
}

.quality-metric {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.quality-metric span {
  font-size: 11px;
  color: var(--secondary);
}
.quality-metric strong {
  font-size: 18px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.hitch-legend {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 11px;
  color: var(--secondary);
}
.hitch-legend span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.hitch-legend i {
  width: 3px;
  height: 8px;
  border-radius: 1px;
}
.hitch-legend i.capture { background: var(--warning); }
.hitch-legend i.network { background: var(--accent); }

.quality-fallback {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: 11px;
  color: var(--secondary);
}

.quality-advice {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.quality-advice h3 {
  font-size: 11px;
  font-weight: 600;
}
.suggestion-row {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  width: 100%;
  padding: 0;
  font-size: 11px;
  color: var(--secondary);
  text-align: left;
}
.suggestion-row > span {
  flex: 1;
  min-width: 0;
}
.suggestion-icon { margin-top: 1px; }
.suggestion-icon.tone-neutral { color: var(--secondary); }
.suggestion-icon.tone-ok { color: var(--success); }
.suggestion-icon.tone-act { color: var(--warning); }
.suggestion-info {
  margin-top: 1px;
  color: var(--secondary);
  opacity: 0.7;
}
.suggestion-detail {
  margin: 6px 0 2px;
  padding: 10px 12px;
  display: grid;
  gap: 8px;
  border-radius: 10px;
  background: var(--control);
  font-size: 11px;
}
.suggestion-detail-title {
  font-weight: 600;
  color: var(--text);
}
.suggestion-field span {
  display: block;
  margin-bottom: 2px;
  color: var(--secondary);
}
.suggestion-field p {
  color: var(--secondary);
  overflow-wrap: anywhere;
}

.quality-diagnostics {
  font-size: 11px;
  color: var(--secondary);
}
.quality-diagnostics summary {
  display: flex;
  align-items: center;
  gap: 5px;
  font-weight: 500;
  cursor: pointer;
  list-style: none;
}
.quality-diagnostics summary::-webkit-details-marker {
  display: none;
}
.diagnostics-chevron {
  transition: transform 0.12s ease;
}
.quality-diagnostics[open] .diagnostics-chevron {
  transform: rotate(90deg);
}
.diagnostics-body {
  display: grid;
  gap: 6px;
  padding-top: 6px;
}
.diagnostics-body p {
  overflow-wrap: anywhere;
}

.quality-footnote {
  font-size: 11px;
  color: var(--secondary);
  opacity: 0.75;
}

.hint {
  font-size: 11px;
  color: var(--secondary);
  text-align: right;
}

.canvas-wrap {
  flex: 1;
  min-height: 280px;
}

.receiver-page {
  flex: 1;
  min-height: 0;
  display: grid;
  place-items: center;
  padding: 28px;
  overflow: auto;
}

.receiver {
  width: min(520px, 100%);
  display: grid;
  justify-items: center;
  gap: 20px;
  text-align: center;
}

.receiver-icon {
  color: var(--secondary);
}

.receiver h2 {
  font-size: 15px;
}

.receiver p {
  font-size: 12px;
  color: var(--secondary);
}

.receiver p.environment {
  margin-top: -8px;
}

.receiver :deep(.notice) {
  width: 100%;
  text-align: left;
}

.pop-backdrop {
  position: fixed;
  inset: 0;
  z-index: 55;
}

.wifi-pop {
  position: fixed;
  z-index: 60;
  width: 380px;
  max-width: calc(100vw - 24px);
  max-height: calc(100vh - 24px);
  overflow: auto;
  padding: 16px;
  display: grid;
  gap: 12px;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 16px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.2);
}

.wifi-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
}

.wifi-grid {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr);
  gap: 8px 16px;
  align-items: start;
  font-size: 12px;
}

.wifi-label {
  color: var(--secondary);
}

.wifi-value {
  min-width: 0;
  overflow-wrap: anywhere;
}

.mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
}

.secondary {
  color: var(--secondary);
}

.note {
  margin-left: 8px;
  font-size: 11px;
}

.warn-text {
  color: var(--warning);
}

.peer-list {
  display: grid;
  gap: 4px;
}

.peer-row {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 6px;
}

.match {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 11px;
}

.wifi-message {
  font-size: 11px;
  color: var(--secondary);
}

.dialog-desc {
  font-size: 12px;
  color: var(--secondary);
}

.dialog-empty {
  display: grid;
  justify-items: center;
  gap: 6px;
  padding: 24px 12px;
  color: var(--secondary);
  text-align: center;
}

.dialog-empty h3 {
  font-size: 13px;
  color: var(--text);
}

.dialog-empty p {
  font-size: 12px;
}

.candidate-list {
  display: grid;
}

.candidate {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 0;
  border-bottom: 0.5px solid var(--line);
}

.candidate-icon {
  color: var(--secondary);
}

.candidate-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.candidate-text strong {
  font-size: 13px;
  font-weight: 600;
}

.candidate-text small {
  font-size: 11px;
}

.dialog-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 4px;
}

.spinner {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 900px) {
  .page-header {
    align-items: flex-start;
    flex-direction: column;
  }
  .header-actions {
    justify-content: flex-start;
  }
}
</style>

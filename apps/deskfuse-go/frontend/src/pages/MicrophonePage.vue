<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { backend, errorMessage } from '../services/backend'
import { friendlyError, unwrapErrorMessage } from './features/core'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import Toggle from './features/Toggle.vue'
import FeatureGate from './features/FeatureGate.vue'
import MicLinkCanvas from './features/MicLinkCanvas.vue'
import DeviceIcon from './features/DeviceIcon.vue'
import { useDeviceArt } from './features/deviceArt'

type Role = 'provider' | 'receiver' | ''
type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
type DriverAction = 'install' | 'uninstall' | 'restart-audio'

const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ (e: 'navigate', page: string): void }>()

const localId = computed(() => props.snapshot?.localId ?? props.snapshot?.local?.id ?? '')
const localName = computed(() => String(props.snapshot?.local?.name ?? '').trim() || '这台 Mac')
const mic = computed<any>(() => props.snapshot?.microphone ?? null)
const featureEnabled = computed(() => props.snapshot?.features?.microphone !== false)
const peers = computed<any[]>(() =>
  (props.snapshot?.devices ?? []).filter(
    (device: any) => device.trusted && !device.blocked && device.id !== localId.value,
  ),
)

// 页面可见时轮询轻量投影，让电平跟得上真实采集；快照仍按全局 1.5 s 刷新。
const live = ref<any>(null)
const liveView = computed<any>(() => {
  const base = mic.value ?? {}
  const next = live.value ?? {}
  return {
    ...base,
    ...next,
    settings: { ...(base.settings ?? {}), ...(next.settings ?? {}) },
    native: { ...(base.native ?? {}), ...(next.native ?? {}) },
  }
})
const settings = computed<any>(() => liveView.value.settings ?? {})
const nativeState = computed<any>(() => liveView.value.native ?? {})
// 宿主先给出驱动/配对等静态字段，worker 的会话快照随后才到；在此之前不能把
// “还没有 settings”当成“没有来源/驱动未连接”，否则冷启动会短暂误报不可用。
const micLoaded = computed(() => liveView.value.loaded !== false)
const inputs = computed<any[]>(() => liveView.value.inputs ?? [])
const readerApps = computed<any[]>(() => liveView.value.reader_apps ?? [])

const localRole = ref<Role>('')
const source = ref('')
const inputUid = ref('')
const showPicker = ref(false)
// 窄屏舞台把两端改为上下排列，链路画布也随之为纵向；桌面端始终横向。
const narrowQuery = window.matchMedia('(max-width: 760px)')
const narrowLayout = ref(narrowQuery.matches)
function onNarrowLayout(event: MediaQueryListEvent) { narrowLayout.value = event.matches }
const pendingSource = ref<string | null>(null)
const pendingInput = ref<string | null>(null)
const busy = ref(false)
const error = ref('')
const errorRaw = ref('')
const showSourcePicker = ref(false)
const driverAction = ref<DriverAction | null>(null)
const restartAudio = ref(true)
const speechText = ref('')
const speech = ref<any>({ phase: 'loading', listening: false, text: '', level: 0, input_name: null, on_device: false, problem: null })
let pendingTimer: ReturnType<typeof setTimeout> | undefined
let speechTimer: ReturnType<typeof setInterval> | undefined
let liveTimer: ReturnType<typeof setInterval> | undefined
let liveAvailable = true
let initialized = false
let lastFocus: HTMLElement | null = null
const sheetEl = ref<HTMLElement | null>(null)

const speechPhase = computed(() => String(speech.value?.phase ?? 'loading'))
const speechListening = computed(() => speech.value?.listening === true)
const speechLevel = computed(() => Math.max(0, Math.min(1, Number(speech.value?.level ?? 0))))
const speechInput = computed(() => String(speech.value?.input_name ?? '') || '系统默认输入')
const speechProblem = computed<any>(() => speech.value?.problem ?? null)
const speechStatus = computed(() => {
  switch (speechPhase.value) {
    case 'loading': return { title: '正在加载语音测试…', detail: '正在读取音频设备与识别服务，可继续操作页面。', parts: [] as string[] }
    case 'authorizing': return { title: '正在确认权限…', detail: '首次使用会请求语音识别与麦克风权限。', parts: [] as string[] }
    case 'listening': return { title: '正在聆听，再次点击停止', detail: '', parts: [`输入设备：${speechInput.value}`, speech.value?.on_device ? '本机识别' : 'Apple 语音识别', '文字不保存'] }
    default: return { title: '点击麦克风开始说话', detail: '', parts: [`输入设备：${speechInput.value}`, '中文识别', '文字不保存'] }
  }
})

function applySpeech(state: any) {
  if (!state || typeof state !== 'object') return
  speech.value = state
  if (state.listening === true) speechText.value = String(state.text ?? '')
}
async function speechInvoke(action: string, params: Record<string, unknown> = {}): Promise<any> {
  try {
    const state = await backend.invoke(action, params)
    applySpeech(state)
    return state
  } catch (e) {
    speech.value = { ...speech.value, phase: 'idle', listening: false, level: 0,
      problem: { title: '语音测试不可用', message: friendlyError(errorMessage(e)), settings_url: null } }
    return null
  }
}
function prepareSpeech() { void speechInvoke('microphone.speech.prepare') }
function toggleSpeech() { void speechInvoke('microphone.speech.toggle', { text: speechText.value }) }
function stopSpeech() { void speechInvoke('microphone.speech.stop') }
function dismissSpeechProblem() { void speechInvoke('microphone.speech.dismiss') }
function openSpeechSettings() {
  const url = String(speechProblem.value?.settings_url ?? '')
  if (url) void speechInvoke('microphone.speech.settings', { url })
}

function startSpeechPolling() {
  if (speechTimer) return
  speechTimer = setInterval(() => { void speechInvoke('microphone.speech.state') }, 120)
}
function stopSpeechPolling() {
  if (!speechTimer) return
  clearInterval(speechTimer)
  speechTimer = undefined
}
watch(speechListening, (listening) => {
  if (listening) {
    startSpeechPolling()
  } else {
    stopSpeechPolling()
    // 停止采集后识别器还会给出最后一段文字，补读一次避免丢尾词。
    void speechInvoke('microphone.speech.state')
  }
})

function startPendingWindow() {
  if (pendingTimer) clearTimeout(pendingTimer)
  pendingTimer = setTimeout(() => {
    pendingSource.value = null
    pendingInput.value = null
  }, 5000)
}

watch(
  mic,
  (state) => {
    if (!state) return
    const nextSource = String(state.settings?.source ?? '')
    const nextInput = String(state.settings?.input_uid ?? '')
    // 轮询快照可能比引擎应用配置更早返回；本地选择等快照确认后再交还。
    if (pendingSource.value !== null) {
      if (nextSource === pendingSource.value) pendingSource.value = null
    } else {
      source.value = nextSource
    }
    if (pendingInput.value !== null) {
      if (nextInput === pendingInput.value) pendingInput.value = null
    } else {
      inputUid.value = nextInput
    }
  },
  { immediate: true },
)

const resolvedRole = computed<Role>(() => {
  if (localRole.value) return localRole.value
  const state = liveView.value
  if (!state || (!state.loaded && !state.settings)) return ''
  if (String(state.settings?.source ?? '')) return 'receiver'
  const preferred = String(state.preferred_role ?? '')
  if (preferred === 'provider' || preferred === 'receiver') return preferred
  if (state.settings?.enabled || (state.settings?.allowed ?? []).length) return 'provider'
  return ''
})
const role = resolvedRole
const configured = computed(() => Boolean(role.value))

const permission = computed(() => String(nativeState.value.permission ?? ''))
const phase = computed(() => String(liveView.value.phase ?? ''))
// 功能开关只在「设置 › 通用 › 功能」；这里只读取状态用于展示。
const enabled = computed(() => Boolean(settings.value.enabled))
const driverInstalled = computed(() => Boolean(liveView.value.driver_installed))
const legacyDriverInstalled = computed(() => Boolean(liveView.value.legacy_driver_installed))
const nativeDriver = computed(() => Boolean(nativeState.value.driver))
const defaultInput = computed(() => liveView.value.is_default_input === true)
const managingDriver = computed(() => Boolean(liveView.value.managing_driver))
const onlinePeerCount = computed(() => peers.value.filter((peer) => peer.connected).length)
const sourcePeer = computed(() => peers.value.find((peer) => peer.id === source.value) ?? null)
const sourceConnected = computed(() => sourcePeer.value?.connected === true)
const level = computed(() => Math.max(0, Math.min(1, Number(nativeState.value.level ?? 0))))
const waveform = computed<number[]>(() => {
  const raw = nativeState.value.waveform
  if (!Array.isArray(raw)) return []
  return raw.map(Number).filter((value: number) => Number.isFinite(value)).slice(-512)
})
const waveformActive = computed(() => nativeState.value.waveform_active === true && waveform.value.length > 0)
// 宿主给出窗口时长与末端时延：画布按真实时间连续滚动，这两个值决定时间轴。
const waveformSpan = computed(() => {
  const span = Number(nativeState.value.waveform_span_ms ?? 0)
  return Number.isFinite(span) && span > 0 ? span : 0
})
const waveformAge = computed(() => {
  const age = Number(nativeState.value.waveform_age_ms ?? 0)
  return Number.isFinite(age) && age > 0 ? age : 0
})
const waveformRate = computed(() => {
  const rate = Number(nativeState.value.waveform_rate ?? 0)
  return Number.isFinite(rate) && rate > 0 ? rate : 48_000
})

const sourcePeerName = computed(() =>
  !source.value ? '选择来源设备' : sourcePeer.value?.name ?? '原来源设备不可用',
)
const inputDisplayName = computed(() =>
  !inputUid.value
    ? '系统默认麦克风'
    : inputs.value.find((input) => input.id === inputUid.value)?.name ?? '所选麦克风不可用',
)
// 电平以 dBFS 展示；0 值显示为 −∞，避免把“没有信号”写成 0 dB。
const levelDb = computed(() => {
  const value = level.value
  if (value <= 0.0005) return '−∞'
  const db = Math.round(20 * Math.log10(value))
  return db < 0 ? `−${-db}` : String(db)
})
const readerNames = computed(() => {
  const names = [...new Set(readerApps.value.map((app) => String(app.name ?? '').trim()).filter(Boolean))]
  return names
})
const readerTitle = computed(() => {
  if (!readerNames.value.length) return ''
  return readerNames.value.length === 1 ? readerNames.value[0] : `${readerNames.value[0]} 等 ${readerNames.value.length} 个应用`
})

// ---- 声桥：身份选择、两端设备与真实波形 ----

const activePeer = computed(() => {
  const id = String(liveView.value.peer ?? '')
  return id ? peers.value.find((peer) => peer.id === id) ?? null : null
})

const pickPeer = computed(() => {
  if (role.value === 'receiver' && sourcePeer.value) return sourcePeer.value
  return peers.value.find((peer) => peer.connected) ?? peers.value[0] ?? null
})
const pickPeerName = computed(() => String(pickPeer.value?.name ?? '').trim() || '对方的 Mac')
const peerSummary = computed(() => {
  if (!peers.value.length) return '还没有已配对的电脑'
  return `${peers.value.length} 台已配对 · ${onlinePeerCount.value} 台在线`
})
const activePeerName = computed(() =>
  String(activePeer.value?.name ?? '').trim() || String(destPeerName.value || '').trim() || '对方的 Mac',
)
const destPeerName = computed(() => {
  if (role.value === 'receiver') return localName.value
  const peer = activePeer.value ?? peers.value.find((item) => item.connected) ?? peers.value[0] ?? null
  return String(peer?.name ?? '').trim()
})
const destPeer = computed(() =>
  activePeer.value ?? peers.value.find((item) => item.connected) ?? peers.value[0] ?? null,
)

// 两端都用宿主解析的真实产品图（与设备页、原生列表同源）；对端只知道名称，
// 宿主按名称里的型号词回退，拿不到图时页面保留同形轮廓。
const localModel = computed(() => String(props.snapshot?.local?.model_identifier ?? ''))
const sourceArt = useDeviceArt(computed(() =>
  role.value === 'provider'
    ? { name: localName.value, model: localModel.value }
    : { name: String(sourcePeer.value?.name ?? ''), model: '' },
))
const destArt = useDeviceArt(computed(() =>
  role.value === 'receiver'
    ? { name: localName.value, model: localModel.value }
    : { name: String(destPeer.value?.name ?? ''), model: '' },
))

type PodFact = { text: string; tone?: 'warn' }
// 对端设备卡显示真实链路：线路类型与探测到的 RTT 比“在线”更能解释音频质量。
const peerFacts = computed<PodFact[]>(() => {
  const peer = role.value === 'receiver' ? sourcePeer.value : destPeer.value
  if (!peer) return [{ text: '等待配对设备', tone: 'warn' }]
  const routes: any[] = Array.isArray(peer.routes) ? peer.routes : []
  const route = routes.find((item) => item.active) ?? routes.find((item) => item.preferred) ?? routes[0] ?? null
  const facts: PodFact[] = []
  const path = String(route?.path ?? '').trim()
  if (path) facts.push({ text: path })
  const rtt = Number(route?.rtt_ms)
  if (Number.isFinite(rtt) && rtt > 0) facts.push({ text: `${Math.round(rtt)} ms` })
  if (facts.length) return facts
  const detail = String(peer.detail ?? '').trim()
  return [{ text: detail || (peer.connected ? '已连接' : '离线'), tone: peer.connected ? undefined : 'warn' }]
})
// 来源端写真正会挡住采集的状态：实体麦克风是否已获授权。
// 使用方本机端（去向）显示的是读取应用，所以这里只服务于提供方本机。
const localFacts = computed<PodFact[]>(() => (permission.value === 'authorized'
  ? [{ text: '麦克风已授权' }]
  : [{ text: '需要麦克风授权', tone: 'warn' }]))
const sourceFacts = computed<PodFact[]>(() => (role.value === 'provider' ? localFacts.value : peerFacts.value))
const destFacts = computed<PodFact[]>(() => (role.value === 'provider' ? peerFacts.value : []))
const headerSummary = computed(() => {
  if (!enabled.value) {
    return role.value === 'provider'
      ? '共享已暂停，开启后对方才能使用这台 Mac 的麦克风。'
      : '接收已暂停，开启后 DeskFuse Mic 才会接入远端声音。'
  }
  if (role.value === 'provider') {
    if (phase.value === 'streaming') return `${localName.value} 正在把麦克风传给 ${activePeerName.value}`
    if (!peers.value.length) return '先配对另一台 Mac，之后可直接使用这台 Mac 的麦克风。'
    return `${localName.value} 会在已配对的 Mac 读取麦克风时提供输入。`
  }
  if (phase.value === 'streaming') return `${sourcePeerName.value} 正在把麦克风传给这台 Mac`
  if (!source.value) return '选择一台已配对的 Mac 作为远端麦克风。'
  return `${sourcePeerName.value} 的麦克风会出现在 DeskFuse Mic 中。`
})
const sourcePod = computed(() => {
  if (role.value === 'provider') {
    const state = !enabled.value
      ? '已暂停'
      : permission.value !== 'authorized'
        ? permission.value === 'denied' ? '权限被关闭' : '需要权限'
        : phase.value === 'streaming' ? '采集中' : '待命'
    return {
      name: localName.value,
      detail: inputUid.value ? `${inputDisplayName.value} · 本机麦克风` : '系统默认麦克风',
      state,
      icon: 'mic',
      tone: !enabled.value || permission.value !== 'authorized' ? 'warn' : phase.value === 'streaming' ? 'live' : 'idle',
    }
  }
  const peer = sourcePeer.value
  const state = !enabled.value
    ? '已暂停'
    : !source.value
      ? '未选择来源'
      : !sourceConnected.value
        ? '等待来源'
        : phase.value === 'streaming' ? '接收中' : '待命'
  return {
    name: peer?.name || sourcePeerName.value,
    detail: peer ? `远端麦克风 · ${peer.connected ? '在线' : '离线'}` : '选择提供麦克风的 Mac',
    state,
    icon: 'mic',
    tone: !enabled.value || !sourceConnected.value ? 'warn' : phase.value === 'streaming' ? 'live' : 'idle',
  }
})
const destPod = computed(() => {
  if (role.value === 'provider') {
    const peer = destPeer.value
    const state = !enabled.value
      ? '已暂停'
      : phase.value === 'streaming' && activePeer.value
        ? '使用中'
        : peer?.connected ? '待命' : '不可用'
    return {
      name: peer?.name || (peers.value.length ? '已配对电脑' : '还没有已配对电脑'),
      detail: peer
        ? phase.value === 'streaming' && activePeer.value
          ? '已连接 · 正在读取'
          : peer.connected ? '已连接 · 等待读取' : '离线'
        : '前往设备页完成配对',
      state,
      icon: 'waveform',
      foot: phase.value === 'streaming' && activePeer.value
        ? `${activePeer.value.name} 正在读取`
        : `${onlinePeerCount.value}/${peers.value.length} 台在线`,
      tone: !enabled.value || !peer?.connected ? 'warn' : phase.value === 'streaming' ? 'live' : 'idle',
    }
  }
  const state = !enabled.value
    ? '已暂停'
    : !driverInstalled.value || !nativeDriver.value
      ? '未就绪'
      : phase.value === 'streaming' ? '使用中' : '待命'
  return {
    name: 'DeskFuse Mic',
    detail: driverInstalled.value && nativeDriver.value
      ? defaultInput.value ? '虚拟麦克风 · 系统默认输入' : '虚拟麦克风 · 已就绪'
      : driverSummary.value,
    state,
    icon: 'waveform',
    foot: !enabled.value
      ? '接收已暂停'
      : readerTitle.value ? `正在读取 · ${readerTitle.value}` : '等待语音应用读取',
    tone: !enabled.value || !driverInstalled.value || !nativeDriver.value ? 'warn' : phase.value === 'streaming' ? 'live' : 'idle',
  }
})
const stageStatusText = computed(() => (enabled.value ? connectionLabel.value : '已暂停'))
const stageStatusTone = computed(() => {
  if (!enabled.value) return 'off'
  if (connectionTone.value === 'success') return 'live'
  if (connectionTone.value === 'warning' || phase.value === 'failed') return 'warn'
  return 'idle'
})
const stageMetaRate = computed(() => {
  const rate = waveformRate.value
  const value = rate / 1000
  return `${Number.isInteger(value) ? value : value.toFixed(1)} kHz`
})
const stageMetaWindow = computed(() => (waveform.value.length && waveformSpan.value
  ? `${(waveformSpan.value / 1000).toFixed(1)} s 窗口`
  : '实时采样'))
const stageMetaSource = computed(() => (role.value === 'provider' ? '本机输入' : '远端输入'))
const litBars = computed(() => Math.max(0, Math.min(12, Math.round(level.value * 12))))
const stageNotice = computed<null | { title: string; detail: string; action: string; label: string }>(() => {
  if (!enabled.value) return null
  if (role.value === 'provider') {
    if (permission.value !== 'authorized') {
      return {
        title: permission.value === 'denied' ? '麦克风访问已被系统关闭' : '需要麦克风访问权限',
        detail: permission.value === 'denied'
          ? '在系统设置中允许 DeskFuse 使用麦克风；授权后真实波形才会出现。'
          : '允许 DeskFuse 读取实体麦克风，才能采集并显示真实输入。',
        action: 'permission',
        label: permission.value === 'denied' ? '打开设置…' : '允许访问…',
      }
    }
    if (!peers.value.length) {
      return {
        title: '先配对另一台 Mac',
        detail: '已配对的 Mac 才能读取这台 Mac 的麦克风；配对后无需逐台授权。',
        action: 'devices',
        label: '前往设备',
      }
    }
  }
  if (role.value === 'receiver') {
    if (!source.value) {
      return {
        title: '还没有选择来源 Mac',
        detail: '选择一台已配对的 Mac，远端麦克风会出现在 DeskFuse Mic 中。',
        action: 'source',
        label: '选择来源',
      }
    }
    if (!driverInstalled.value || !nativeDriver.value) {
      return {
        title: driverInstalled.value ? '虚拟麦克风未响应' : '虚拟麦克风尚未安装',
        detail: driverSummary.value,
        action: driverButton.value ? 'driver' : '',
        label: driverButton.value?.title ?? '处理中…',
      }
    }
    if (!sourceConnected.value) {
      return {
        title: '来源设备离线',
        detail: '已配对的 Mac 重新上线后会自动连接，不需要重新配置。',
        action: 'source',
        label: '更换来源',
      }
    }
  }
  if (phase.value === 'failed') {
    return {
      title: '连接暂时中断',
      detail: statusText.value,
      action: 'retry',
      label: '重试',
    }
  }
  return null
})

async function pickRole(next: 'provider' | 'receiver') {
  if (role.value === next) {
    showPicker.value = false
    return
  }
  await chooseRole(next)
  if (role.value === next) showPicker.value = false
}
function openPicker() {
  showPicker.value = true
}
function handleStageNotice() {
  const notice = stageNotice.value
  if (!notice) return
  if (notice.action === 'permission') fixPermission()
  else if (notice.action === 'devices') goDevices()
  else if (notice.action === 'source') toggleSourcePicker()
  else if (notice.action === 'driver') driverButtonClick()
  else if (notice.action === 'retry') void run('microphone.retry')
}

const ready = computed(() => {
  if (!enabled.value || nativeState.value.locked || phase.value !== 'standby') return false
  if (role.value === 'provider') return permission.value === 'authorized' && onlinePeerCount.value > 0
  if (role.value === 'receiver')
    return driverInstalled.value && nativeDriver.value && sourceConnected.value
  return false
})

const connectionTone = computed<Tone>(() => {
  if (!enabled.value) return 'neutral'
  if (phase.value === 'streaming') return 'success'
  if (role.value === 'provider' && permission.value !== 'authorized') return 'warning'
  if (role.value === 'receiver' && (!driverInstalled.value || !nativeDriver.value)) return 'warning'
  if (phase.value === 'failed') return 'warning'
  if (ready.value) return 'accent'
  return 'neutral'
})

const connectionLabel = computed(() => {
  if (!enabled.value) return '已关闭'
  if (phase.value === 'streaming') return '传输中'
  if (ready.value) return '按需连接'
  if (role.value === 'provider') {
    if (permission.value !== 'authorized') return '需要权限'
    if (!peers.value.length) return '等待配对'
    if (onlinePeerCount.value === 0) return '等待上线'
  } else if (role.value === 'receiver') {
    if (!source.value) return '未选择来源'
    if (!driverInstalled.value) return '未安装驱动'
    if (!nativeDriver.value) return '驱动未响应'
  }
  if (phase.value === 'waiting') return '连接中'
  return '未连接'
})

const statusText = computed(() => {
  if (!enabled.value) return '共享已关闭'
  if (role.value === 'provider') {
    if (permission.value !== 'authorized')
      return permission.value === 'denied' ? '未允许麦克风访问' : '需要允许麦克风访问'
    if (!peers.value.length) return '配对电脑后即可直接使用此麦克风'
  } else if (role.value === 'receiver') {
    if (!source.value) return '选择提供麦克风的 Mac'
    if (!driverInstalled.value) return '安装虚拟麦克风后即可使用'
    if (!nativeDriver.value) return '驱动服务未响应，重启音频服务后可重新连接'
    if (!sourceConnected.value) return '来源设备离线，等待重新连接'
  }
  return micFriendly(String(liveView.value.detail ?? ''))
})

const driverStatus = computed<{ title: string; tone: 'accent' | 'warning'; icon: string } | null>(() => {
  if (managingDriver.value) return null
  if (!driverInstalled.value || legacyDriverInstalled.value)
    return {
      title: legacyDriverInstalled.value ? '需要更新驱动' : '驱动未安装',
      tone: 'accent',
      icon: 'download-circle',
    }
  if (!nativeDriver.value) return { title: '驱动未就绪', tone: 'warning', icon: 'alert' }
  return null
})

const driverButton = computed<{ title: string; action: DriverAction } | null>(() => {
  if (managingDriver.value) return null
  if (legacyDriverInstalled.value || !driverInstalled.value)
    return { title: legacyDriverInstalled.value ? '更新驱动…' : '安装驱动…', action: 'install' }
  if (!nativeDriver.value) return { title: '重启音频服务…', action: 'restart-audio' }
  return null
})

const driverSummary = computed(() => {
  if (!driverInstalled.value) return legacyDriverInstalled.value ? '检测到旧版驱动，需要更新' : '尚未安装虚拟麦克风'
  if (!nativeDriver.value) return '驱动服务未响应，需要重启音频服务'
  return '已就绪'
})

const micError = computed(() =>
  String(mic.value?.error ?? props.snapshot?.microphone_error ?? '').trim(),
)
const micErrorFriendly = computed(() => micFriendly(micError.value))
const micErrorDetail = computed(() =>
  micError.value && micError.value !== micErrorFriendly.value ? micError.value : '',
)

const driverTitle = computed(() => driverTitleFor(driverAction.value))

function driverTitleFor(action: DriverAction | null): string {
  if (action === 'install') return '安装麦克风驱动'
  if (action === 'uninstall') return '卸载麦克风驱动'
  return '重启音频服务'
}

function micFriendly(raw: string): string {
  const text = unwrapErrorMessage(raw)
  if (!text) return '麦克风服务暂时不可用，请稍后重试。'
  const lower = text.toLowerCase()
  if (/deadline|timed out|timeout/.test(lower)) return '麦克风服务响应超时，请稍后重试。'
  if (/connection refused|econnrefused/.test(lower)) return '请确认后台服务正在运行，然后重新连接。'
  if (/connection reset|econnreset|broken pipe/.test(lower)) return '麦克风服务连接已断开，请重试。'
  if (/not installed/.test(lower)) return '虚拟麦克风驱动尚未安装。'
  if (/尚未就绪|not ready|starting/.test(lower)) return '麦克风服务正在启动，请稍后重试。'
  if (/permission|denied/.test(lower)) return '麦克风权限不可用，请在系统设置中检查。'
  if (/[一-鿿]/.test(text)) return text
  return '麦克风服务返回了错误，请重试。'
}

async function run(action: string, params: Record<string, unknown> = {}): Promise<boolean> {
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

async function chooseRole(next: 'provider' | 'receiver') {
  if (role.value === next) return
  const previous = resolvedRole.value
  localRole.value = next
  showSourcePicker.value = false
  if (next === 'provider') {
    source.value = ''
    pendingSource.value = null
  }
  const ok = await run('microphone.chooseRole', { role: next })
  if (!ok) localRole.value = previous
}

async function configure(patch: Record<string, unknown>) {
  return run('microphone.configure', { settings: { ...settings.value, ...patch } })
}

function chooseSource(id: string) {
  source.value = id
  pendingSource.value = id
  startPendingWindow()
  showSourcePicker.value = false
  void configure({ source: id }).then((ok) => {
    if (!ok) pendingSource.value = null
  })
}

function chooseInput(id: string) {
  inputUid.value = id
  pendingInput.value = id
  startPendingWindow()
  showSourcePicker.value = false
  void configure({ input_uid: id }).then((ok) => {
    if (!ok) pendingInput.value = null
  })
}

function toggleSourcePicker() {
  // 与原生输入选择器一致：打开时刷新系统输入列表。
  if (!showSourcePicker.value && role.value === 'provider') {
    void backend.invoke('microphone.refreshInputs').catch(() => {})
  }
  showSourcePicker.value = !showSourcePicker.value
}

function requestDriver(action: DriverAction) {
  driverAction.value = action
  restartAudio.value = action === 'install'
}

function driverButtonClick() {
  const button = driverButton.value
  if (!button) return
  requestDriver(button.action)
}

async function confirmDriver() {
  const action = driverAction.value
  if (!action) return
  driverAction.value = null
  await run('microphone.manageDriver', {
    action,
    confirm: true,
    restart_audio: action === 'restart-audio' || (action === 'install' && restartAudio.value),
  })
}

function fixPermission() {
  void run('permissions.open', {
    permission: 'microphone',
    mode: permission.value === 'denied' ? 'settings' : 'authorize',
  })
}

function setDefaultInput() {
  void run('microphone.setDefaultInput')
}

function reportMicError() {
  void run('support.report', {
    error: micErrorDetail.value || micError.value || micErrorFriendly.value,
  })
}

function focusSheet() {
  // 焦点放在对话框本身：读屏先播报标题，也不会一打开就点亮某个按钮的焦点圈。
  sheetEl.value?.focus({ preventScroll: true })
}

function trapSheetTab(event: KeyboardEvent) {
  const root = sheetEl.value
  if (!root) return
  const items = Array.from(
    root.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), [href], [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((item) => item.offsetParent !== null)
  if (!items.length) return
  const first = items[0]
  const last = items[items.length - 1]
  if (document.activeElement === root) {
    event.preventDefault()
    ;(event.shiftKey ? last : first).focus()
  } else if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}

watch(
  () => showSourcePicker.value || Boolean(driverAction.value),
  (open) => {
    if (open) {
      lastFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
      void nextTick(focusSheet)
    } else {
      void nextTick(() => {
        lastFocus?.focus({ preventScroll: true })
        lastFocus = null
      })
    }
  },
)

function goDevices() {
  showSourcePicker.value = false
  emit('navigate', 'devices')
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && (showSourcePicker.value || driverAction.value)) {
    showSourcePicker.value = false
    driverAction.value = null
    return
  }
  if (event.key === 'Tab' && (showSourcePicker.value || driverAction.value)) trapSheetTab(event)
}

async function pollLive() {
  if (!liveAvailable || !mic.value) return
  if (document.visibilityState === 'hidden') return
  try {
    const state = await backend.invoke('microphone.status')
    if (state && typeof state === 'object') live.value = state
  } catch {
    // 旧宿主或 Windows 适配层没有这个动作时退回 1.5 s 快照，避免反复报错。
    liveAvailable = false
    stopLivePolling()
  }
}
function startLivePolling() {
  if (liveTimer || !liveAvailable) return
  void pollLive()
  // 160ms 轮询：数据末端离当前更近，画布只需滞后 300ms 就能连续滚动；投影本身是轻量字段。
  liveTimer = setInterval(() => { void pollLive() }, 160)
}
function stopLivePolling() {
  if (!liveTimer) return
  clearInterval(liveTimer)
  liveTimer = undefined
}

watch(
  mic,
  (value) => {
    if (!value) return
    if (!initialized) {
      initialized = true
      // 与原生页 onAppear 一致：页面出现时确保后台运行时已启动，并补齐输入列表。
      void backend.invoke('microphone.ensure').catch(() => {})
      void backend.invoke('microphone.refreshInputs').catch(() => {})
    }
    startLivePolling()
  },
  { immediate: true },
)

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  narrowQuery.addEventListener('change', onNarrowLayout)
  prepareSpeech()
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  narrowQuery.removeEventListener('change', onNarrowLayout)
  if (pendingTimer) clearTimeout(pendingTimer)
  stopSpeechPolling()
  stopLivePolling()
  // 离开页面即释放录音与识别资源，避免后台继续占用麦克风。
  if (speechListening.value) void speechInvoke('microphone.speech.stop')
})
</script>


<template>
  <div class="microphone-page" :class="role ? `is-${role}` : ''">
    <FeatureGate
      v-if="snapshot && !featureEnabled"
      page="microphone"
      title="麦克风共享"
      :blocked-reason="snapshot?.capabilities?.microphone_reason"
    />

    <div v-else-if="!snapshot" class="mic-scroll state-scroll">
      <div class="state-page">
        <header class="state-top">
          <span class="mono">DESK FUSE / MICROPHONE</span>
          <span class="state-mark">BOOT</span>
        </header>
        <div class="state-body">
          <span class="state-glyph"><Icon name="mic" :size="22" /></span>
          <div class="state-copy">
            <h1>正在准备麦克风共享</h1>
            <p>读取系统输入、已配对设备和驱动状态，首次打开可能需要几秒钟。</p>
          </div>
          <span class="spinner state-spinner" aria-label="正在加载" />
        </div>
        <Notice
          v-if="error"
          class="page-error"
          tone="danger"
          title="操作未完成"
          :message="error"
          :raw="errorRaw || undefined"
          dismissible
          @dismiss="error = ''"
        />
        <p class="privacy-line">
          <Icon name="shield" :size="12" />
          <span>仅可信的已配对设备可以使用；DeskFuse 不保存或转写音频，只在应用读取麦克风时采集和传输。</span>
        </p>
      </div>
    </div>

    <template v-else-if="!mic || !micLoaded">
      <div class="mic-scroll state-scroll">
        <div class="state-page">
          <header class="state-top">
            <span class="mono">DESK FUSE / MICROPHONE</span>
            <span class="state-mark" :class="{ error: Boolean(micError) }">{{ micError ? 'ERROR' : 'BOOT' }}</span>
          </header>
          <section v-if="micError" class="error-state">
            <div class="state-body">
              <span class="state-glyph danger"><Icon name="mic-off" :size="22" /></span>
              <div class="state-copy">
                <h1>无法连接麦克风服务</h1>
                <p>{{ micErrorFriendly }}</p>
              </div>
            </div>
            <details v-if="micErrorDetail" class="tech">
              <summary><Icon name="chevron-right" :size="11" class="chev" /><span>技术详情</span></summary>
              <pre>{{ micErrorDetail }}</pre>
            </details>
            <div class="state-actions">
              <button type="button" class="line-button primary" :disabled="busy" @click="run('microphone.retry')">
                重新连接<Icon name="arrow-right" :size="13" />
              </button>
              <button type="button" class="line-button" :disabled="busy" @click="reportMicError">
                反馈问题…
              </button>
            </div>
          </section>
          <section v-else class="state-body">
            <span class="state-glyph"><Icon name="mic" :size="22" /></span>
            <div class="state-copy">
              <h1>正在准备麦克风共享</h1>
              <p>读取系统输入、已配对设备和驱动状态，首次打开可能需要几秒钟。</p>
            </div>
            <span class="spinner state-spinner" aria-label="正在加载" />
          </section>
          <Notice
            v-if="error"
            class="page-error"
            tone="danger"
            title="操作未完成"
            :message="error"
            :raw="errorRaw || undefined"
            dismissible
            @dismiss="error = ''"
          />
          <p class="privacy-line">
            <Icon name="shield" :size="12" />
            <span>仅可信的已配对设备可以使用；DeskFuse 不保存或转写音频，只在应用读取麦克风时采集和传输。</span>
          </p>
        </div>
      </div>
    </template>

    <div v-else class="mic-scroll">
      <div class="mic-page">
        <section v-if="!configured || showPicker" class="picker-view" aria-labelledby="pickerTitle">
          <div class="picker-bar">
            <span class="picker-kicker">MICROPHONE / CHOOSE A SIDE</span>
            <button v-if="configured" type="button" class="picker-back" @click="showPicker = false">
              <Icon name="chevron-right" :size="12" class="flip" /><span>返回链路</span>
            </button>
          </div>
          <header class="picker-head">
            <h1 id="pickerTitle">这台 Mac 站在<span class="no-break">声音链路</span>的哪一端？</h1>
            <p>两端共用同一套配对与权限。选择一端开始，之后随时可以从页头回到这里重选；正在使用的输入会先停止。</p>
          </header>

          <div class="role-choices" role="group" aria-label="选择这台 Mac 的身份">
            <button
              type="button"
              class="role-choice"
              :class="{ 'is-current': configured && role === 'provider' }"
              :aria-current="configured && role === 'provider' ? 'true' : undefined"
              :disabled="busy"
              @click="pickRole('provider')"
            >
              <span class="choice-top">
                <span class="choice-code">PROVIDER / OUT</span>
                <span class="choice-current">当前身份</span>
              </span>
              <span class="choice-diagram" aria-hidden="true">
                <span class="choice-endpoint">
                  <span class="choice-icon accent"><Icon name="mic" :size="14" /></span>
                  <span class="choice-endpoint-copy"><b>{{ localName }}</b><small>本机 · 麦克风</small></span>
                </span>
                <span class="choice-link">
                  <svg viewBox="0 0 100 26" preserveAspectRatio="none"><path d="M2 13H98M92 8l6 5-6 5" /></svg>
                  <i class="choice-dot d1" /><i class="choice-dot d2" /><i class="choice-dot d3" />
                </span>
                <span class="choice-endpoint">
                  <span class="choice-icon"><Icon name="laptop" :size="14" /></span>
                  <span class="choice-endpoint-copy"><b>{{ pickPeerName }}</b><small>{{ pickPeer ? (pickPeer.connected ? '对方 · 在线' : '对方 · 离线') : '对方 · 尚未配对' }}</small></span>
                </span>
              </span>
              <span class="choice-title">分享这台 Mac 的麦克风</span>
              <span class="choice-desc">让已配对的 Mac 把这台 Mac 的输入当作麦克风使用。</span>
              <span class="choice-points">
                <span><Icon name="check-circle" :size="12" />对方读取 DeskFuse 输入时，会收到这台 Mac 的声音。</span>
                <span><Icon name="check-circle" :size="12" />对方停止读取后，共享自动结束。</span>
              </span>
              <span class="choice-go">
                <span class="choice-go-label">{{ configured && role === 'provider' ? '继续使用这一端' : '选择提供端' }}</span>
                <Icon name="chevron-right" :size="13" />
              </span>
            </button>

            <button
              type="button"
              class="role-choice"
              :class="{ 'is-current': configured && role === 'receiver' }"
              :aria-current="configured && role === 'receiver' ? 'true' : undefined"
              :disabled="busy"
              @click="pickRole('receiver')"
            >
              <span class="choice-top">
                <span class="choice-code">RECEIVER / IN</span>
                <span class="choice-current">当前身份</span>
              </span>
              <span class="choice-diagram" aria-hidden="true">
                <span class="choice-endpoint">
                  <span class="choice-icon"><Icon name="laptop" :size="14" /></span>
                  <span class="choice-endpoint-copy"><b>{{ pickPeerName }}</b><small>{{ pickPeer ? (pickPeer.connected ? '对方 · 在线' : '对方 · 离线') : '对方 · 尚未配对' }}</small></span>
                </span>
                <span class="choice-link">
                  <svg viewBox="0 0 100 26" preserveAspectRatio="none"><path d="M2 13H98M92 8l6 5-6 5" /></svg>
                  <i class="choice-dot d1" /><i class="choice-dot d2" /><i class="choice-dot d3" />
                </span>
                <span class="choice-endpoint">
                  <span class="choice-icon accent"><Icon name="mic" :size="14" /></span>
                  <span class="choice-endpoint-copy"><b>{{ localName }}</b><small>本机 · DeskFuse Mic</small></span>
                </span>
              </span>
              <span class="choice-title">使用另一台 Mac 的麦克风</span>
              <span class="choice-desc">把远端声音接入这台 Mac，在通话、听写和录音里当作输入。</span>
              <span class="choice-points">
                <span><Icon name="check-circle" :size="12" />系统里会出现 DeskFuse Mic，选中它就能用远端麦克风。</span>
                <span><Icon name="check-circle" :size="12" />远端断开时回到待机，不会切回内置麦克风。</span>
              </span>
              <span class="choice-go">
                <span class="choice-go-label">{{ configured && role === 'receiver' ? '继续使用这一端' : '选择使用端' }}</span>
                <Icon name="chevron-right" :size="13" />
              </span>
            </button>
          </div>

          <Notice
            v-if="!peers.length"
            tone="neutral"
            title="先配对另一台 Mac"
            message="提供和使用麦克风都需要两台 Mac 已在“设备”中完成配对。"
            action-title="前往设备"
            @action="goDevices"
          />
          <Notice
            v-if="error"
            tone="danger"
            title="操作未完成"
            :message="error"
            :raw="errorRaw || undefined"
            dismissible
            @dismiss="error = ''"
          />
          <p class="preview-note">
            <Icon name="shield" :size="12" />
            <span>波形来自真实音频采样，只在这台 Mac 上分析。共享只在应用读取麦克风时进行，音频不上传、不保存。</span>
          </p>
        </section>

        <template v-else>
          <header class="page-header">
            <div>
              <h1>麦克风共享</h1>
              <p class="subtitle">{{ headerSummary }}</p>
            </div>
            <div class="header-actions">
              <button type="button" class="identity-switch" :disabled="busy" @click="openPicker">
                <Icon name="swap" :size="13" /><span>切换身份</span>
              </button>
            </div>
          </header>

          <section class="stage" aria-label="麦克风实时链路">
            <div class="stage-top">
              <span class="stage-kicker">{{ role === 'provider' ? 'PROVIDER / 提供方链路' : 'RECEIVER / 使用方链路' }}</span>
              <span class="stage-status" :class="stageStatusTone" aria-live="polite"><i /><span>{{ stageStatusText }}</span></span>
              <span class="stage-meta">
                <span>{{ stageMetaRate }}</span><span class="sep">·</span>
                <span>{{ stageMetaWindow }}</span><span class="sep">·</span>
                <span>{{ stageMetaSource }}</span>
              </span>
            </div>

            <div v-if="stageNotice" class="stage-notice" role="status">
              <span class="stage-notice-icon" aria-hidden="true"><Icon name="alert" :size="13" /></span>
              <div class="stage-notice-copy"><b>{{ stageNotice.title }}</b><span>{{ stageNotice.detail }}</span></div>
              <button v-if="stageNotice.action" type="button" class="stage-notice-action" @click="handleStageNotice">{{ stageNotice.label }}</button>
            </div>

            <div class="stage-main">
              <article class="pod pod-source" :class="`is-${sourcePod.tone}`">
                <div class="pod-head">
                  <span class="role-badge">{{ role === 'provider' ? '本机' : '对方' }}</span>
                  <span class="pod-state"><i /><span>{{ sourcePod.state }}</span></span>
                </div>
                <div class="pod-device">
                  <span class="pod-icon">
                    <DeviceIcon :name="sourcePod.name" :src="sourceArt" :size="42" />
                    <span class="pod-badge"><Icon :name="sourcePod.icon" :size="9" /></span>
                  </span>
                  <div>
                    <b>{{ sourcePod.name }}</b>
                    <small>{{ sourcePod.detail }}</small>
                  </div>
                </div>
                <p class="pod-meta">
                  <span v-for="fact in sourceFacts" :key="fact.text" :class="{ warn: fact.tone === 'warn' }">{{ fact.text }}</span>
                </p>
              </article>

              <div class="link">
                <div class="link-meter">
                  <span class="link-level"><b>{{ levelDb }}</b><small>dBFS</small></span>
                  <span class="level-bars" aria-hidden="true">
                    <i v-for="index in 12" :key="index" :class="{ on: index <= litBars }" />
                  </span>
                </div>
                <div class="link-canvas">
                  <MicLinkCanvas :samples="waveform" :span-ms="waveformSpan" :age-ms="waveformAge" :active="phase === 'streaming' && waveformActive" :paused="!enabled" :vertical="narrowLayout" />
                </div>
              </div>

              <article class="pod pod-dest" :class="`is-${destPod.tone}`">
                <div class="pod-head">
                  <span class="role-badge">{{ role === 'provider' ? '对方' : '本机' }}</span>
                  <span class="dest-pill"><i /><span>{{ destPod.state }}</span></span>
                </div>
                <div class="pod-device">
                  <span class="pod-icon">
                    <DeviceIcon :name="destPod.name" :src="destArt" :size="42" />
                    <span class="pod-badge" :class="{ socket: role === 'receiver' }"><Icon :name="destPod.icon" :size="9" /></span>
                  </span>
                  <div>
                    <b>{{ destPod.name }}</b>
                    <small>{{ destPod.detail }}</small>
                  </div>
                </div>
                <p class="pod-meta">
                  <span v-if="role === 'receiver'" class="app-read"><i /><span>{{ destPod.foot }}</span></span>
                  <span v-for="fact in destFacts" :key="fact.text" :class="{ warn: fact.tone === 'warn' }">{{ fact.text }}</span>
                </p>
              </article>
            </div>
          </section>

          <section class="setup">
            <div class="setup-col">
              <div class="section-label">来源 / SOURCE</div>

              <button type="button" class="setting" @click="toggleSourcePicker">
                <span class="row-icon"><Icon :name="role === 'provider' ? 'mic' : 'laptop'" :size="14" /></span>
                <span class="row-copy">
                  <b>{{ role === 'provider' ? inputDisplayName : sourcePeerName }}</b>
                  <small>{{ role === 'provider' ? (inputUid ? '本机麦克风' : '跟随系统当前输入设备') : (sourceConnected ? '远端来源 · 在线' : source ? '远端来源 · 离线' : '点击选择一台已配对电脑') }}</small>
                </span>
                <span class="row-action">{{ role === 'provider' || source ? '更换' : '选择' }}<Icon name="chevron-right" :size="12" /></span>
              </button>

              <div v-if="role === 'provider'" class="setting">
                <span class="row-icon"><Icon name="shield" :size="14" /></span>
                <span class="row-copy">
                  <b>麦克风访问</b>
                  <small>{{ permission === 'authorized' ? '已允许 DeskFuse 读取实体麦克风' : permission === 'denied' ? '已在系统设置中关闭' : '首次使用时会请求系统授权' }}</small>
                </span>
                <span v-if="permission === 'authorized'" class="row-value ok"><Icon name="check-circle" :size="11" />已允许</span>
                <button v-else type="button" class="row-action" :disabled="busy" @click="fixPermission">
                  {{ permission === 'denied' ? '打开设置…' : '允许访问…' }}
                </button>
              </div>

              <button v-if="role === 'provider'" type="button" class="setting" @click="goDevices">
                <span class="row-icon"><Icon name="users" :size="14" /></span>
                <span class="row-copy">
                  <b>可信设备</b>
                  <small>{{ peerSummary }}</small>
                </span>
                <span class="row-action">{{ peers.length ? '管理' : '去配对' }}<Icon name="chevron-right" :size="12" /></span>
              </button>

              <template v-else>
                <div class="setting">
                  <span class="row-icon" :class="driverStatus ? `tone-${driverStatus.tone}` : ''"><Icon :name="driverStatus?.icon ?? 'check-circle'" :size="14" /></span>
                  <span class="row-copy">
                    <b>虚拟麦克风</b>
                    <small>{{ driverSummary }}</small>
                  </span>
                  <button v-if="driverButton" type="button" class="row-action" :disabled="busy" @click="driverButtonClick">{{ driverButton.title }}</button>
                  <span v-else-if="managingDriver" class="row-value"><span class="spinner" />处理中…</span>
                  <span v-else class="row-value ok"><Icon name="check-circle" :size="11" />就绪</span>
                </div>
                <div class="setting">
                  <span class="row-icon"><Icon name="waveform" :size="14" /></span>
                  <span class="row-copy">
                    <b>系统输入</b>
                    <small>{{ !driverInstalled ? '安装虚拟麦克风后可设置' : defaultInput ? '已是系统默认输入' : !nativeDriver ? '驱动恢复后可设置' : '建议设为系统默认输入' }}</small>
                  </span>
                  <button v-if="driverInstalled && nativeDriver && !defaultInput" type="button" class="row-action" :disabled="busy" @click="setDefaultInput">设为默认</button>
                  <span v-else-if="defaultInput" class="row-value ok"><Icon name="check-circle" :size="11" />默认</span>
                </div>
              </template>

              <details v-if="role === 'receiver' && (driverInstalled || legacyDriverInstalled)" class="maintenance">
                <summary>
                  <Icon name="wrench" :size="13" />
                  <span>驱动维护</span>
                  <Icon name="chevron-right" :size="12" class="chev" />
                </summary>
                <div class="maintenance-body">
                  <template v-if="driverInstalled">
                    <div class="maintenance-row">
                      <div><b>音频服务</b><small>连接异常时先重启，保留驱动与共享设置。</small></div>
                      <button type="button" class="line-button small" @click="requestDriver('restart-audio')">重启…</button>
                    </div>
                    <div class="maintenance-row">
                      <div><b>麦克风驱动</b><small>重启后仍不可用时，可尝试重新安装。</small></div>
                      <button type="button" class="line-button small" @click="requestDriver('install')">重新安装…</button>
                    </div>
                  </template>
                  <div class="maintenance-row danger-row">
                    <div><b>移除驱动</b><small>移除后，此 Mac 将无法接收远端麦克风。</small></div>
                    <button type="button" class="line-button small" @click="requestDriver('uninstall')">{{ legacyDriverInstalled ? '卸载驱动（含旧版）…' : '卸载…' }}</button>
                  </div>
                </div>
              </details>

              <Notice
                v-if="error"
                class="page-error"
                tone="danger"
                title="操作未完成"
                :message="error"
                :raw="errorRaw || undefined"
                dismissible
                @dismiss="error = ''"
              />
            </div>

            <div class="setup-col">
              <div class="section-label">语音测试 / VOICE TEST</div>
              <button
                type="button"
                class="voice-test"
                :class="{ listening: speechListening }"
                :disabled="speechPhase === 'loading' || speechPhase === 'authorizing'"
                @click="speechListening ? stopSpeech() : toggleSpeech()"
              >
                <span class="vt-icon"><Icon :name="speechListening ? 'stop' : 'mic'" :size="15" /></span>
                <span class="vt-copy">
                  <b>{{ speechStatus.title }}</b>
                  <small>{{ speechStatus.detail || (speechStatus.parts.join('，')) }}</small>
                </span>
                <span class="vt-action">{{ speechListening ? '停止' : '开始' }}</span>
              </button>
              <div v-if="speechListening" class="speech-level" role="img" aria-label="语音测试输入电平">
                <span v-for="index in 12" :key="index" :class="{ on: speechLevel * 12 >= index }" />
              </div>
              <Notice
                v-if="speechProblem"
                class="speech-notice"
                tone="warning"
                dismissible
                :title="String(speechProblem.title ?? '语音测试不可用')"
                :message="String(speechProblem.message ?? '')"
                :action-title="speechProblem.settings_url ? '打开系统设置' : undefined"
                @action="openSpeechSettings"
                @dismiss="dismissSpeechProblem"
              />
              <textarea
                id="microphone-speech-text"
                name="microphone-speech-text"
                v-model="speechText"
                class="speech-input"
                :class="{ listening: speechListening }"
                rows="2"
                :readonly="speechListening"
                :placeholder="speechListening ? '正在聆听，请说话…' : '识别结果，也可直接输入'"
                aria-label="语音识别结果"
              />
              <p class="privacy">
                <Icon name="shield" :size="11" />
                <span>真实输入只在这台 Mac 上分析，不上传、不保存。</span>
              </p>
            </div>
          </section>

          <p class="page-foot">
            <Icon name="shield" :size="11" />
            <span>仅可信设备可用 · 共享在对方停止读取后自动结束</span>
          </p>
        </template>
      </div>
    </div>

    <Teleport to="body">
      <Transition name="sheet">
        <div v-if="showSourcePicker" class="sheet-scrim" @mousedown.self="showSourcePicker = false">
          <div
            ref="sheetEl"
            class="sheet picker-sheet"
            role="dialog"
            aria-modal="true"
            tabindex="-1"
            :aria-label="role === 'provider' ? '选择麦克风' : '选择来源设备'"
          >
            <div class="sheet-head">
              <h2>{{ role === 'provider' ? '选择麦克风' : '选择来源设备' }}</h2>
              <button type="button" class="icon-button" aria-label="关闭" @click="showSourcePicker = false">
                <Icon name="x" :size="14" />
              </button>
            </div>
            <p v-if="role === 'receiver'" class="secondary picker-desc">
              选择提供麦克风的 Mac。已配对设备可直接使用。
            </p>

            <template v-if="role === 'provider'">
              <div class="pick-list">
                <button
                  type="button"
                  class="pick-row"
                  :class="{ selected: inputUid === '' }"
                  :aria-pressed="inputUid === ''"
                  @click="chooseInput('')"
                >
                  <span class="pick-icon"><Icon name="mic" :size="14" /></span>
                  <span class="pick-name">系统默认麦克风</span>
                  <span class="pick-check" :class="{ on: inputUid === '' }" />
                </button>
                <button
                  v-for="input in inputs"
                  :key="input.id"
                  type="button"
                  class="pick-row"
                  :class="{ selected: inputUid === input.id }"
                  :aria-pressed="inputUid === input.id"
                  @click="chooseInput(input.id)"
                >
                  <span class="pick-icon"><Icon name="mic" :size="14" /></span>
                  <span class="pick-name">{{ input.name }}</span>
                  <span class="pick-check" :class="{ on: inputUid === input.id }" />
                </button>
                <div v-if="inputUid && !inputs.some((input) => input.id === inputUid)" class="pick-row selected">
                  <span class="pick-icon"><Icon name="mic-off" :size="14" /></span>
                  <span class="pick-name">所选麦克风不可用</span>
                  <span class="pick-check on" />
                </div>
              </div>
            </template>

            <template v-else>
              <div v-if="!peers.length" class="peer-empty">
                <span class="peer-empty-icon"><Icon name="laptop" :size="18" /></span>
                <div class="peer-empty-text">
                  <strong>还没有已配对的电脑</strong>
                  <small>先在“设备”中完成配对；之后无需额外授权。</small>
                </div>
                <button type="button" class="line-button small" @click="goDevices">前往设备</button>
              </div>
              <div v-else class="pick-list">
                <button
                  v-for="peer in peers"
                  :key="peer.id"
                  type="button"
                  class="pick-row peer-row"
                  :class="{ selected: source === peer.id }"
                  :aria-pressed="source === peer.id"
                  @click="chooseSource(peer.id)"
                >
                  <span class="pick-icon"><Icon name="laptop" :size="14" /></span>
                  <span class="pick-name">
                    {{ peer.name || '未命名设备' }}
                    <small :class="peer.connected ? 'online' : ''">{{ peer.connected ? '在线' : '离线' }}</small>
                  </span>
                  <span class="pick-check" :class="{ on: source === peer.id }" />
                </button>
              </div>
            </template>
          </div>
        </div>
      </Transition>

      <Transition name="sheet">
        <div v-if="driverAction" class="sheet-scrim" @mousedown.self="driverAction = null">
          <div ref="sheetEl" class="sheet" role="dialog" aria-modal="true" tabindex="-1" :aria-label="driverTitle">
            <div class="sheet-head">
              <h2>{{ driverTitle }}</h2>
              <button type="button" class="icon-button" aria-label="关闭" @click="driverAction = null">
                <Icon name="x" :size="14" />
              </button>
            </div>
            <template v-if="driverAction === 'restart-audio'">
              <p>这会短暂中断此 Mac 上所有应用的录音和播放，需要管理员授权。</p>
              <p class="secondary">保留已安装的驱动和当前共享设置。服务恢复后按需连接，语音应用可能需要重新开始录音。</p>
            </template>
            <template v-else>
              <p>仅操作 DeskFuse 的麦克风驱动（包括旧版名称遗留的驱动），需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。</p>
              <p v-if="driverAction === 'uninstall'" class="sheet-warning">卸载后，此 Mac 将无法接收远端麦克风，直到重新安装；正在使用麦克风的应用会断开。</p>
              <template v-if="driverAction === 'install'">
                <label class="sheet-check">
                  <span>完成后立即重启音频服务</span>
                  <Toggle :model-value="restartAudio" label="完成后立即重启音频服务" @update:model-value="restartAudio = $event" />
                </label>
                <p class="secondary">{{ restartAudio ? '这会短暂中断此 Mac 上所有应用的录音和播放。' : '稍后重启 Mac 后生效。' }}</p>
              </template>
            </template>
            <div class="sheet-actions">
              <button type="button" class="button" @click="driverAction = null">取消</button>
              <button type="button" class="button" :class="driverAction === 'uninstall' ? 'danger-solid' : 'primary'" :disabled="busy" @click="confirmDriver">{{ driverTitle }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.microphone-page {
  --line-strong: color-mix(in srgb, var(--text) 16%, transparent);
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  color: var(--text);
}
.mic-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
.mic-page {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  padding: 22px 24px 28px;
}
.mono {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}
.no-break { white-space: nowrap; }
button:disabled { opacity: .45; }

/* ---------- 页头 ---------- */
.page-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 24px;
  margin-bottom: 16px;
}
.page-header h1 {
  font-size: 21px;
  line-height: 1.25;
  letter-spacing: -.015em;
}
.subtitle {
  margin-top: 4px;
  color: var(--secondary);
  font-size: 12px;
  line-height: 1.55;
  text-wrap: pretty;
}
.header-actions { display: flex; align-items: center; gap: 10px; flex: none; }
.identity-switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 34px;
  padding: 0 10px;
  border: .5px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  font-size: 11.5px;
  color: var(--secondary);
  white-space: nowrap;
  transition: border-color var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out);
}
.identity-switch:hover { border-color: var(--line-strong); color: var(--text); }

/* ---------- 身份选择 ---------- */
.picker-view { padding-top: 8px; }
.picker-head { max-width: 660px; }
.picker-bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 22px; }
.picker-back {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 0;
  font-size: 10.5px;
  color: var(--secondary);
  transition: color var(--dur-fast) var(--ease-out);
}
.picker-back:hover { color: var(--text); }
.picker-back .flip { transform: rotate(180deg); }
.picker-kicker {
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: .12em;
  color: var(--secondary);
}
.picker-head h1 {
  margin: 9px 0 0;
  font-size: 27px;
  line-height: 1.2;
  font-weight: 600;
  letter-spacing: -.025em;
  text-wrap: balance;
}
.picker-head p {
  margin: 8px 0 0;
  color: var(--secondary);
  font-size: 12.5px;
  line-height: 1.6;
  text-wrap: pretty;
}
.role-choices {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  margin-top: 24px;
}
.role-choice {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  min-height: 280px;
  padding: 18px;
  text-align: left;
  border: 1px solid var(--line);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  transition: border-color var(--dur-base) var(--ease-out), box-shadow var(--dur-base) var(--ease-out), transform var(--dur-base) var(--ease-out);
}
.role-choice:hover {
  border-color: color-mix(in srgb, var(--accent) 26%, var(--line));
  box-shadow: 0 10px 30px rgba(15, 23, 42, .06);
  transform: translateY(-1px);
}
.role-choice:active { transform: translateY(0); }
.role-choice.is-current {
  border-color: color-mix(in srgb, var(--accent) 40%, var(--line));
  background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 3%, var(--surface)), var(--surface));
}
.role-choice:disabled { cursor: default; transform: none; }
.choice-top { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.choice-code {
  font-family: var(--font-mono);
  font-size: 9px;
  letter-spacing: .1em;
  color: var(--secondary);
}
.choice-current {
  display: none;
  align-items: center;
  height: 17px;
  padding: 0 6px;
  border-radius: 4px;
  font-size: 9px;
  line-height: 1;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 8%, var(--surface));
  border: .5px solid color-mix(in srgb, var(--accent) 22%, transparent);
}
.role-choice.is-current .choice-current { display: inline-flex; }
.choice-diagram {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 18px 0;
  padding: 15px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius-card);
  background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 2%, var(--surface)), color-mix(in srgb, var(--accent) 1%, var(--canvas)));
}
.choice-endpoint { display: flex; align-items: center; gap: 7px; min-width: 0; }
.choice-icon {
  width: 30px;
  height: 30px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: var(--control);
  color: var(--secondary);
}
.choice-icon.accent { background: color-mix(in srgb, var(--accent) 9%, var(--surface)); color: var(--accent); }
.choice-endpoint-copy { min-width: 0; }
.choice-endpoint-copy b {
  display: block;
  font-size: 10.5px;
  font-weight: 600;
  line-height: 1.3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 130px;
}
.choice-endpoint-copy small {
  display: block;
  margin-top: 1px;
  font-size: 9px;
  line-height: 1.3;
  color: var(--secondary);
  white-space: nowrap;
}
.choice-link { position: relative; flex: 1 1 48px; min-width: 36px; height: 26px; }
.choice-link svg { width: 100%; height: 100%; overflow: visible; }
.choice-link path { fill: none; stroke: color-mix(in srgb, var(--accent) 55%, transparent); stroke-width: 1.4; vector-effect: non-scaling-stroke; }
.choice-dot { position: absolute; top: 50%; width: 4px; height: 4px; margin-top: -2px; border-radius: 50%; background: var(--accent); opacity: .88; }
.choice-dot.d1 { left: 24%; }
.choice-dot.d2 { left: 50%; }
.choice-dot.d3 { left: 76%; }
.choice-title { display: block; font-size: 15px; font-weight: 600; line-height: 1.35; letter-spacing: -.01em; }
.choice-desc { display: block; margin-top: 5px; font-size: 11px; line-height: 1.55; color: var(--secondary); }
.choice-points { display: grid; gap: 5px; margin-top: 12px; }
.choice-points span { display: flex; align-items: flex-start; gap: 5px; font-size: 10.5px; line-height: 1.45; color: var(--secondary); }
.choice-points svg { color: var(--success); margin-top: 1px; flex: none; }
.choice-go {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-top: auto;
  padding-top: 16px;
  font-size: 11.5px;
  font-weight: 500;
  color: var(--accent);
}
.preview-note {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  margin-top: 14px;
  padding: 9px 11px;
  border: .5px solid color-mix(in srgb, var(--warning) 32%, var(--line));
  border-radius: 8px;
  background: color-mix(in srgb, var(--warning) 6%, var(--surface));
  font-size: 10.5px;
  line-height: 1.55;
  color: var(--secondary);
}
.preview-note svg { color: var(--warning); margin-top: 1px; flex: none; }
.picker-view :deep(.notice) { margin-top: 14px; }
.picker-view :deep(.notice .button) { min-height: 28px; font-size: 11px; }

/* ---------- 声桥舞台 ---------- */
.stage {
  position: relative;
  overflow: hidden;
  padding: 14px 18px 12px;
  border: 1px solid var(--line);
  border-radius: var(--radius-panel);
  background:
    radial-gradient(90% 130% at 50% 48%, color-mix(in srgb, var(--accent) 6%, transparent), transparent 64%),
    linear-gradient(180deg, color-mix(in srgb, var(--accent) 2%, var(--surface)) 0%, var(--canvas) 100%);
}
.stage::before {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-image: repeating-linear-gradient(90deg, var(--line) 0 1px, transparent 1px 56px);
  opacity: .45;
  mask-image: linear-gradient(180deg, transparent 0, #000 16%, #000 84%, transparent 100%);
  -webkit-mask-image: linear-gradient(180deg, transparent 0, #000 16%, #000 84%, transparent 100%);
}
.stage-top { position: relative; z-index: 3; display: flex; align-items: center; gap: 14px; min-height: 22px; }
.stage-kicker { font-family: var(--font-mono); font-size: 9.5px; letter-spacing: .12em; color: var(--secondary); white-space: nowrap; }
.stage-status { display: inline-flex; align-items: center; gap: 6px; font-size: 10.5px; font-weight: 500; color: var(--success); white-space: nowrap; }
.stage-status i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; box-shadow: 0 0 0 3px color-mix(in srgb, currentColor 12%, transparent); }
.stage-status.live i { animation: statusBreathe 2s ease-in-out infinite; }
.stage-status.idle { color: var(--secondary); }
.stage-status.idle i { box-shadow: none; animation: none; }
.stage-status.warn { color: var(--warning); }
.stage-status.warn i { box-shadow: none; animation: none; }
.stage-status.off { color: var(--secondary); }
.stage-status.off i { box-shadow: none; animation: none; }
@keyframes statusBreathe { 0%, 100% { opacity: 1; } 50% { opacity: .45; } }
.stage-meta { margin-left: auto; font-family: var(--font-mono); font-size: 9.5px; color: var(--secondary); white-space: nowrap; font-variant-numeric: tabular-nums; }
.stage-meta .sep { opacity: .5; margin: 0 4px; }
.stage-main {
  position: relative;
  z-index: 2;
  display: grid;
  grid-template-columns: 206px minmax(0, 1fr) 206px;
  align-items: center;
  min-height: 272px;
  padding: 8px 0 10px;
}
/* 电平行占顶部，底部留同样高度，画布中心仍与两侧设备卡的中线对齐。 */
.link { position: relative; display: grid; grid-template-rows: 22px minmax(0, 1fr); height: 272px; padding-bottom: 22px; min-width: 0; }
.stage-notice {
  position: relative;
  z-index: 3;
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 6px 0 0;
  padding: 8px 10px;
  border: .5px solid color-mix(in srgb, var(--warning) 32%, var(--line));
  border-radius: 8px;
  background: color-mix(in srgb, var(--warning) 6%, var(--surface));
}
.stage-notice-icon { color: var(--warning); flex: none; display: grid; place-items: center; }
.stage-notice-copy { flex: 1; min-width: 0; }
.stage-notice-copy b { display: block; font-size: 11px; font-weight: 500; line-height: 1.35; }
.stage-notice-copy span { display: block; margin-top: 1px; font-size: 10px; line-height: 1.45; color: var(--secondary); }
.stage-notice-action { flex: none; padding: 4px 0 4px 8px; font-size: 10.5px; color: var(--accent); white-space: nowrap; }

.pod {
  position: relative;
  z-index: 3;
  width: 206px;
  padding: 11px 12px 10px;
  border: 1px solid var(--line);
  border-radius: var(--radius-card);
  background: color-mix(in srgb, var(--surface) 95%, transparent);
  box-shadow: 0 1px 2px rgba(0, 0, 0, .025);
}
.pod-source { justify-self: start; }
.pod-dest { justify-self: end; }
.pod.is-live { border-color: color-mix(in srgb, var(--accent) 32%, var(--line)); background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 4%, var(--surface)), var(--surface)); }
.pod.is-warn { border-color: color-mix(in srgb, var(--warning) 30%, var(--line)); }
.pod-head { display: flex; align-items: center; justify-content: space-between; gap: 6px; min-height: 17px; }
.role-badge { display: inline-flex; align-items: center; height: 16px; padding: 0 5px; border-radius: 4px; background: var(--control); color: var(--secondary); font-size: 9px; line-height: 1; white-space: nowrap; }
.pod.is-live .role-badge { background: color-mix(in srgb, var(--accent) 10%, var(--surface)); color: var(--accent); }
.pod-state { display: inline-flex; align-items: center; gap: 4px; color: var(--secondary); font-size: 9px; white-space: nowrap; }
.pod-state i { width: 5px; height: 5px; border-radius: 50%; background: var(--success); }
.pod.is-live .pod-state { color: var(--success); }
.pod.is-live .pod-state i { background: var(--success); }
.pod.is-warn .pod-state { color: var(--warning); }
.pod.is-warn .pod-state i { background: var(--warning); }
.pod.is-idle .pod-state i { background: var(--secondary); }
.pod-icon { position: relative; width: 42px; height: 42px; flex: none; display: grid; place-items: center; border-radius: 10px; background: var(--control); color: var(--secondary); }
.pod.is-live .pod-icon { background: color-mix(in srgb, var(--accent) 9%, var(--surface)); color: var(--accent); }
.pod-badge { position: absolute; right: -3px; bottom: -3px; width: 16px; height: 16px; display: grid; place-items: center; border-radius: 50%; background: var(--surface); border: .5px solid var(--line); color: var(--secondary); }
.pod.is-live .pod-badge { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 38%, var(--line)); }
.pod.is-warn .pod-badge { color: var(--warning); border-color: color-mix(in srgb, var(--warning) 38%, var(--line)); }
.pod-badge.socket { box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 8%, transparent); }
.pod-device { display: flex; align-items: center; gap: 10px; margin-top: 11px; min-width: 0; }
.pod-device > div { min-width: 0; }
.pod-device b { display: block; font-size: 13px; font-weight: 600; line-height: 1.3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pod-device small { display: block; margin-top: 2px; font-size: 10px; line-height: 1.35; color: var(--secondary); white-space: normal; word-break: keep-all; }
.pod-meta { display: flex; align-items: center; gap: 6px; min-height: 25px; margin: 10px 0 0; padding-top: 9px; border-top: .5px solid var(--line); font-family: var(--font-mono); font-size: 9px; line-height: 1.4; color: var(--secondary); white-space: nowrap; overflow: hidden; font-variant-numeric: tabular-nums; }
.pod-meta span + span::before { content: "·"; margin: 0 4px; opacity: .55; }
.pod-meta span.warn { color: var(--warning); }
.link-meter { display: flex; align-items: center; justify-content: center; gap: 9px; }
.link-level { display: inline-flex; align-items: baseline; gap: 3px; white-space: nowrap; }
.link-level b { font-family: var(--font-mono); font-size: 15px; font-weight: 600; letter-spacing: -.03em; font-variant-numeric: tabular-nums; }
.link-level small { font-family: var(--font-mono); font-size: 8.5px; color: var(--secondary); }
.level-bars { display: flex; align-items: center; gap: 2px; width: 84px; flex: none; }
.level-bars i { flex: 1; height: 9px; border-radius: 1px; background: var(--control); transition: background-color 90ms linear; }
.level-bars i.on { background: var(--accent); }
.link-canvas { position: relative; min-height: 0; }
.dest-pill { display: inline-flex; align-items: center; gap: 4px; height: 16px; padding: 0 5px; border-radius: 4px; font-size: 9px; line-height: 1; white-space: nowrap; color: var(--success); background: color-mix(in srgb, var(--success) 10%, var(--surface)); }
.dest-pill i { width: 4px; height: 4px; border-radius: 50%; background: currentColor; }
.pod.is-warn .dest-pill { color: var(--warning); background: color-mix(in srgb, var(--warning) 10%, var(--surface)); }
.pod.is-idle .dest-pill { color: var(--secondary); background: var(--control); }
.app-read { display: inline-flex; align-items: center; gap: 6px; min-width: 0; font-size: 9.5px; color: var(--secondary); }
.app-read i { width: 5px; height: 5px; border-radius: 50%; background: var(--accent); flex: none; }
.pod.is-idle .app-read i { background: var(--secondary); }
.pod.is-warn .app-read i { background: var(--warning); }
.app-read span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

/* ---------- 设置 ---------- */
.setup {
  display: grid;
  grid-template-columns: 1.08fr .92fr;
  margin-top: 16px;
  overflow: hidden;
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius-card);
}
.setup-col { min-width: 0; padding: 14px 16px 12px; }
.setup-col + .setup-col { border-left: .5px solid var(--line); }
.section-label { margin-bottom: 6px; font-family: var(--font-mono); font-size: 9px; letter-spacing: .11em; color: var(--secondary); }
.setting {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-height: 39px;
  padding: 6px 0;
  text-align: left;
}
.setting + .setting { border-top: .5px solid var(--line); }
button.setting:hover .row-copy b { color: var(--accent); }
.row-icon { width: 27px; height: 27px; flex: none; display: grid; place-items: center; border-radius: 7px; background: var(--control); color: var(--secondary); }
.row-icon.tone-accent { color: var(--accent); background: color-mix(in srgb, var(--accent) 9%, var(--surface)); }
.row-icon.tone-warning { color: var(--warning); background: color-mix(in srgb, var(--warning) 9%, var(--surface)); }
.row-copy { flex: 1; min-width: 0; }
.row-copy b { display: block; font-size: 11.5px; font-weight: 500; line-height: 1.35; transition: color var(--dur-fast) var(--ease-out); }
.row-copy small { display: block; margin-top: 1px; font-size: 10px; color: var(--secondary); line-height: 1.4; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.row-action { flex: none; display: inline-flex; align-items: center; gap: 2px; padding: 4px 0 4px 8px; font-size: 10.5px; color: var(--accent); white-space: nowrap; }
.row-action:disabled { cursor: default; opacity: .45; }
.row-value { flex: none; display: inline-flex; align-items: center; gap: 4px; font-size: 10px; color: var(--secondary); }
.row-value.ok { color: var(--secondary); }
.row-value.ok svg { color: var(--success); }
.voice-test {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 2px;
  padding: 10px;
  border: .5px solid var(--line);
  border-radius: 9px;
  background: color-mix(in srgb, var(--accent) 2%, var(--surface));
  text-align: left;
  transition: border-color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out), transform var(--dur-fast) var(--ease-out);
}
.voice-test:hover { border-color: color-mix(in srgb, var(--accent) 28%, var(--line)); }
.voice-test:active { transform: scale(.995); }
.voice-test.listening { border-color: color-mix(in srgb, var(--accent) 40%, var(--line)); }
.vt-icon { width: 32px; height: 32px; flex: none; display: grid; place-items: center; border-radius: 50%; background: color-mix(in srgb, var(--accent) 9%, var(--surface)); color: var(--accent); }
.voice-test.listening .vt-icon { animation: vtPulse 1.6s ease-in-out infinite; }
@keyframes vtPulse { 0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 12%, transparent); } 50% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 0%, transparent); } }
.vt-copy { flex: 1; min-width: 0; }
.vt-copy b { display: block; font-size: 11.5px; font-weight: 500; }
.vt-copy small { display: block; margin-top: 1px; font-size: 10px; color: var(--secondary); }
.vt-action { flex: none; font-size: 10.5px; color: var(--accent); }
.speech-level { display: flex; align-items: center; gap: 2px; height: 18px; margin: 7px 1px 0; }
.speech-level span { flex: 1; height: 4px; border-radius: 1px; background: var(--control); }
.speech-level span.on { background: var(--accent); }
.speech-input { width: 100%; min-height: 48px; margin-top: 8px; padding: 7px 9px; font-size: 11px; line-height: 1.45; resize: vertical; }
.speech-input.listening { border-color: color-mix(in srgb, var(--accent) 40%, var(--line)); }
.speech-notice { margin-top: 8px; }
.privacy { display: flex; align-items: flex-start; gap: 5px; margin: 10px 0 0; font-size: 10px; line-height: 1.5; color: var(--secondary); }
.privacy svg { flex: none; margin-top: 1px; }
.page-foot { display: flex; align-items: center; gap: 5px; margin: 12px 2px 0; font-size: 10px; color: var(--secondary); }
.page-foot svg { flex: none; }
.page-error { margin-top: 10px; }
.setup :deep(.notice) { margin-top: 10px; }
.maintenance { margin-top: 10px; border-top: .5px solid var(--line); }
.maintenance summary { display: flex; align-items: center; gap: 6px; padding: 9px 0 4px; font-size: 10.5px; color: var(--secondary); cursor: pointer; list-style: none; }
.maintenance summary::-webkit-details-marker { display: none; }
.maintenance summary .chev { margin-left: auto; transition: transform var(--dur-fast) var(--ease-out); }
.maintenance[open] summary .chev { transform: rotate(90deg); }
.maintenance-body { display: grid; gap: 2px; padding: 3px 0 2px; }
.maintenance-row { display: flex; align-items: center; gap: 10px; padding: 7px 0; }
.maintenance-row + .maintenance-row { border-top: .5px solid var(--line); }
.maintenance-row > div { flex: 1; min-width: 0; }
.maintenance-row b { display: block; font-size: 11px; font-weight: 500; }
.maintenance-row small { display: block; margin-top: 1px; font-size: 10px; color: var(--secondary); line-height: 1.4; }
.danger-row b { color: var(--danger); }

/* ---------- 状态页 ---------- */
.state-scroll { display: flex; align-items: flex-start; justify-content: center; }
.state-page { width: min(760px, 100%); padding: 30px 24px; }
.state-top { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-bottom: 10px; border-bottom: .5px solid var(--line); color: var(--secondary); font-size: 9px; letter-spacing: .08em; }
.state-mark { font-family: var(--font-mono); font-size: 9px; color: var(--accent); }
.state-mark.error { color: var(--danger); }
.state-body { display: flex; align-items: center; gap: 14px; padding: 28px 0 20px; }
.state-glyph { width: 42px; height: 42px; flex: none; display: grid; place-items: center; border-radius: 11px; background: color-mix(in srgb, var(--accent) 8%, var(--surface)); color: var(--accent); }
.state-glyph.danger { color: var(--danger); background: color-mix(in srgb, var(--danger) 8%, var(--surface)); }
.state-copy h1 { font-size: 20px; }
.state-copy p { margin-top: 5px; color: var(--secondary); font-size: 12px; }
.state-spinner { margin-left: auto; }
.spinner { width: 14px; height: 14px; border-radius: 50%; border: 1.5px solid var(--line); border-top-color: var(--accent); animation: spin .8s linear infinite; flex: none; }
.state-actions { display: flex; gap: 8px; padding: 14px 0; }
.tech { margin: 4px 0 12px; }
.tech summary { display: flex; align-items: center; gap: 5px; width: fit-content; color: var(--secondary); font-size: 11px; cursor: pointer; list-style: none; }
.tech summary::-webkit-details-marker { display: none; }
.tech[open] .chev { transform: rotate(90deg); }
.tech .chev { transition: transform var(--dur-fast) var(--ease-out); }
.tech pre { margin-top: 8px; padding: 10px; border: .5px solid var(--line); border-radius: 8px; background: var(--control); color: var(--secondary); font-size: 10px; white-space: pre-wrap; overflow-wrap: anywhere; }
.privacy-line { display: flex; align-items: flex-start; gap: 6px; margin-top: 12px; color: var(--secondary); font-size: 10px; line-height: 1.5; }
.privacy-line svg { flex: none; margin-top: 1px; }
.line-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  min-height: 30px;
  padding: 0 10px;
  border: .5px solid var(--line);
  border-radius: 7px;
  background: var(--surface);
  color: var(--text);
  font-size: 11px;
  white-space: nowrap;
  transition: border-color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out);
}
.line-button:hover { border-color: var(--line-strong); background: var(--control); }
.line-button.primary { background: var(--accent); border-color: transparent; color: #fff; }
.line-button.primary:hover { background: color-mix(in srgb, var(--accent) 88%, #000); }
.line-button.small { min-height: 26px; padding: 0 8px; font-size: 10.5px; }
@keyframes spin { to { transform: rotate(360deg); } }

/* ---------- 选择弹层 ---------- */
.sheet-scrim {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: grid;
  place-items: center;
  padding: 20px;
  background: rgba(20, 22, 26, .28);
  backdrop-filter: blur(2px);
}
.sheet {
  width: min(430px, 100%);
  max-height: min(80vh, 620px);
  overflow: auto;
  padding: 20px;
  border-radius: 16px;
  background: var(--surface);
  border: 1px solid var(--line);
  box-shadow: 0 24px 70px rgba(0, 0, 0, .18);
}
/* 焦点落在对话框容器本身：读屏先播报标题，不画一圈不属于控件的焦点环。 */
.sheet:focus,
.sheet:focus-visible { outline: none; }
.sheet h2 { font-size: 17px; font-weight: 600; letter-spacing: -.01em; }
.sheet p { margin-top: 6px; font-size: 12px; line-height: 1.6; color: var(--secondary); }
.sheet p.sheet-warning { color: var(--danger); }
.sheet-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.sheet-head .icon-button { width: 26px; height: 26px; min-height: 26px; }
.picker-desc { margin-bottom: 10px; }
.pick-list { display: grid; gap: 2px; margin-top: 8px; }
.pick-row { display: flex; align-items: center; gap: 10px; width: 100%; min-height: 38px; padding: 0 9px; border-radius: 8px; text-align: left; }
.pick-row:hover { background: var(--control); }
.pick-row.selected { background: color-mix(in srgb, var(--accent) 7%, var(--surface)); }
.pick-icon { width: 26px; height: 26px; flex: none; display: grid; place-items: center; border-radius: 7px; background: var(--control); color: var(--secondary); }
.pick-row.selected .pick-icon { color: var(--accent); background: color-mix(in srgb, var(--accent) 10%, var(--surface)); }
.pick-name { flex: 1; min-width: 0; font-size: 12px; }
.pick-name small { display: inline-block; margin-left: 6px; font-size: 10px; color: var(--secondary); }
.pick-name small.online { color: var(--success); }
.pick-check { width: 13px; height: 13px; flex: none; border: 1px solid var(--line); border-radius: 50%; }
.pick-check.on { border-color: var(--accent); background: var(--accent); position: relative; }
.pick-check.on::after { content: ""; position: absolute; left: 3px; top: 1px; width: 4px; height: 7px; border: solid #fff; border-width: 0 1.5px 1.5px 0; transform: rotate(45deg); }
.peer-empty { display: flex; align-items: center; gap: 10px; margin-top: 12px; padding: 12px; border: .5px solid var(--line); border-radius: 9px; }
.peer-empty-icon { width: 32px; height: 32px; flex: none; display: grid; place-items: center; border-radius: 8px; background: var(--control); color: var(--secondary); }
.peer-empty-text { flex: 1; min-width: 0; }
.peer-empty-text strong { display: block; font-size: 11.5px; font-weight: 500; }
.peer-empty-text small { display: block; margin-top: 1px; font-size: 10px; color: var(--secondary); }
.sheet-check { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-top: 12px; font-size: 12px; }
.sheet-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
.sheet .button { min-height: 32px; font-size: 12px; }
.sheet .button.danger-solid { background: var(--danger); border-color: transparent; color: #fff; }
.sheet-enter-active, .sheet-leave-active { transition: opacity var(--dur-base) var(--ease-out); }
.sheet-enter-active .sheet, .sheet-leave-active .sheet { transition: transform var(--dur-base) var(--ease-out), opacity var(--dur-base) var(--ease-out); }
.sheet-enter-from, .sheet-leave-to { opacity: 0; }
.sheet-enter-from .sheet, .sheet-leave-to .sheet { transform: translateY(6px) scale(.99); opacity: 0; }

/* ---------- 响应式 ---------- */
@media (max-width: 1040px) {
  .mic-page { padding: 20px; }
  .stage-main { grid-template-columns: 184px minmax(0, 1fr) 184px; }
  .pod { width: 184px; padding: 10px 11px 9px; }
  .pod-device { gap: 8px; }
}
@media (max-width: 980px) {
  .role-choices { grid-template-columns: 1fr; }
  .role-choice { min-height: 0; }
}
@media (max-width: 760px) {
  .mic-page { padding: 16px 14px 24px; }
  .page-header { flex-direction: column; gap: 12px; margin-bottom: 12px; }
  .subtitle { font-size: 11.5px; }
  .header-actions { width: 100%; justify-content: space-between; gap: 8px; }
  .identity-switch { flex: 1; justify-content: center; }
  .picker-view { padding-top: 2px; }
  .picker-head h1 { font-size: 22px; }
  .picker-head p { font-size: 11.5px; }
  .role-choices { margin-top: 16px; gap: 10px; }
  .role-choice { padding: 14px; border-radius: 14px; }
  .choice-diagram { margin: 12px 0 14px; padding: 10px; gap: 7px; }
  .choice-icon { width: 26px; height: 26px; }
  .choice-title { font-size: 14px; }
  .choice-desc { font-size: 10.5px; }
  .choice-points span { font-size: 10px; }
  .choice-go { padding-top: 12px; }
  .preview-note { margin-top: 10px; }
  .stage { padding: 12px 12px 14px; }
  .stage-top { flex-wrap: wrap; row-gap: 5px; }
  .stage-meta { margin-left: 0; order: 3; width: 100%; }
  .stage-notice { flex-wrap: wrap; row-gap: 4px; }
  .stage-notice-copy { flex: 1 1 200px; }
  .stage-main { display: flex; flex-direction: column; align-items: stretch; min-height: 0; padding: 8px 0 0; gap: 0; }
  .pod, .pod-source, .pod-dest { width: 100%; max-width: 560px; margin-inline: auto; justify-self: auto; }
  .pod-source { order: 1; }
  .link { order: 2; width: calc(100% - 32px); max-width: 420px; height: 196px; padding-bottom: 0; margin: 0 auto; }
  .pod-dest { order: 3; }
  .pod-device { margin-top: 9px; }
  .pod-meta { margin-top: 9px; padding-top: 8px; }
  .setup { grid-template-columns: 1fr; margin-top: 12px; }
  .setup-col + .setup-col { border-left: 0; border-top: .5px solid var(--line); }
  .page-foot { margin-top: 10px; }
}
@media (max-width: 420px) {
  .picker-head h1 { font-size: 20px; }
  .choice-diagram { gap: 6px; padding: 9px; }
  .choice-endpoint { gap: 6px; }
  .choice-endpoint-copy b { font-size: 10px; max-width: 92px; }
  .choice-endpoint-copy small { font-size: 8.5px; }
  .choice-icon { width: 24px; height: 24px; }
  .choice-link { min-width: 28px; }
  .identity-switch { padding: 0 8px; font-size: 11px; }
  .stage-kicker { font-size: 9px; }
  .stage-status { font-size: 10px; }
  .stage-meta { font-size: 9px; }
  .level-bars { width: 58px; }
}
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation: none !important; transition: none !important; }
}
</style>

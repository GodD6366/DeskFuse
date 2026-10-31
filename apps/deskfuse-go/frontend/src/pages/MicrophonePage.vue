<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { backend, errorMessage } from '../services/backend'
import { friendlyError } from './features/core'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import Toggle from './features/Toggle.vue'
import FeatureGate from './features/FeatureGate.vue'

type Role = 'provider' | 'receiver' | ''
type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
type DriverAction = 'install' | 'uninstall' | 'restart-audio'

const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ (e: 'navigate', page: string): void }>()

const localId = computed(() => props.snapshot?.localId ?? props.snapshot?.local?.id ?? '')
const mic = computed<any>(() => props.snapshot?.microphone ?? null)
const settings = computed<any>(() => mic.value?.settings ?? {})
const nativeState = computed<any>(() => mic.value?.native ?? {})
const featureEnabled = computed(() => props.snapshot?.features?.microphone !== false)
const peers = computed<any[]>(() =>
  (props.snapshot?.devices ?? []).filter(
    (device: any) => device.trusted && !device.blocked && device.id !== localId.value,
  ),
)
const inputs = computed<any[]>(() => mic.value?.inputs ?? [])
const readerApps = computed<any[]>(() => mic.value?.reader_apps ?? [])

const localRole = ref<Role>('')
const editingRole = ref(false)
const source = ref('')
const inputUid = ref('')
const pendingSource = ref<string | null>(null)
const pendingInput = ref<string | null>(null)
const busy = ref(false)
const error = ref('')
const errorRaw = ref('')
const pendingEnabled = ref<boolean | null>(null)
const showSourcePicker = ref(false)
const driverAction = ref<DriverAction | null>(null)
const restartAudio = ref(true)
const speechText = ref('')
const speech = ref<any>({ phase: 'loading', listening: false, text: '', level: 0, input_name: null, on_device: false, problem: null })
let pendingTimer: ReturnType<typeof setTimeout> | undefined
let speechTimer: ReturnType<typeof setInterval> | undefined

const speechPhase = computed(() => String(speech.value?.phase ?? 'loading'))
const speechListening = computed(() => speech.value?.listening === true)
const speechLevel = computed(() => Math.max(0, Math.min(1, Number(speech.value?.level ?? 0))))
const speechInput = computed(() => String(speech.value?.input_name ?? '') || '系统默认输入')
const speechProblem = computed<any>(() => speech.value?.problem ?? null)
const speechStatus = computed(() => {
  switch (speechPhase.value) {
    case 'loading': return { title: '正在加载语音测试…', detail: '正在读取音频设备与识别服务，可继续操作页面。' }
    case 'authorizing': return { title: '正在确认权限…', detail: '首次使用会请求语音识别与麦克风权限。' }
    case 'listening': return { title: '正在聆听，再次点击停止', detail: `输入设备：${speechInput.value} · ${speech.value?.on_device ? '本机识别' : 'Apple 语音识别'} · 文字不保存` }
    default: return { title: '点击麦克风开始说话', detail: `输入设备：${speechInput.value} · 中文识别 · 文字不保存` }
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
function clearSpeech() { speechText.value = ''; void speechInvoke('microphone.speech.clear') }
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
    pendingEnabled.value = null
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
watch(
  () => mic.value?.settings?.enabled,
  (value) => {
    if (pendingEnabled.value !== null && value === pendingEnabled.value) pendingEnabled.value = null
  },
)

const resolvedRole = computed<Role>(() => {
  if (localRole.value) return localRole.value
  const state = mic.value
  if (!state) return ''
  if (String(state.settings?.source ?? '')) return 'receiver'
  const preferred = String(state.preferred_role ?? '')
  if (preferred === 'provider' || preferred === 'receiver') return preferred
  if (state.settings?.enabled || (state.settings?.allowed ?? []).length) return 'provider'
  return ''
})
const role = computed<Role>(() => (editingRole.value ? '' : resolvedRole.value))
const configured = computed(() => Boolean(role.value))

const permission = computed(() => String(nativeState.value.permission ?? ''))
const phase = computed(() => String(mic.value?.phase ?? ''))
const enabled = computed(() => pendingEnabled.value ?? Boolean(settings.value.enabled))
const driverInstalled = computed(() => Boolean(mic.value?.driver_installed))
const legacyDriverInstalled = computed(() => Boolean(mic.value?.legacy_driver_installed))
const nativeDriver = computed(() => Boolean(nativeState.value.driver))
const defaultInput = computed(() => mic.value?.is_default_input === true)
const managingDriver = computed(() => Boolean(mic.value?.managing_driver))
const onlinePeerCount = computed(() => peers.value.filter((peer) => peer.connected).length)
const sourcePeer = computed(() => peers.value.find((peer) => peer.id === source.value) ?? null)
const sourceConnected = computed(() => sourcePeer.value?.connected === true)
const level = computed(() => Number(nativeState.value.level ?? 0))

const sourcePeerName = computed(() =>
  !source.value ? '选择来源设备' : sourcePeer.value?.name ?? '原来源设备不可用',
)
const inputDisplayName = computed(() =>
  !inputUid.value
    ? '系统默认麦克风'
    : inputs.value.find((input) => input.id === inputUid.value)?.name ?? '所选麦克风不可用',
)
const activePeerName = computed(() => {
  const id = String(mic.value?.peer ?? '')
  if (!id) return null
  return peers.value.find((peer) => peer.id === id)?.name ?? '已连接设备'
})

const ready = computed(() => {
  if (!enabled.value || nativeState.value.locked || phase.value !== 'standby') return false
  if (role.value === 'provider') return permission.value === 'authorized' && onlinePeerCount.value > 0
  if (role.value === 'receiver')
    return driverInstalled.value && nativeDriver.value && sourceConnected.value
  return false
})

const hasIssue = computed(() => {
  if (!enabled.value || phase.value === 'streaming') return false
  if (phase.value === 'failed') return true
  if (role.value === 'provider') return permission.value !== 'authorized'
  if (role.value === 'receiver')
    return !source.value || !driverInstalled.value || !nativeDriver.value
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

const signalMode = computed(() => {
  if (!enabled.value) return 'off'
  if (phase.value === 'streaming') return 'streaming'
  if (connectionTone.value === 'warning') return 'attention'
  if (ready.value) return 'ready'
  return 'waiting'
})

const roleDetail = computed(() => {
  if (role.value === 'provider') {
    if (permission.value !== 'authorized') {
      return enabled.value
        ? '需要麦克风权限才能提供声音。'
        : '先允许 DeskFuse 读取此 Mac 的麦克风，再开启共享。'
    }
    if (!peers.value.length) return '配对电脑后即可直接使用，不需要逐台授权。'
    const active = activePeerName.value
    if (active) {
      const action = phase.value === 'streaming' ? '正在使用' : '已连接'
      return `${peers.value.length} 台已配对设备 · ${active} ${action}`
    }
    if (enabled.value) {
      return onlinePeerCount.value > 0
        ? `${peers.value.length} 台已配对设备 · ${onlinePeerCount.value} 台在线，可直接使用`
        : `${peers.value.length} 台已配对设备 · 等待上线`
    }
    return `${peers.value.length} 台已配对设备可直接使用；打开开关即可。`
  }
  if (role.value === 'receiver') {
    if (!source.value) return '选择提供麦克风的 Mac，再安装虚拟麦克风。'
    const name = sourcePeerName.value
    if (!driverInstalled.value) return `来源：${name} · 尚未安装虚拟麦克风`
    if (!nativeDriver.value) return `来源：${name} · 等待驱动就绪`
    if (!defaultInput.value) return `来源：${name} · 请设为系统默认输入`
    if (!enabled.value) return `来源：${name} · 设置已完成，打开开关即可使用`
    return `来源：${name} · 已是系统默认输入`
  }
  return ''
})

const routeSource = computed(() =>
  role.value === 'provider'
    ? { icon: 'mic', title: inputDisplayName.value, subtitle: '点击选择麦克风' }
    : { icon: 'laptop', title: sourcePeerName.value, subtitle: '声音来源' },
)

const routeDestination = computed(() => {
  if (role.value === 'provider') {
    const count = peers.value.length
    const active = activePeerName.value
    const title =
      active ??
      (count === 0 ? '等待配对设备' : count === 1 ? '1 台已配对电脑' : `${count} 台已配对电脑`)
    let subtitle: string
    if (active) subtitle = phase.value === 'streaming' ? `正在使用 · 共 ${count} 台` : `已连接 · 共 ${count} 台`
    else if (count === 0) subtitle = '先在设备中配对'
    else if (!enabled.value) subtitle = '已配对 · 共享未开启'
    else subtitle = onlinePeerCount.value > 0 ? '无需授权 · 按需连接' : '等待设备上线'
    return { icon: 'laptop', title, subtitle }
  }
  return {
    icon: 'waveform',
    title: driverInstalled.value ? 'DeskFuse 麦克风' : '虚拟麦克风',
    subtitle: driverInstalled.value ? '此 Mac 的虚拟输入' : '需要安装驱动',
  }
})

const usageSymbol = computed(() => {
  if (phase.value === 'streaming') return 'waveform'
  if (hasIssue.value) return 'pause-circle'
  if (ready.value) return 'moon'
  return 'mic'
})

const usageTitle = computed(() => {
  if (!enabled.value) return '共享已关闭'
  if (hasIssue.value) return '暂不可用'
  if (role.value === 'provider') {
    if (phase.value === 'streaming') return `${activePeerName.value ?? '另一台 Mac'} 正在使用麦克风`
    if (!peers.value.length) return '等待配对设备'
    if (onlinePeerCount.value === 0) return '等待已配对设备上线'
    if (ready.value) {
      return peers.value.length === 1
        ? '已就绪，等待设备使用'
        : `已就绪，${peers.value.length} 台已配对设备可直接使用`
    }
    if (phase.value === 'waiting') return '设备正在连接'
    if (nativeState.value.locked) return '已暂停共享'
    return '等待已配对设备使用'
  }
  if (role.value === 'receiver') {
    if (phase.value === 'streaming') {
      const names = [...new Set(readerApps.value.map((app) => String(app.name)))].sort()
      return names.length ? `${names.join('、')}正在使用` : '正在使用远端麦克风'
    }
    if (ready.value) return '已就绪，等待语音应用使用'
    if (!sourceConnected.value) return '来源设备离线'
    if (phase.value === 'waiting') return '正在连接来源设备'
    if (nativeState.value.locked) return '已暂停共享'
    return '等待来源设备'
  }
  return ''
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
  return micFriendly(String(mic.value?.detail ?? ''))
})

const soundSettingsReader = computed(() =>
  readerApps.value.some((app) => /sound settings|声音设置/i.test(String(app.name ?? ''))),
)

const usageDetail = computed(() => {
  if (!enabled.value) {
    return role.value === 'provider'
      ? '开启后，仅在设备使用麦克风时采集和传输。'
      : '开启后，仅在语音应用读取虚拟麦克风时连接来源设备。'
  }
  if (hasIssue.value) {
    return role.value === 'receiver'
      ? '需要处理的问题已列在右侧设置步骤中，处理后自动恢复。'
      : '需要处理的问题已列在下方，处理后自动恢复。'
  }
  if (role.value === 'provider') {
    if (phase.value === 'streaming') {
      return peers.value.length > 1
        ? '应用停止读取后自动结束。其他已配对设备需等本次使用结束。'
        : '应用停止读取后自动结束采集与传输。'
    }
    if (!peers.value.length) return '先在“设备”中配对另一台电脑；配对后可直接使用，不需要逐台授权。'
    if (onlinePeerCount.value === 0) return '已配对的电脑上线后即可直接使用，不需要逐台授权。'
    if (ready.value) {
      return peers.value.length > 1
        ? `当前没有采集或传输音频。${peers.value.length} 台已配对设备中，先开始的设备会建立连接。`
        : '当前没有采集或传输音频。设备开始使用麦克风时自动连接。'
    }
    return statusText.value
  }
  if (role.value === 'receiver') {
    if (soundSettingsReader.value)
      return '系统声音设置正在检测输入电平。离开该页面后，若没有其他应用使用，传输会自动停止。'
    if (phase.value === 'streaming') return '语音应用停止读取后自动结束采集与传输。'
    if (ready.value) return '当前没有采集或传输音频。语音应用开始录音时自动连接。'
    return statusText.value
  }
  return ''
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
  if (!defaultInput.value) return { title: '默认输入待设置', tone: 'warning', icon: 'alert' }
  return null
})

const driverButton = computed<{ title: string; action: DriverAction | 'set-default' } | null>(() => {
  if (managingDriver.value) return null
  if (legacyDriverInstalled.value || !driverInstalled.value)
    return { title: legacyDriverInstalled.value ? '更新驱动…' : '安装驱动…', action: 'install' }
  if (!nativeDriver.value) return { title: '重启音频服务…', action: 'restart-audio' }
  if (!defaultInput.value) return { title: '设为默认输入', action: 'set-default' }
  return null
})

const levelBars = Array.from({ length: 12 }, (_, index) => 8 + (index % 4) * 4)
const levelFraction = computed(() => Math.sqrt(Math.max(0, Math.min(1, level.value))))

const micError = computed(() =>
  String(mic.value?.error ?? props.snapshot?.microphone_error ?? '').trim(),
)
const micErrorFriendly = computed(() => micFriendly(micError.value))
const micErrorDetail = computed(() =>
  micError.value && micError.value !== micErrorFriendly.value ? micError.value : '',
)

const transient = computed(() =>
  enabled.value && phase.value === 'failed'
    ? { title: '连接暂时中断', message: statusText.value }
    : null,
)

const driverTitle = computed(() => driverTitleFor(driverAction.value))

function driverTitleFor(action: DriverAction | null): string {
  if (action === 'install') return '安装麦克风驱动'
  if (action === 'uninstall') return '卸载麦克风驱动'
  return '重启音频服务'
}

function micFriendly(raw: string): string {
  const text = raw.trim()
  if (!text) return '麦克风服务暂时不可用，请稍后重试。'
  const lower = text.toLowerCase()
  if (/deadline|timed out|timeout/.test(lower)) return '麦克风服务响应超时，请稍后重试。'
  if (/connection refused|econnrefused/.test(lower)) return '无法连接麦克风服务，请确认后台服务正在运行。'
  if (/connection reset|econnreset|broken pipe/.test(lower)) return '麦克风服务连接已断开，请重试。'
  if (/not installed/.test(lower)) return '虚拟麦克风驱动尚未安装。'
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
  const previous = resolvedRole.value
  localRole.value = next
  editingRole.value = false
  if (next === 'provider') {
    source.value = ''
    pendingSource.value = null
  }
  const ok = await run('microphone.chooseRole', { role: next })
  if (!ok) localRole.value = previous
}

function cancelIdentity() {
  editingRole.value = false
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

function setEnabled(value: boolean) {
  pendingEnabled.value = value
  startPendingWindow()
  void run('feature.setEnabled', { feature: 'microphone', enabled: value }).then((ok) => {
    if (!ok) pendingEnabled.value = null
  })
}

function requestDriver(action: DriverAction) {
  driverAction.value = action
  restartAudio.value = action === 'install'
}

function driverButtonClick() {
  const button = driverButton.value
  if (!button) return
  if (button.action === 'set-default') {
    void run('microphone.setDefaultInput')
    return
  }
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

function goDevices() {
  emit('navigate', 'devices')
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  showSourcePicker.value = false
  driverAction.value = null
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  prepareSpeech()
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  if (pendingTimer) clearTimeout(pendingTimer)
  stopSpeechPolling()
  // 离开页面即释放录音与识别资源，避免后台继续占用麦克风。
  if (speechListening.value) void speechInvoke('microphone.speech.stop')
})
</script>

<template>
  <div class="microphone-page">
    <FeatureGate
      v-if="snapshot && !featureEnabled"
      page="microphone"
      title="麦克风共享"
    />

    <div v-else-if="!snapshot" class="mic-scroll">
      <div class="focus loading-focus">
        <section class="card loading-card">
          <span class="loading-icon"><Icon name="mic" :size="24" /></span>
          <div class="loading-text">
            <h2>正在准备麦克风共享</h2>
            <p>读取系统输入、已配对设备和驱动状态，首次打开可能需要几秒钟。</p>
          </div>
        </section>
      </div>
    </div>

    <template v-else-if="!mic">
      <div class="mic-scroll">
        <div class="focus loading-focus">
          <section v-if="micError" class="card error-card">
            <div class="error-head">
              <span class="error-icon"><Icon name="mic-off" :size="18" /></span>
              <div class="error-text">
                <h2>无法连接麦克风服务</h2>
                <p>{{ micErrorFriendly }}</p>
              </div>
            </div>
            <details v-if="micErrorDetail" class="tech">
              <summary>技术详情</summary>
              <pre>{{ micErrorDetail }}</pre>
            </details>
            <div class="error-actions">
              <button class="button primary" :disabled="busy" @click="run('microphone.retry')">
                重新连接
              </button>
            </div>
          </section>
          <section v-else class="card loading-card">
            <span class="loading-icon"><Icon name="mic" :size="24" /></span>
            <div class="loading-text">
              <h2>正在准备麦克风共享</h2>
              <p>读取系统输入、已配对设备和驱动状态，首次打开可能需要几秒钟。</p>
            </div>
            <span class="spinner" aria-label="正在加载" />
          </section>
          <div class="privacy">
            <Icon name="shield" :size="12" />
            <span>仅可信的已配对设备可以使用；DeskFuse 不保存或转写音频，只在应用读取麦克风时采集和传输。</span>
          </div>
        </div>
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
    </template>

    <template v-else-if="!configured">
      <div class="mic-scroll">
        <div class="focus identity-focus">
          <div class="identity-head">
            <div>
              <h1>这台 Mac 要怎么参与？</h1>
              <p>先选择身份，再完成对应的设置。之后可以随时更改。</p>
            </div>
            <button v-if="resolvedRole" type="button" class="df-link" @click="cancelIdentity">
              取消
            </button>
          </div>

          <div class="role-grid">
            <button type="button" class="identity-role" @click="chooseRole('provider')">
              <div class="role-top">
                <span class="role-icon"><Icon name="mic" :size="24" /></span>
                <Icon name="arrow-right" :size="14" class="role-arrow" />
              </div>
              <div class="role-copy">
                <h2>我提供麦克风</h2>
                <p>把此 Mac 连接的麦克风分享给已配对的电脑。可信设备可直接使用，不需要逐台授权。</p>
              </div>
              <div class="role-foot success">
                <Icon name="check-circle" :size="12" />无需逐台授权
              </div>
            </button>
            <button type="button" class="identity-role" @click="chooseRole('receiver')">
              <div class="role-top">
                <span class="role-icon"><Icon name="laptop" :size="24" /></span>
                <Icon name="arrow-right" :size="14" class="role-arrow" />
              </div>
              <div class="role-copy">
                <h2>我使用其他设备的麦克风</h2>
                <p>使用另一台 Mac 的麦克风，在本机听写、通话或录音。</p>
              </div>
              <div class="role-foot warning">
                <Icon name="download-circle" :size="12" />需要安装虚拟麦克风驱动
              </div>
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
            v-else
            tone="neutral"
            title="开始前请确认"
            message="两台 Mac 需要先完成配对并保持可信。音频只在应用读取麦克风时传输，不会保存。"
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
        </div>
      </div>
    </template>

    <template v-else>
      <div class="mic-scroll">
        <div class="focus">
          <section class="card role-card">
            <div class="role-head">
              <span class="role-symbol"
                ><Icon :name="role === 'provider' ? 'mic' : 'laptop'" :size="20"
              /></span>
              <div class="role-headings">
                <div class="role-title-row">
                  <h2>{{ role === 'provider' ? '提供麦克风' : '接收麦克风' }}</h2>
                  <span class="role-badge">{{ role === 'provider' ? '提供端' : '接收端' }}</span>
                </div>
                <p>{{ roleDetail }}</p>
              </div>
              <button type="button" class="df-link" @click="editingRole = true">更换身份</button>
            </div>
            <div class="divider" />

            <div class="stage" :class="`tone-${connectionTone}`">
              <div class="node" :class="{ active: signalMode === 'ready' || signalMode === 'streaming' }">
                <button
                  type="button"
                  class="node-button"
                  :aria-label="role === 'provider' ? '选择麦克风' : '选择来源设备'"
                  :title="role === 'provider' ? '点击选择麦克风' : '点击选择来源设备'"
                  @click="toggleSourcePicker"
                >
                  <span class="node-circle"><Icon :name="routeSource.icon" :size="24" /></span>
                </button>
                <div class="node-sub">
                  <span>{{ routeSource.subtitle }}</span>
                  <Icon name="chevron-down" :size="10" />
                </div>
                <div class="node-title">{{ routeSource.title }}</div>
              </div>

              <div class="signal">
                <div class="track" :class="signalMode" />
                <div class="signal-pill">{{ connectionLabel }}</div>
                <div class="signal-note">音频不保存</div>
              </div>

              <div class="node" :class="{ active: signalMode === 'ready' || signalMode === 'streaming' }">
                <span class="node-circle"><Icon :name="routeDestination.icon" :size="24" /></span>
                <div class="node-sub">{{ routeDestination.subtitle }}</div>
                <div class="node-title">{{ routeDestination.title }}</div>
                <div v-if="role === 'receiver'" class="driver-controls">
                  <template v-if="managingDriver">
                    <span class="spinner" />
                    <small>正在处理驱动…</small>
                    <small>请完成系统管理员授权</small>
                  </template>
                  <template v-else>
                    <small
                      v-if="driverStatus"
                      class="driver-status"
                      :class="`tone-${driverStatus.tone}`"
                      ><Icon :name="driverStatus.icon" :size="11" />{{ driverStatus.title }}</small
                    >
                    <button
                      v-if="driverButton"
                      type="button"
                      class="button primary driver-button"
                      :disabled="busy"
                      @click="driverButtonClick"
                    >
                      {{ driverButton.title }}
                    </button>
                  </template>
                </div>
              </div>

              <template v-if="showSourcePicker">
                <div class="pop-backdrop" @click="showSourcePicker = false" />
                <div class="source-pop" role="dialog" aria-label="选择来源">
                  <div class="source-pop-head">
                    {{ role === 'provider' ? '选择麦克风' : '选择来源设备' }}
                  </div>
                  <template v-if="role === 'provider'">
                    <button
                      type="button"
                      class="pick-row"
                      :class="{ selected: inputUid === '' }"
                      @click="chooseInput('')"
                    >
                      <span class="pick-name">系统默认麦克风</span>
                      <span class="pick-check" :class="{ on: inputUid === '' }" />
                    </button>
                    <button
                      v-for="input in inputs"
                      :key="input.id"
                      type="button"
                      class="pick-row"
                      :class="{ selected: inputUid === input.id }"
                      @click="chooseInput(input.id)"
                    >
                      <span class="pick-name">{{ input.name }}</span>
                      <span class="pick-check" :class="{ on: inputUid === input.id }" />
                    </button>
                    <div
                      v-if="inputUid && !inputs.some((input) => input.id === inputUid)"
                      class="pick-row selected"
                    >
                      <span class="pick-name">所选麦克风不可用</span>
                      <span class="pick-check on" />
                    </div>
                  </template>
                  <template v-else>
                    <p class="source-pop-desc">选择提供麦克风的 Mac。已配对设备可直接使用。</p>
                    <div v-if="!peers.length" class="peer-empty">
                      <span class="peer-empty-icon"><Icon name="laptop" :size="18" /></span>
                      <div class="peer-empty-text">
                        <strong>还没有已配对的电脑</strong>
                        <small>先在“设备”中完成配对；之后无需额外授权。</small>
                      </div>
                      <button type="button" class="button" @click="goDevices">前往设备</button>
                    </div>
                    <div v-else class="peer-list">
                      <button
                        v-for="peer in peers"
                        :key="peer.id"
                        type="button"
                        class="peer-row"
                        :class="{ selected: source === peer.id }"
                        @click="chooseSource(peer.id)"
                      >
                        <Icon
                          name="laptop"
                          :size="16"
                          class="peer-icon"
                          :class="{ selected: source === peer.id }"
                        />
                        <span class="peer-text">
                          <strong>{{ peer.name || '未命名设备' }}</strong>
                          <small>{{ peer.connected ? '在线' : '离线' }}</small>
                        </span>
                        <span class="pick-check" :class="{ on: source === peer.id }" />
                      </button>
                    </div>
                  </template>
                </div>
              </template>
            </div>
            <div class="divider" />

            <div class="usage">
              <span class="usage-icon" :class="`tone-${connectionTone}`">
                <Icon :name="usageSymbol" :size="16" />
              </span>
              <div class="usage-copy">
                <h2>{{ usageTitle }}</h2>
                <p>{{ usageDetail }}</p>
              </div>
              <span v-if="phase === 'streaming'" class="level" aria-label="麦克风电平">
                <span
                  v-for="(bar, index) in levelBars"
                  :key="index"
                  :style="{ height: `${bar}px` }"
                  :class="{ on: index / levelBars.length < levelFraction }"
                />
              </span>
              <Toggle
                :model-value="enabled"
                label="启用麦克风共享"
                :disabled="busy"
                @update:model-value="setEnabled"
              />
            </div>
          </section>

          <Notice
            v-if="role === 'provider' && permission !== 'authorized'"
            :tone="permission === 'denied' ? 'warning' : 'accent'"
            :title="permission === 'denied' ? '麦克风权限已关闭' : '需要允许麦克风访问'"
            message="macOS 会询问是否允许 DeskFuse 读取实体麦克风。"
            :action-title="permission === 'denied' ? '打开系统设置…' : '允许访问…'"
            @action="fixPermission"
          />

          <Notice
            v-if="role === 'receiver' && source && !sourcePeer && peers.length"
            tone="warning"
            title="原来源设备不可用"
            message="重新配对来源设备，或点击“声音来源”换一台 Mac。"
          />

          <section class="card speech">
            <div class="speech-head">
              <h2><Icon name="text" :size="15" />语音输入测试</h2>
              <p>
                点击麦克风，测试{{
                  role === 'receiver' ? '远端声音输入' : '本机默认输入'
                }}与实时识别。
              </p>
            </div>
            <textarea
              v-model="speechText"
              class="speech-input"
              :class="{ listening: speechListening }"
              rows="2"
              :readonly="speechListening"
              :placeholder="speechListening ? '正在聆听，请说话…' : '识别结果，也可直接输入'"
              aria-label="语音识别结果"
            />
            <Notice
              v-if="speechProblem"
              tone="warning"
              dismissible
              :title="String(speechProblem.title ?? '语音测试不可用')"
              :message="String(speechProblem.message ?? '')"
              :action-title="speechProblem.settings_url ? '打开系统设置' : undefined"
              @action="openSpeechSettings"
              @dismiss="dismissSpeechProblem"
            />
            <div class="speech-controls">
              <button
                type="button"
                class="mic-button"
                :class="{ listening: speechListening }"
                :disabled="speechPhase === 'loading' || speechPhase === 'authorizing'"
                :title="speechListening ? '停止语音输入' : '开始语音输入'"
                :aria-label="speechListening ? '停止语音输入' : '开始语音输入'"
                :aria-pressed="speechListening"
                @click="speechListening ? stopSpeech() : toggleSpeech()"
              >
                <span
                  v-if="speechListening"
                  class="mic-halo"
                  :style="{ transform: `scale(${1 + speechLevel * 0.45})` }"
                  aria-hidden="true"
                />
                <Icon :name="speechListening ? 'square' : 'mic'" :size="18" />
              </button>
              <div class="speech-copy">
                <strong>{{ speechStatus.title }}</strong>
                <small>{{ speechStatus.detail }}</small>
              </div>
              <span class="df-spacer" />
              <div v-if="speechListening" class="speech-level" aria-label="输入电平">
                <span v-for="index in 12" :key="index" :class="{ on: speechLevel * 12 >= index }" />
              </div>
              <button
                type="button"
                class="df-link"
                :disabled="!speechText || speechListening"
                @click="clearSpeech"
              >
                清空
              </button>
            </div>
          </section>

          <details
            v-if="role === 'receiver' && (driverInstalled || legacyDriverInstalled)"
            class="maintenance"
          >
            <summary>
              <Icon name="wrench" :size="13" />
              <span>驱动维护</span>
              <Icon name="chevron-right" :size="12" class="chev" />
            </summary>
            <div class="maintenance-body">
              <template v-if="driverInstalled">
                <div class="setting-row">
                  <div>
                    <b>音频服务</b>
                    <small>连接异常时先重启，保留驱动与共享设置。</small>
                  </div>
                  <button type="button" class="button" @click="requestDriver('restart-audio')">
                    重启…
                  </button>
                </div>
                <div class="divider" />
                <div class="setting-row">
                  <div>
                    <b>麦克风驱动</b>
                    <small>重启后仍不可用时，可尝试重新安装。</small>
                  </div>
                  <button type="button" class="button" @click="requestDriver('install')">
                    重新安装…
                  </button>
                </div>
                <div class="divider" />
              </template>
              <div class="setting-row">
                <div>
                  <b>移除驱动</b>
                  <small>移除后，此 Mac 将无法接收远端麦克风。</small>
                </div>
                <button type="button" class="button danger" @click="requestDriver('uninstall')">
                  {{ legacyDriverInstalled ? '卸载驱动（含旧版）…' : '卸载…' }}
                </button>
              </div>
            </div>
          </details>

          <Notice
            v-if="error"
            tone="danger"
            title="操作未完成"
            :message="error"
            :raw="errorRaw || undefined"
            dismissible
            @dismiss="error = ''"
          />
        </div>
      </div>

      <div v-if="transient" class="transient">
        <Notice tone="warning" :title="transient.title" :message="transient.message" />
      </div>
    </template>

    <Teleport to="body">
      <div v-if="driverAction" class="sheet-scrim" @mousedown.self="driverAction = null">
        <div class="sheet" role="dialog" aria-modal="true" :aria-label="driverTitle">
          <h2>{{ driverTitle }}</h2>
          <template v-if="driverAction === 'restart-audio'">
            <p>这会短暂中断此 Mac 上所有应用的录音和播放，需要管理员授权。</p>
            <p class="secondary">
              保留已安装的驱动和当前共享设置。服务恢复后按需连接，语音应用可能需要重新开始录音。
            </p>
          </template>
          <template v-else>
            <p>
              仅操作 DeskFuse 的麦克风驱动（包括旧版名称遗留的驱动），需要在系统弹窗中授权管理员权限。操作前会关闭麦克风共享。
            </p>
            <template v-if="driverAction === 'install'">
              <label class="sheet-check">
                <span>完成后立即重启音频服务</span>
                <Toggle
                  :model-value="restartAudio"
                  label="完成后立即重启音频服务"
                  @update:model-value="restartAudio = $event"
                />
              </label>
              <p class="secondary">
                {{ restartAudio ? '这会短暂中断此 Mac 上所有应用的录音和播放。' : '稍后重启 Mac 后生效。' }}
              </p>
            </template>
          </template>
          <div class="sheet-actions">
            <button type="button" class="button" @click="driverAction = null">取消</button>
            <button
              type="button"
              class="button"
              :class="driverAction === 'uninstall' ? 'danger-solid' : 'primary'"
              :disabled="busy"
              @click="confirmDriver"
            >
              {{ driverTitle }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.microphone-page {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  position: relative;
}

.mic-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 28px 24px 24px;
}

.focus {
  width: 100%;
  max-width: 880px;
  display: grid;
  gap: 16px;
}

.loading-focus {
  gap: 24px;
}

.identity-focus {
  gap: 16px;
}

.identity-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.identity-head h1 {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 4px;
}

.identity-head p {
  font-size: 12px;
  color: var(--secondary);
}

.role-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.identity-role {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 196px;
  padding: 24px;
  text-align: left;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 12px;
  color: var(--text);
  transition: border-color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease;
}

.identity-role:hover {
  border-color: color-mix(in srgb, var(--accent) 65%, transparent);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.12);
  transform: translateY(-2px);
}

.role-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.role-icon {
  width: 48px;
  height: 48px;
  display: grid;
  place-items: center;
  border-radius: 12px;
  background: var(--control);
  color: var(--accent);
  transition: background 0.18s ease, color 0.18s ease;
}

.identity-role:hover .role-icon {
  background: var(--accent);
  color: #fff;
}

.role-arrow {
  color: var(--secondary);
  transition: color 0.18s ease, transform 0.18s ease;
}

.identity-role:hover .role-arrow {
  color: var(--accent);
  transform: translateX(4px);
}

.role-copy {
  display: grid;
  gap: 4px;
}

.role-copy h2 {
  font-size: 15px;
  font-weight: 600;
}

.role-copy p {
  font-size: 12px;
  color: var(--secondary);
  line-height: 1.5;
}

.role-foot {
  margin-top: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 500;
}

.role-foot.success {
  color: var(--success);
}

.role-foot.warning {
  color: var(--warning);
}

.card {
  padding: 16px;
}

.role-card {
  padding: 0;
  overflow: visible;
}

.role-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 24px;
}

.role-symbol {
  width: 40px;
  height: 40px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: var(--control);
  color: var(--accent);
}

.role-headings {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.role-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.role-title-row h2 {
  font-size: 15px;
  font-weight: 600;
}

.role-badge {
  font-size: 11px;
  font-weight: 500;
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--control);
  color: var(--secondary);
}

.role-headings p {
  font-size: 12px;
  color: var(--secondary);
}

.divider {
  height: 0;
  border-top: 0.5px solid var(--line);
}

.stage {
  position: relative;
  display: grid;
  grid-template-columns: 140px minmax(0, 1fr) 140px;
  align-items: start;
  gap: 8px;
  padding: 28px 24px;
}

.tone-neutral {
  --tone: var(--secondary);
}

.tone-accent {
  --tone: var(--accent);
}

.tone-success {
  --tone: var(--success);
}

.tone-warning {
  --tone: var(--warning);
}

.tone-danger {
  --tone: var(--danger);
}

.node {
  display: grid;
  justify-items: center;
  gap: 8px;
  width: 140px;
}

.node-button {
  padding: 0;
  border-radius: 50%;
}

.node-circle {
  width: 64px;
  height: 64px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  background: var(--control);
  border: 0.5px solid var(--line);
  color: var(--secondary);
  transition: background 0.18s ease, border-color 0.18s ease, color 0.18s ease;
}

.node.active .node-circle {
  background: color-mix(in srgb, var(--tone) 12%, transparent);
  border-color: color-mix(in srgb, var(--tone) 25%, transparent);
  color: var(--tone);
}

.node-button:hover .node-circle {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}

.node-sub {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--secondary);
}

.node-title {
  max-width: 140px;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.3;
  text-align: center;
  overflow-wrap: anywhere;
}

.signal {
  display: grid;
  justify-items: center;
  gap: 8px;
  min-width: 0;
  padding-top: 0;
}

.track {
  width: 100%;
  height: 64px;
  position: relative;
}

.track::before {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  top: 31px;
  border-top: 1.5px dashed color-mix(in srgb, var(--tone) 45%, transparent);
}

.track.off::before {
  border-top-color: var(--line);
}

.track.attention::before {
  border-top-color: color-mix(in srgb, var(--tone) 50%, transparent);
}

.track.waiting::before {
  animation: dash-flow 3s linear infinite;
}

.track.ready::before {
  border-top-style: solid;
  border-top-color: color-mix(in srgb, var(--tone) 30%, transparent);
}

.track.streaming::before {
  border-top-style: solid;
  border-top-width: 2px;
  border-top-color: var(--tone);
}

@keyframes dash-flow {
  to {
    transform: translateX(10px);
  }
}

.signal-pill {
  padding: 4px 10px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  color: var(--tone);
  background: color-mix(in srgb, var(--tone) 12%, transparent);
  border: 0.5px solid color-mix(in srgb, var(--tone) 25%, transparent);
}

.signal-note {
  font-size: 11px;
  color: var(--secondary);
}

.driver-controls {
  display: grid;
  justify-items: center;
  gap: 6px;
  max-width: 140px;
  text-align: center;
}

.driver-controls small {
  font-size: 11px;
  line-height: 1.35;
}

.driver-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-weight: 500;
  color: var(--tone);
}

.driver-status.tone-accent {
  --tone: var(--accent);
}

.driver-status.tone-warning {
  --tone: var(--warning);
}

.driver-button {
  min-height: 30px;
  height: 30px;
  padding: 0 10px;
  font-size: 12px;
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

.pop-backdrop {
  position: fixed;
  inset: 0;
  z-index: 40;
}

.source-pop {
  position: absolute;
  z-index: 41;
  top: 92px;
  left: 24px;
  width: min(320px, calc(100% - 48px));
  max-height: 340px;
  overflow: auto;
  padding: 16px;
  display: grid;
  gap: 8px;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 12px;
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.2);
}

.source-pop-head {
  font-size: 15px;
  font-weight: 600;
}

.source-pop-desc {
  font-size: 12px;
  color: var(--secondary);
}

.pick-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 40px;
  padding: 6px 0;
  text-align: left;
  border-bottom: 0.5px solid var(--line);
}

.pick-row:last-child {
  border-bottom: 0;
}

.pick-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pick-check {
  width: 16px;
  height: 16px;
  flex: none;
  border-radius: 50%;
  border: 1.5px solid color-mix(in srgb, var(--secondary) 50%, transparent);
  position: relative;
}

.pick-check.on {
  border-color: var(--accent);
  background: var(--accent);
}

.pick-check.on::after {
  content: '';
  position: absolute;
  left: 4px;
  top: 1px;
  width: 4px;
  height: 8px;
  border: solid #fff;
  border-width: 0 1.5px 1.5px 0;
  transform: rotate(45deg);
}

.peer-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 52px;
  padding: 8px 0;
  text-align: left;
  border-bottom: 0.5px solid var(--line);
}

.peer-row:last-child {
  border-bottom: 0;
}

.peer-icon {
  color: var(--secondary);
  flex: none;
}

.peer-icon.selected {
  color: var(--accent);
}

.peer-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.peer-text strong {
  font-size: 13px;
  font-weight: 400;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.peer-row.selected .peer-text strong {
  font-weight: 500;
}

.peer-text small {
  font-size: 12px;
}

.peer-empty {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  border-radius: 8px;
  background: var(--control);
}

.peer-empty-icon {
  width: 32px;
  height: 32px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: var(--surface);
  color: var(--secondary);
}

.peer-empty-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.peer-empty-text strong {
  font-size: 13px;
  font-weight: 500;
}

.peer-empty-text small {
  font-size: 12px;
  white-space: normal;
}

.usage {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 20px 24px;
}

.usage-icon {
  width: 24px;
  height: 20px;
  flex: none;
  display: grid;
  place-items: center;
  color: var(--secondary);
}

.usage-icon.tone-warning {
  color: var(--warning);
}

.usage-icon.tone-success {
  color: var(--success);
}

.usage-copy {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.usage-copy h2 {
  font-size: 15px;
  font-weight: 600;
  line-height: 1.25;
}

.usage-copy p {
  font-size: 12px;
  color: var(--secondary);
  line-height: 1.45;
  min-height: 2.4em;
}

.level {
  display: flex;
  align-items: center;
  gap: 3px;
  width: 72px;
  height: 24px;
  flex: none;
}

.level > span {
  width: 3px;
  border-radius: 2px;
  background: color-mix(in srgb, var(--text) 8%, transparent);
}

.level > span.on {
  background: var(--success);
}

.speech {
  display: grid;
  gap: 12px;
}

.speech-head h2 {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 15px;
  font-weight: 600;
  line-height: 1.2;
}

.speech-head h2 svg {
  color: var(--secondary);
}

.speech-head p {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.4;
  color: var(--secondary);
}

.speech-input {
  width: 100%;
  height: 44px;
  min-height: 44px;
  resize: vertical;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 8px;
  padding: 4px 10px;
  font-size: 13px;
  line-height: 16px;
}

.speech-controls {
  display: flex;
  align-items: center;
  gap: 12px;
}

.mic-button {
  width: 40px;
  height: 40px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 50%;
  border: 0.5px solid var(--line);
  background: var(--control);
  color: var(--accent);
}

.mic-button:disabled {
  opacity: 0.7;
}

.mic-button {
  position: relative;
}

.mic-button.listening {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}

.mic-halo {
  position: absolute;
  inset: -6px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  transition: transform 0.12s ease-out;
  pointer-events: none;
}

.speech-input.listening {
  border-color: var(--accent);
}

.speech-level {
  display: inline-flex;
  align-items: flex-end;
  gap: 2px;
  height: 20px;
}

.speech-level span {
  width: 3px;
  height: 6px;
  border-radius: 2px;
  background: color-mix(in srgb, var(--secondary) 30%, transparent);
  transition: background 0.1s ease;
}

.speech-level span.on {
  background: var(--accent);
  height: 100%;
}

.speech-copy {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.speech-copy strong {
  font-size: 13px;
  font-weight: 500;
}

.speech-copy small {
  font-size: 12px;
  line-height: 1.4;
}

.maintenance {
  padding: 0;
}

.maintenance summary {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  list-style: none;
}

.maintenance summary::-webkit-details-marker {
  display: none;
}

.maintenance summary > span {
  flex: 1;
}

.maintenance summary > svg:first-child {
  color: var(--secondary);
}

.chev {
  color: var(--secondary);
  transition: transform 0.15s ease;
}

.maintenance[open] > summary .chev {
  transform: rotate(90deg);
}

.maintenance-body {
  display: grid;
  gap: 0;
  padding: 12px 0 8px;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 0;
}

.setting-row > div {
  display: grid;
  gap: 3px;
  min-width: 0;
}

.setting-row b {
  font-size: 13px;
  font-weight: 500;
}

.setting-row small {
  font-size: 12px;
  color: var(--secondary);
}

.maintenance .button {
  min-height: 28px;
  height: 28px;
  padding: 0 10px;
  font-size: 12px;
}

.maintenance .button.danger {
  color: var(--danger);
}

.privacy {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 0 4px;
  font-size: 11px;
  color: var(--secondary);
  line-height: 1.5;
}

.privacy svg {
  margin-top: 2px;
}

.loading-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 24px;
}

.loading-icon {
  width: 48px;
  height: 48px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: var(--control);
  color: var(--accent);
}

.loading-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.loading-text h2 {
  font-size: 15px;
  font-weight: 600;
}

.loading-text p {
  font-size: 12px;
  color: var(--secondary);
}

.error-card {
  display: grid;
  gap: 12px;
  padding: 24px;
}

.error-head {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.error-icon {
  width: 36px;
  height: 36px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: color-mix(in srgb, var(--warning) 6%, transparent);
  color: var(--warning);
}

.error-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.error-text h2 {
  font-size: 15px;
  font-weight: 600;
}

.error-text p {
  font-size: 12px;
  color: var(--secondary);
}

.tech summary {
  font-size: 11px;
  font-weight: 500;
  color: var(--secondary);
  cursor: pointer;
}

.tech pre {
  margin: 8px 0 0;
  padding: 10px;
  border-radius: 8px;
  background: var(--control);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--secondary);
}

.error-actions {
  display: flex;
  justify-content: flex-end;
}

.page-error {
  position: absolute;
  left: 24px;
  right: 24px;
  bottom: 24px;
  max-width: 880px;
  z-index: 45;
}

.transient {
  position: absolute;
  left: 24px;
  right: 24px;
  bottom: 24px;
  max-width: 880px;
  z-index: 45;
}

.transient :deep(.notice) {
  background: var(--surface);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
}

.sheet-scrim {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.32);
}

.sheet {
  width: min(460px, 100%);
  max-height: 85vh;
  overflow: auto;
  display: grid;
  gap: 12px;
  padding: 24px;
  border-radius: 16px;
  background: var(--surface);
  border: 0.5px solid var(--line);
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.28);
}

.sheet h2 {
  font-size: 20px;
  font-weight: 600;
}

.sheet p {
  font-size: 13px;
  line-height: 1.5;
}

.sheet p.secondary {
  font-size: 12px;
  color: var(--secondary);
}

.sheet-check {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 13px;
}

.sheet-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}

.danger-solid {
  background: var(--danger);
  border-color: transparent;
  color: #fff;
}

@media (max-width: 760px) {
  .role-grid {
    grid-template-columns: 1fr;
  }
  .stage {
    grid-template-columns: 1fr;
    justify-items: center;
  }
  .node {
    width: 100%;
  }
  .signal {
    width: 100%;
  }
}
</style>

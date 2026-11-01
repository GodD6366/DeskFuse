<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { backend, errorMessage } from '../services/backend'
import { friendlyError } from './features/core'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import ConfirmDialog from './features/ConfirmDialog.vue'
import FeatureGate from './features/FeatureGate.vue'
import MousePhotoEditor from './mouse/MousePhotoEditor.vue'
import MouseGenericEditor from './mouse/MouseGenericEditor.vue'
import MouseContributionDialog from './mouse/MouseContributionDialog.vue'
import { CHOICES, MODELS, actionFor, actionValue, isG502Button, modelOf } from './mouse/model'

const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ refresh: [] }>()

const mouse = computed<any>(() => props.snapshot?.mouse ?? null)
const devices = computed<any[]>(() => mouse.value?.devices ?? [])
const bindings = computed<any[]>(() => mouse.value?.bindings ?? [])
const availableButtons = computed<number[]>(() => (mouse.value?.available_buttons ?? []).map(Number))
const featureEnabled = computed(() => props.snapshot?.features?.mouse !== false)
const loaded = computed(() => Boolean(mouse.value?.loaded))
const tapReady = computed(() => mouse.value?.tap_ready ?? mouse.value?.tapReady)
const hidStatus = computed(() => String(mouse.value?.hid_status ?? mouse.value?.hidStatus ?? '等待原生设备状态'))

const busy = ref(false)
const error = ref('')
const errorRaw = ref('')
const selection = ref('')
const modifiers = ref(0)
const reset = ref(false)
const recording = ref<number | null>(null)
const menu = ref<{ button: number; left: number; top: number } | null>(null)
const contributionOpen = ref(false)
const wheelOverrides = ref<Record<string, boolean>>({})

const modifierChoices = [
  ['⌘', 1048576],
  ['⇧', 131072],
  ['⌥', 524288],
  ['⌃', 262144],
] as const

// 中间层未返回 options 时按原生逻辑兜底分组。
const options = computed<any[]>(() => {
  const list = mouse.value?.options
  if (Array.isArray(list) && list.length) return list
  const byModel = new Map<string, any>()
  const generic: any[] = []
  for (const device of devices.value) {
    const id = String(device.model ?? modelOf(String(device.name ?? '')) ?? '')
    if (!id) {
      generic.push(device)
      continue
    }
    const existing = byModel.get(id)
    if (existing) existing.device_ids.push(device.id)
    else {
      byModel.set(id, {
        id: `model:${id}`,
        title: MODELS.find((model) => model.id === id)?.name ?? String(device.name ?? ''),
        model: id,
        device_ids: [device.id],
      })
    }
  }
  const result = MODELS.filter((model) => byModel.has(model.id)).map((model) => byModel.get(model.id))
  if (generic.length || result.length === 0) {
    result.push({ id: 'generic', title: '通用鼠标', model: null, device_ids: generic.map((device) => device.id) })
  }
  return result
})

const selectedOption = computed<any>(() => options.value.find((option) => option.id === selection.value) ?? options.value[0])
const profile = computed<string | null>(() => selectedOption.value?.model ?? null)
const isPhotoModel = computed(() => ['g502Lightspeed', 'mxMaster3', 'mxMaster4'].includes(profile.value ?? ''))

const g502 = computed<any>(() => mouse.value?.g502 ?? {
  locations: mouse.value?.g502_locations ?? {},
  detecting: mouse.value?.detecting ?? false,
  capture_released: mouse.value?.capture_released ?? false,
  last_button: mouse.value?.last_button ?? null,
})
const g502Locations = computed<Record<string, number>>(() => g502.value?.locations ?? {})
const detecting = computed(() => Boolean(g502.value?.detecting))
const captureReleased = computed(() => Boolean(g502.value?.capture_released))
const lastButton = computed<number | null>(() => g502.value?.last_button ?? null)

// 支持列表由 Go 中间层组装；缺失时按设备名兜底，避免旧宿主空白。
const modelViews = computed<any[]>(() => {
  const list = mouse.value?.models
  if (Array.isArray(list) && list.length) return list
  return MODELS.map((model) => {
    const device = devices.value.find(
      (entry) => String(entry.model ?? modelOf(String(entry.name ?? '')) ?? '') === model.id,
    )
    return {
      ...model,
      connected: Boolean(device),
      battery_percent: device?.battery_percent ?? null,
      battery_charging: Boolean(device?.battery_charging),
      wheel_available: device?.wheel_freespin !== undefined && device?.wheel_freespin !== null,
      wheel_freespin: device?.wheel_freespin ?? null,
      wheel_name: device?.name ?? null,
    }
  })
})

const selectedDevices = computed<any[]>(() => {
  const ids: any[] = selectedOption.value?.device_ids ?? []
  return ids.map((id) => devices.value.find((device) => String(device.id) === String(id))).filter(Boolean)
})
const contributionDevice = computed<any>(() => selectedDevices.value[0] ?? null)

// 识别到、但不在支持列表里的鼠标。它们只走标准五键，需要单独标注。
const unsupportedDevices = computed<any[]>(() => devices.value.filter((device) => !String(device.model ?? '')))

const selectedModelName = computed<string | null>(() => {
  const id = selectedOption.value?.model
  if (!id) return null
  return MODELS.find((model) => model.id === id)?.name ?? String(selectedOption.value?.title ?? id)
})

const genericDeviceName = computed<string | null>(() => {
  if (selectedOption.value?.model) return null
  const device = selectedDevices.value.find((entry) => !String(entry.model ?? ''))
  return device?.name ? String(device.name) : null
})

const activeModelView = computed<any | null>(() => {
  if (!profile.value) return null
  return modelViews.value.find((model) => model.id === profile.value) ?? null
})

const activeWheelModel = computed<any | null>(() => {
  const model = activeModelView.value
  if (!model || !model.wheel_available || model.wheel_freespin === null || model.wheel_freespin === undefined) {
    return null
  }
  return model
})

const singleDeviceTitle = computed(() => {
  if (selectedOption.value) return optionLabel(selectedOption.value)
  return '标准五键'
})

const currentDeviceTone = computed<'on' | 'warn' | 'idle'>(() => {
  if (activeModelView.value?.connected) return 'on'
  if (genericDeviceName.value) return 'warn'
  return 'idle'
})

const currentStatusPill = computed<{ text: string; tone: string } | null>(() => {
  if (activeModelView.value) return connectionPill(activeModelView.value)
  if (genericDeviceName.value) return { text: '未适配', tone: 'warn' }
  return { text: '未连接', tone: 'neutral' }
})

const connectedModelCount = computed(() => modelViews.value.filter((model) => model.connected).length)
const connectedSummary = computed(() => {
  const adapted = connectedModelCount.value
  const unadapted = unsupportedDevices.value.length
  if (adapted > 0 && unadapted > 0) return `${adapted} 款已连接 · ${unadapted} 台未适配`
  if (adapted > 0) return `${adapted} 款已连接`
  if (unadapted > 0) return `${unadapted} 台未适配`
  return `支持 ${modelViews.value.length} 款型号`
})

/** 示意图只画标准五键，其余能绑的键放进扩展条。 */
const extraButtons = computed(() => buttonIDs.value.filter((button) => button > 4))

/** 选择器里避免把未适配的设备笼统叫成“通用鼠标”。 */
function optionLabel(option: any): string {
  if (option.model) return String(option.title ?? '')
  const names = (option.device_ids ?? []).map(
    (id: any) => devices.value.find((device) => String(device.id) === String(id))?.name,
  ).filter(Boolean)
  if (names.length === 1) return `${names[0]}（未适配）`
  if (names.length > 1) return `${names.length} 个未适配鼠标`
  return '标准五键'
}

function baseButtons(model: string | null): number[] {
  if (model === 'g502Lightspeed') return Array.from({ length: 11 }, (_, index) => 208 + index)
  if (model) return [0, 1, 2, 3, 4, 195, 196]
  return [0, 1, 2, 3, 4]
}

function allowedButton(button: number, model: string | null): boolean {
  if (model === 'g502Lightspeed') return isG502Button(button)
  if (model) return (button >= 0 && button <= 4) || button === 195 || button === 196
  return button >= 0 && button <= 4
}

const buttonIDs = computed(() => {
  const model = profile.value
  const set = new Set(baseButtons(model))
  for (const binding of bindings.value) set.add(Number(binding.button))
  return [...set].filter((button) => Number.isFinite(button) && allowedButton(button, model)).sort((a, b) => a - b)
})

function connectionPill(model: any): { text: string; tone: string } {
  if (!model.connected) return { text: '未连接', tone: 'neutral' }
  const percent = model.battery_percent
  if (percent === null || percent === undefined) return { text: '已连接', tone: 'success' }
  const charging = model.battery_charging === true
  const text = charging ? `充电中 ${percent}%` : `电量 ${percent}%`
  const tone = percent <= 15 ? 'danger' : percent <= 30 ? 'warn' : 'success'
  return { text, tone }
}

function wheelValue(model: any): boolean {
  const key = String(model.wheel_name ?? '')
  if (key && typeof wheelOverrides.value[key] === 'boolean') return wheelOverrides.value[key]
  return model.wheel_freespin === true
}

async function setWheel(model: any, value: boolean) {
  const key = String(model.wheel_name ?? '')
  if (!key) return
  wheelOverrides.value[key] = value
  const result = await run('mouse.setWheelMode', { name: key, freespin: value })
  if (!result.ok || result.value?.applied === false) {
    delete wheelOverrides.value[key]
    if (result.ok) error.value = '滚轮模式未能应用，请确认鼠标仍已连接。'
    return
  }
  emit('refresh')
}

async function invoke(action: string, params: Record<string, unknown> = {}) {
  try {
    return await backend.invoke(action, params)
  } catch {
    return null
  }
}

async function run(action: string, params: Record<string, unknown> = {}): Promise<{ ok: boolean; value?: any }> {
  busy.value = true
  error.value = ''
  errorRaw.value = ''
  try {
    const value = await backend.invoke(action, params)
    return { ok: true, value }
  } catch (e) {
    errorRaw.value = errorMessage(e)
    error.value = friendlyError(errorRaw.value)
    return { ok: false }
  } finally {
    busy.value = false
  }
}

function effectiveModifiersFor(button: number): number {
  return button >= 195 || isG502Button(button) ? 0 : modifiers.value
}

function currentMenuValue(button: number): string {
  const mods = effectiveModifiersFor(button)
  const found = bindings.value.find(
    (entry) => Number(entry.button) === button && Number(entry.modifiers) === mods,
  )?.action ?? null
  return actionValue(found)
}

function openMenu(button: number, event: MouseEvent) {
  if (busy.value || !loaded.value || recording.value !== null) return
  const target = event.currentTarget as HTMLElement | null
  const rect = target?.getBoundingClientRect()
  const width = 228
  const left = rect
    ? Math.max(10, Math.min(rect.left, window.innerWidth - width - 10))
    : Math.max(10, (window.innerWidth - width) / 2)
  const top = rect
    ? Math.max(10, Math.min(rect.bottom + 6, window.innerHeight - 436))
    : 120
  menu.value = { button, left, top }
}

const menuChoices = computed(() => {
  const current = menu.value
  if (!current) return []
  const physical = current.button === 195 || current.button === 196 || isG502Button(current.button)
  return CHOICES.filter(([id]) => {
    const kind = id === 'default' ? null : id === 'disabled' ? 'disabled' : id.startsWith('mouse:') ? 'mouse' : 'shortcut'
    if (physical && kind === 'mouse') return false
    if (isG502Button(current.button) && kind === 'disabled') return false
    return true
  })
})

const menuActiveValue = computed(() => (menu.value ? currentMenuValue(menu.value.button) : ''))
const menuSupportsLayerKeep = computed(() => Boolean(menu.value && effectiveModifiersFor(menu.value.button) !== 0))

async function applyAction(button: number, action: any) {
  menu.value = null
  const result = await run('mouse.setMapping', {
    button,
    modifiers: effectiveModifiersFor(button),
    action,
  })
  if (result.ok) emit('refresh')
}

async function applyChoice(button: number, id: string) {
  await applyAction(button, actionFor(id))
}

function menuApply(id: string) {
  if (menu.value) void applyChoice(menu.value.button, id)
}

function menuRecord() {
  if (menu.value) void beginRecording(menu.value.button)
}

function menuKeepOriginal() {
  const current = menu.value
  if (current) void applyAction(current.button, { kind: 'mouse', button: current.button })
}

function closeMenu() {
  menu.value = null
}

function clearModifiers() {
  void endRecording()
  modifiers.value = 0
}

function toggleModifier(bit: number) {
  void endRecording()
  modifiers.value ^= bit
}

async function beginRecording(button: number) {
  menu.value = null
  recording.value = button
  await invoke('input.set_recording', { enabled: true })
  window.addEventListener('keydown', onRecordKey, true)
}

async function onRecordKey(event: KeyboardEvent) {
  if (recording.value === null) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape' || event.keyCode === 53) {
    await endRecording()
    return
  }
  const button = recording.value
  const code = event.keyCode || event.which
  let mods = 0
  if (event.metaKey) mods |= 1048576
  if (event.shiftKey) mods |= 131072
  if (event.altKey) mods |= 524288
  if (event.ctrlKey) mods |= 262144
  recording.value = null
  window.removeEventListener('keydown', onRecordKey, true)
  await invoke('input.set_recording', { enabled: false })
  if (!code) {
    error.value = '这个按键没有可用的键码，请换一个按键。'
    errorRaw.value = ''
    return
  }
  await applyAction(button, { kind: 'shortcut', key_code: code, modifiers: mods })
}

async function endRecording() {
  if (recording.value === null) return
  recording.value = null
  window.removeEventListener('keydown', onRecordKey, true)
  await invoke('input.set_recording', { enabled: false })
}

async function beginCapture() {
  await run('mouse.beginDetection', { g502: true })
}

async function detectExtension() {
  await run('mouse.beginDetection', { g502: false })
}

async function cancelCapture() {
  await invoke('mouse.endDetection')
  await invoke('mouse.clearCapture')
  emit('refresh')
}

async function locateG502(button: number, position: string) {
  const result = await run('mouse.locateG502', { button, position })
  await cancelCapture()
  if (result.ok) emit('refresh')
}

function contribute() {
  contributionOpen.value = true
}

async function resetDefaults() {
  if ((await run('mouse.reset', { confirm: true })).ok) {
    reset.value = false
    emit('refresh')
  }
}

function onEscape(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    menu.value = null
    reset.value = false
  }
}

watch(
  () => selectedOption.value?.id,
  () => {
    modifiers.value = 0
    void endRecording()
  },
)

watch(
  () => options.value.map((option) => option.id).join('|'),
  () => {
    if (!options.value.some((option) => option.id === selection.value)) {
      selection.value = options.value[0]?.id ?? ''
    }
  },
  { immediate: true },
)

watch(
  () => mouse.value?.models,
  (list) => {
    if (!Array.isArray(list)) return
    for (const model of list) {
      const key = String(model.wheel_name ?? '')
      if (key && wheelOverrides.value[key] === model.wheel_freespin) delete wheelOverrides.value[key]
    }
  },
  { deep: true },
)

onMounted(() => {
  void invoke('mouse.scan', { enabled: true })
  window.addEventListener('keydown', onEscape)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onEscape)
  void endRecording()
  void invoke('mouse.endDetection')
  void invoke('mouse.clearCapture')
  void invoke('mouse.scan', { enabled: false })
})
</script>

<template>
  <div v-if="!snapshot" class="empty-state">
    <Icon name="mouse" :size="40" />
    <h2>鼠标服务未连接</h2>
    <p>连接原生服务后读取设备和按键配置。</p>
  </div>
  <FeatureGate v-else-if="!featureEnabled" page="mouse" title="鼠标按键" />
  <div v-else class="mouse-page">
    <div class="mouse-inner">
      <Notice
        v-if="mouse?.enabled && tapReady === false"
        tone="warning"
        title="需要辅助功能与输入监控权限"
        action-title="检查权限"
        @action="run('permissions.open', { permission: 'input_monitoring', mode: 'settings' })"
      />
      <Notice
        v-if="error || mouse?.error"
        tone="danger"
        :title="friendlyError(String(error || mouse?.error))"
        :raw="errorRaw || undefined"
      />

      <!-- 主视觉：就地标注图纸画布 -->
      <MousePhotoEditor
        v-if="isPhotoModel"
        :model="profile ?? ''"
        :bindings="bindings"
        :locations="g502Locations"
        :available-buttons="availableButtons"
        :modifiers="modifiers"
        :recording="recording"
        :active-menu-button="menu?.button ?? null"
        :detecting="detecting"
        :capture-released="captureReleased"
        :last-button="lastButton"
        :save-status="String(mouse?.save_status ?? '')"
        :last-execution="String(mouse?.last_execution ?? '')"
        :hid-status="hidStatus"
        :loaded="loaded"
        @record="beginRecording"
        @cancel-record="endRecording"
        @menu="openMenu"
        @begin-capture="beginCapture"
        @cancel-capture="cancelCapture"
        @locate="locateG502"
        @detect-extension="detectExtension"
      >
        <template #device>
          <div class="device-bar">
            <span class="device-dot" :class="`is-${currentDeviceTone}`" aria-hidden="true" />
            <select
              v-if="options.length > 1"
              v-model="selection"
              class="df-select device-select"
              aria-label="选择鼠标"
              title="只显示当前识别到的鼠标；未适配型号按标准五键映射"
            >
              <option v-for="opt in options" :key="opt.id" :value="opt.id">
                {{ optionLabel(opt) }}
              </option>
            </select>
            <strong v-else class="device-title">{{ singleDeviceTitle }}</strong>

            <span
              v-if="currentStatusPill"
              class="pill"
              :class="{
                on: currentStatusPill.tone === 'success',
                warn: currentStatusPill.tone === 'warn',
                danger: currentStatusPill.tone === 'danger',
              }"
            >{{ currentStatusPill.text }}</span>

            <div v-if="activeWheelModel" class="inline-wheel">
              <span class="inline-wheel-label">滚轮</span>
              <div class="segmented" role="group" aria-label="滚轮模式">
                <button
                  type="button"
                  :class="{ active: wheelValue(activeWheelModel) }"
                  @click="setWheel(activeWheelModel, true)"
                >自由滚动</button>
                <button
                  type="button"
                  :class="{ active: !wheelValue(activeWheelModel) }"
                  @click="setWheel(activeWheelModel, false)"
                >棘轮</button>
              </div>
            </div>
          </div>
        </template>

        <template #modifiers>
          <div class="modifier-bar" role="group" aria-label="修饰键组合层">
            <span class="modifier-label">组合键层</span>
            <button
              type="button"
              class="layer-pill"
              :class="{ active: modifiers === 0 }"
              :aria-pressed="modifiers === 0"
              @click="clearModifiers"
            >默认</button>
            <button
              v-for="[name, bit] in modifierChoices"
              :key="bit"
              type="button"
              class="keycap"
              :class="{ active: (modifiers & bit) !== 0 }"
              :aria-pressed="(modifiers & bit) !== 0"
              :title="`按住 ${name} 时使用独立映射`"
              @click="toggleModifier(bit)"
            >{{ name }}</button>
            <span v-if="modifiers !== 0" class="modifier-hint">未单独设置的按键沿用默认映射</span>
          </div>
        </template>

        <template #foot-actions>
          <span class="save-note"><Icon name="check-circle" :size="12" />更改自动保存</span>
          <button type="button" class="df-link danger" :disabled="busy" @click="reset = true">恢复默认…</button>
        </template>
      </MousePhotoEditor>

      <MouseGenericEditor
        v-else
        :bindings="bindings"
        :modifiers="modifiers"
        :recording="recording"
        :active-menu-button="menu?.button ?? null"
        :save-status="String(mouse?.save_status ?? '')"
        :last-execution="String(mouse?.last_execution ?? '')"
        :hid-status="hidStatus"
        :model-name="selectedModelName"
        :device-name="genericDeviceName"
        :extras="extraButtons"
        :loaded="loaded"
        @menu="openMenu"
        @record="beginRecording"
        @cancel-record="endRecording"
      >
        <template #device>
          <div class="device-bar">
            <span class="device-dot" :class="`is-${currentDeviceTone}`" aria-hidden="true" />
            <select
              v-if="options.length > 1"
              v-model="selection"
              class="df-select device-select"
              aria-label="选择鼠标"
              title="只显示当前识别到的鼠标；未适配型号按标准五键映射"
            >
              <option v-for="opt in options" :key="opt.id" :value="opt.id">
                {{ optionLabel(opt) }}
              </option>
            </select>
            <strong v-else class="device-title">{{ singleDeviceTitle }}</strong>

            <span
              v-if="currentStatusPill"
              class="pill"
              :class="{
                on: currentStatusPill.tone === 'success',
                warn: currentStatusPill.tone === 'warn',
                danger: currentStatusPill.tone === 'danger',
              }"
            >{{ currentStatusPill.text }}</span>

            <div v-if="activeWheelModel" class="inline-wheel">
              <span class="inline-wheel-label">滚轮</span>
              <div class="segmented" role="group" aria-label="滚轮模式">
                <button
                  type="button"
                  :class="{ active: wheelValue(activeWheelModel) }"
                  @click="setWheel(activeWheelModel, true)"
                >自由滚动</button>
                <button
                  type="button"
                  :class="{ active: !wheelValue(activeWheelModel) }"
                  @click="setWheel(activeWheelModel, false)"
                >棘轮</button>
              </div>
            </div>
          </div>
        </template>

        <template #modifiers>
          <div class="modifier-bar" role="group" aria-label="修饰键组合层">
            <span class="modifier-label">组合键层</span>
            <button
              type="button"
              class="layer-pill"
              :class="{ active: modifiers === 0 }"
              :aria-pressed="modifiers === 0"
              @click="clearModifiers"
            >默认</button>
            <button
              v-for="[name, bit] in modifierChoices"
              :key="bit"
              type="button"
              class="keycap"
              :class="{ active: (modifiers & bit) !== 0 }"
              :aria-pressed="(modifiers & bit) !== 0"
              :title="`按住 ${name} 时使用独立映射`"
              @click="toggleModifier(bit)"
            >{{ name }}</button>
          </div>
        </template>

        <template #foot-actions>
          <span class="save-note"><Icon name="check-circle" :size="12" />更改自动保存</span>
          <button type="button" class="df-link danger" :disabled="busy" @click="reset = true">恢复默认…</button>
        </template>
      </MouseGenericEditor>

      <!-- 次级双栏底座：左栏支持目录与适配提交，右栏状态、权限与跨 Mac 同步 -->
      <section class="secondary-grid">
        <div class="card supported-card">
          <div class="deck-head">
            <h2>已支持鼠标</h2>
            <span class="pill" :class="{ on: connectedModelCount > 0 }">{{ connectedSummary }}</span>
          </div>

          <div class="supported-list">
            <template v-for="(model, index) in modelViews" :key="model.id">
              <div v-if="index > 0" class="row-divider" />
              <div class="supported-row">
                <Icon name="mouse" :size="15" class="supported-icon" />
                <div class="supported-text">
                  <strong>{{ model.name }}</strong>
                  <small>{{ model.summary }}</small>
                </div>
                <span
                  class="pill"
                  :class="{
                    on: connectionPill(model).tone === 'success',
                    warn: connectionPill(model).tone === 'warn',
                    danger: connectionPill(model).tone === 'danger',
                  }"
                >{{ connectionPill(model).text }}</span>
              </div>
              <div
                v-if="model.wheel_available && model.id !== activeWheelModel?.id"
                class="wheel-row"
              >
                <Icon name="scroll" :size="13" class="wheel-icon" />
                <span class="wheel-label">滚轮模式</span>
                <template v-if="model.wheel_freespin !== null && model.wheel_freespin !== undefined">
                  <div class="segmented" role="group" aria-label="滚轮模式">
                    <button type="button" :class="{ active: wheelValue(model) }" @click="setWheel(model, true)">自由滚动</button>
                    <button type="button" :class="{ active: !wheelValue(model) }" @click="setWheel(model, false)">棘轮</button>
                  </div>
                </template>
                <span v-else class="unsupported">不支持</span>
              </div>
            </template>

            <template v-for="device in unsupportedDevices" :key="`unsupported-${device.id}`">
              <div class="row-divider" />
              <div class="supported-row">
                <Icon name="mouse" :size="15" class="supported-icon" />
                <div class="supported-text">
                  <strong>{{ device.name }}</strong>
                  <small>未适配型号 · 按标准五键工作</small>
                </div>
                <span class="pill warn">未适配</span>
              </div>
            </template>
          </div>

          <div class="contribute-bar">
            <div class="contribute-copy">
              <strong>{{ unsupportedDevices.length ? '把它加入支持列表' : '没有找到你的鼠标？' }}</strong>
              <p>
                {{ unsupportedDevices.length
                  ? '采集这台鼠标的型号与按键编号生成 Markdown，便于后续补齐实物图。'
                  : '采集型号与按键编号生成 Markdown 数据，可直接附在适配 Issue 中。' }}
              </p>
            </div>
            <button
              type="button"
              class="button"
              :class="{ primary: unsupportedDevices.length > 0 }"
              :disabled="busy"
              @click="contribute"
            >
              提供鼠标数据…
            </button>
          </div>
        </div>

        <div class="card info-card">
          <div class="deck-head">
            <h2>设备状态与同步</h2>
          </div>

          <div class="info-row">
            <div class="info-copy">
              <strong>输入监控与 HID 状态</strong>
              <p>{{ hidStatus }}</p>
            </div>
            <button
              type="button"
              class="button"
              @click="run('permissions.open', { permission: 'input_monitoring', mode: 'settings' })"
            >
              输入监控设置
            </button>
          </div>

          <div class="row-divider" />

          <div class="info-row">
            <div class="info-copy">
              <strong>Mac 间自动同步</strong>
              <p>配置自动同步到已配对且在线的 Mac；离线修改在重连后按版本整套同步，权限需在每台 Mac 单独授予。</p>
            </div>
          </div>

          <div class="row-divider" />

          <details class="note-disclosure">
            <summary>
              <span>按键边界与桌面切换说明</span>
              <Icon name="chevron-right" :size="12" class="chev" />
            </summary>
            <div class="note-body">
              <p>未适配型号按标准五键映射；若鼠标已连接但未列出，请确认系统「输入监控」权限已开启。</p>
              <p>切换桌面需系统已存在对应方向的桌面，且系统键盘快捷键保持启用。MX 扩展键与 G502 原生实体键仅按单层生效，G502 机械滚轮解锁键不产生系统事件。</p>
            </div>
          </details>
        </div>
      </section>
    </div>

    <template v-if="menu">
      <div class="menu-backdrop" @click="closeMenu" @contextmenu.prevent="closeMenu" />
      <div
        class="action-menu"
        role="menu"
        :style="{ left: `${menu.left}px`, top: `${menu.top}px` }"
      >
        <button
          v-for="[id, name] in menuChoices"
          :key="id"
          type="button"
          role="menuitemradio"
          :aria-checked="menuActiveValue === id"
          class="menu-item"
          :class="{ 'is-current': menuActiveValue === id }"
          @click="menuApply(id)"
        >
          <span>{{ name }}</span>
          <span v-if="menuActiveValue === id" class="menu-check" aria-hidden="true">✓</span>
        </button>
        <div class="menu-divider" />
        <button type="button" role="menuitem" class="menu-item" @click="menuRecord">
          <span>录制快捷键…</span>
        </button>
        <button
          v-if="menuSupportsLayerKeep"
          type="button"
          role="menuitem"
          class="menu-item"
          @click="menuKeepOriginal"
        >
          <span>此组合保持原始按钮</span>
        </button>
      </div>
    </template>

    <ConfirmDialog
      :open="reset"
      title="恢复默认并同步到其他 Mac？"
      message="现有按键映射将被清除，功能开关保持不变。"
      confirm-title="恢复默认"
      destructive
      @cancel="reset = false"
      @confirm="resetDefaults"
    />

    <MouseContributionDialog
      :open="contributionOpen"
      :devices="devices"
      :initial-device="contributionDevice"
      @close="contributionOpen = false"
    />
  </div>
</template>

<style scoped>
.mouse-page {
  overflow: auto;
  height: 100%;
  overscroll-behavior: contain;
  padding: 8px 24px 28px;
}

.mouse-inner {
  container-type: inline-size;
  max-width: 1120px;
  margin: 0 auto;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.device-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex-wrap: wrap;
}

.device-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--secondary);
  opacity: 0.45;
  flex: none;
}

.device-dot.is-on {
  background: var(--success, #28a745);
  opacity: 1;
}

.device-dot.is-warn {
  background: var(--warning, #ed8b23);
  opacity: 1;
}

.device-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.device-select {
  min-width: 0;
  max-width: min(300px, 100%);
  font-weight: 500;
}

.inline-wheel {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-left: 4px;
}

.inline-wheel-label {
  font-size: 11.5px;
  color: var(--secondary);
}

.modifier-bar {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.modifier-label {
  font-size: 11.5px;
  color: var(--secondary);
  margin-right: 2px;
}

.layer-pill {
  height: 22px;
  padding: 0 8px;
  border-radius: 5px;
  border: 1px solid var(--line);
  background: var(--surface);
  font-size: 11px;
  font-weight: 500;
  color: var(--secondary);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), border-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.layer-pill.active,
.keycap.active {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}

.modifier-hint {
  font-size: 11px;
  color: var(--secondary);
  margin-left: 2px;
}

.save-note {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--secondary);
}

/* 次级双栏底座 */
.secondary-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.14fr) minmax(0, 0.86fr);
  gap: 16px;
  align-items: start;
}

.supported-card,
.info-card {
  padding: 15px 18px;
  display: flex;
  flex-direction: column;
  gap: 0;
}

.deck-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding-bottom: 10px;
  border-bottom: 0.5px solid var(--line);
}

.deck-head h2 {
  font-size: 13.5px;
  font-weight: 600;
  margin: 0;
}

.supported-list {
  display: flex;
  flex-direction: column;
}

.supported-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 36px;
  padding: 8px 0;
}

.row-divider {
  height: 0;
  border-top: 0.5px solid var(--line);
  margin: 0;
}

.supported-icon {
  width: 18px;
  color: var(--secondary);
  flex: none;
}

.supported-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 2px;
}

.supported-text strong {
  font-size: 12.5px;
  font-weight: 500;
}

.supported-text small {
  font-size: 11px;
  line-height: 1.35;
  color: var(--secondary);
}

.pill {
  flex: none;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--control);
  color: var(--secondary);
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.pill.on {
  color: var(--success);
  background: color-mix(in srgb, var(--success) 12%, transparent);
}

.pill.warn {
  color: var(--warning, #b26a00);
  background: color-mix(in srgb, var(--warning, #b26a00) 14%, transparent);
}

.pill.danger {
  color: var(--danger, #d93025);
  background: color-mix(in srgb, var(--danger, #d93025) 12%, transparent);
}

.wheel-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 0 9px;
  font-size: 11px;
  color: var(--secondary);
}

.wheel-icon {
  width: 18px;
  flex: none;
}

.wheel-label {
  flex: 1;
}

.segmented {
  display: inline-flex;
  padding: 2px;
  border-radius: 7px;
  background: var(--control);
}

.segmented button {
  padding: 3px 9px;
  border-radius: 5px;
  font-size: 11px;
  font-weight: 500;
  color: var(--secondary);
  transition: background-color var(--dur-fast, 120ms) var(--ease-out, ease), color var(--dur-fast, 120ms) var(--ease-out, ease);
}

.segmented button.active {
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
}

.unsupported {
  color: var(--secondary);
}

.contribute-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 4px;
  padding-top: 12px;
  border-top: 0.5px solid var(--line);
  flex-wrap: wrap;
}

.contribute-copy {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 3px;
}

.contribute-copy strong {
  font-size: 12.5px;
  font-weight: 600;
}

.contribute-copy p {
  font-size: 11px;
  line-height: 1.45;
  color: var(--secondary);
}

.info-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 0;
  flex-wrap: wrap;
}

.info-copy {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 3px;
}

.info-copy strong {
  font-size: 12.5px;
  font-weight: 600;
}

.info-copy p {
  font-size: 11.5px;
  line-height: 1.5;
  color: var(--secondary);
}

.note-disclosure summary {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 40px;
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  list-style: none;
}

.note-disclosure summary::-webkit-details-marker {
  display: none;
}

.note-disclosure summary > span {
  flex: 1;
}

.chev {
  color: var(--secondary);
  transition: transform var(--dur-fast, 120ms) var(--ease-out, ease);
}

details[open] > summary .chev {
  transform: rotate(90deg);
}

.note-body {
  display: grid;
  gap: 8px;
  padding: 0 0 6px;
  font-size: 11.5px;
  color: var(--secondary);
  line-height: 1.55;
}

.menu-backdrop {
  position: fixed;
  inset: 0;
  z-index: 70;
}

.action-menu {
  position: fixed;
  z-index: 71;
  width: 228px;
  max-height: min(432px, calc(100vh - 20px));
  overflow: auto;
  overscroll-behavior: contain;
  padding: 5px;
  background: var(--surface);
  border-radius: 10px;
  box-shadow: 0 0 0 1px var(--line), 0 12px 36px rgba(0, 0, 0, 0.2);
}

.menu-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 5px 10px;
  min-height: 28px;
  border-radius: 6px;
  text-align: left;
  font-size: 12.5px;
}

.menu-item.is-current {
  font-weight: 600;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}

@media (hover: hover) and (pointer: fine) {
  .menu-item:hover {
    background: var(--accent);
    color: #fff;
  }
}

.menu-check {
  font-size: 11px;
  font-weight: 600;
}

.menu-divider {
  height: 0;
  border-top: 0.5px solid var(--line);
  margin: 4px 6px;
}

@container (max-width: 660px) {
  .secondary-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 600px) {
  .mouse-page {
    padding: 8px 14px 24px;
  }
  .inline-wheel {
    margin-left: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .layer-pill,
  .segmented button,
  .chev {
    transition: none;
  }
}
</style>

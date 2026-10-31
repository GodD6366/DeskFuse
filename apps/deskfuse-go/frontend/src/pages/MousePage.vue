<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { backend, errorMessage } from '../services/backend'
import { friendlyError } from './features/core'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import ConfirmDialog from './features/ConfirmDialog.vue'
import FeatureGate from './features/FeatureGate.vue'
import MousePhotoEditor from './mouse/MousePhotoEditor.vue'
import MouseContributionDialog from './mouse/MouseContributionDialog.vue'
import {
  CHOICES, MODELS, actionFor, actionLabel, buttonName, isG502Button, modelOf,
} from './mouse/model'

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

function baseButtons(model: string | null): number[] {
  // G502 编辑器的 11 个实体位置对应 208…218；已绑定的其他实体键仍会列出。
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

function binding(button: number): any {
  return (
    bindings.value.find(
      (item) => Number(item.button) === button && Number(item.modifiers) === modifiers.value,
    )?.action ?? null
  )
}
function label(button: number): string {
  return actionLabel(binding(button), modifiers.value)
}
const configured = (button: number) => binding(button) !== null

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
    /* 采集/释放等后台动作失败不打断页面；需要提示的动作由 run 处理。 */
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

function openMenu(button: number, event: MouseEvent) {
  if (busy.value || !loaded.value || recording.value !== null) return
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const width = 220
  menu.value = {
    button,
    left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
    top: Math.max(8, Math.min(rect.bottom + 4, window.innerHeight - 440)),
  }
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

async function applyAction(button: number, action: any) {
  menu.value = null
  const result = await run('mouse.setMapping', { button, modifiers: modifiers.value, action })
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
  <FeatureGate v-else-if="!featureEnabled" page="mouse" title="鼠标自定义" />
  <div v-else class="mouse-page">
    <section class="assignments">
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

      <select
        v-if="options.length > 1"
        v-model="selection"
        class="df-select device-picker"
        aria-label="鼠标类型"
        title="只显示当前识别到的鼠标；未适配型号使用通用映射"
      >
        <option v-for="option in options" :key="option.id" :value="option.id">
          {{ option.title }}
        </option>
      </select>

      <MousePhotoEditor
        v-if="isPhotoModel"
        :model="profile ?? ''"
        :bindings="bindings"
        :locations="g502Locations"
        :available-buttons="availableButtons"
        :modifiers="modifiers"
        :recording="recording"
        :detecting="detecting"
        :capture-released="captureReleased"
        :last-button="lastButton"
        :save-status="String(mouse?.save_status ?? '')"
        :last-execution="String(mouse?.last_execution ?? '')"
        :hid-status="hidStatus"
        @apply="applyAction"
        @record="beginRecording"
        @menu="openMenu"
        @begin-capture="beginCapture"
        @cancel-capture="cancelCapture"
        @locate="locateG502"
        @detect-extension="detectExtension"
      />

      <template v-else>
        <div class="mappings card">
          <template v-for="(id, index) in buttonIDs" :key="id">
            <div v-if="index > 0" class="mapping-divider" />
            <div class="mapping">
              <Icon
                name="mouse"
                :size="16"
                class="mapping-icon"
                :class="{ configured: configured(id) }"
              />
              <span class="mapping-name">{{ buttonName(id) }}</span>
              <button
                type="button"
                class="mapping-choice"
                :class="{ configured: configured(id) }"
                :disabled="busy || !loaded || (recording !== null && recording !== id)"
                :aria-label="`${buttonName(id)}的动作`"
                @click="openMenu(id, $event)"
              >
                <span>{{ recording === id ? '请按快捷键…' : label(id) }}</span>
                <Icon name="chevron-updown" :size="11" />
              </button>
            </div>
          </template>
        </div>

        <small class="map-hint"
          >支持左键、右键、中键及前进、后退键。侧键需鼠标具备对应按键并发送标准鼠标事件。</small
        >

        <details class="combo">
          <summary>
            <span>组合键设置</span>
            <Icon name="chevron-right" :size="12" class="chev" />
          </summary>
          <div class="combo-body">
            <div class="modifier-row">
              <span>按住这些键时使用独立映射</span>
              <button
                v-for="[name, bit] in modifierChoices"
                :key="bit"
                type="button"
                class="keycap"
                :class="{ active: (modifiers & bit) !== 0 }"
                :aria-pressed="(modifiers & bit) !== 0"
                @click="toggleModifier(bit)"
              >
                {{ name }}
              </button>
            </div>
            <small>未设置的组合沿用默认映射。</small>
          </div>
        </details>
      </template>
    </section>

    <aside>
      <section class="side-section">
        <div class="section-label"><Icon name="check-circle" :size="13" />已支持鼠标</div>
        <div class="card supported">
          <template v-for="(model, index) in modelViews" :key="model.id">
            <div v-if="index > 0" class="row-divider" />
            <div class="supported-row">
              <Icon name="mouse" :size="16" class="supported-icon" />
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
            <div v-if="model.wheel_available" class="wheel-row">
              <Icon name="scroll" :size="14" class="wheel-icon" />
              <span class="wheel-label">滚轮模式</span>
              <template v-if="model.wheel_freespin !== null && model.wheel_freespin !== undefined">
                <div class="segmented wheel-seg" role="group" aria-label="滚轮模式">
                  <button type="button" :class="{ active: wheelValue(model) }" @click="setWheel(model, true)">自由滚动</button>
                  <button type="button" :class="{ active: !wheelValue(model) }" @click="setWheel(model, false)">棘轮</button>
                </div>
              </template>
              <span v-else class="unsupported">不支持</span>
            </div>
          </template>
          <div class="contribute">
            <h3>没有找到你的鼠标？</h3>
            <p>记录型号和按键编号，生成 Markdown 数据；后续可直接用于适配 Issue。</p>
            <button class="button primary" :disabled="busy" @click="contribute">
              我要提供鼠标数据…
            </button>
          </div>
        </div>
      </section>

      <section class="card notes">
        <details class="note-disclosure">
          <summary>
            <span>设备状态与使用说明</span>
            <Icon name="chevron-right" :size="12" class="chev" />
          </summary>
          <div class="note-body">
            <p>{{ hidStatus }}</p>
            <p>列表只展示当前识别到的鼠标；未适配型号自动使用通用映射。如已连接但未显示，请检查输入监控权限。</p>
            <p>G502 的原有 DPI、配置切换或 G HUB 动作会继续执行。实体键编号按实际识别为准；不要同时绑定同一按键的通用事件和实体事件。机械滚轮解锁键无法识别。</p>
            <p>切换桌面需要对应方向存在桌面，且系统键盘快捷键已启用。MX 与 G502 原生按键暂不支持修饰键分层。</p>
            <button
              type="button"
              class="button"
              @click="run('permissions.open', { permission: 'input_monitoring', mode: 'settings' })"
            >
              打开输入监控设置
            </button>
          </div>
        </details>
        <div class="row-divider" />
        <details class="note-disclosure">
          <summary>
            <span>Mac 间自动同步</span>
            <Icon name="chevron-right" :size="12" class="chev" />
          </summary>
          <div class="note-body">
            <p>配置自动同步到已配对、已连接且支持此功能的 Mac。离线修改在重连后按逻辑版本同步整套配置，不逐个合并按键；权限需在每台 Mac 单独授予。Windows 不参与。</p>
          </div>
        </details>
      </section>

      <div class="footer">
        <span class="save-note"><Icon name="check-circle" :size="12" />更改自动保存</span>
        <button type="button" class="df-link danger" @click="reset = true">恢复默认…</button>
      </div>
    </aside>

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
          role="menuitem"
          class="menu-item"
          @click="menuApply(id)"
        >
          {{ name }}
        </button>
        <div class="menu-divider" />
        <button type="button" role="menuitem" class="menu-item" @click="menuRecord">
          录制快捷键…
        </button>
        <button
          v-if="modifiers !== 0"
          type="button"
          role="menuitem"
          class="menu-item"
          @click="menuKeepOriginal"
        >
          此组合保持原始按钮
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
  display: grid;
  grid-template-columns: minmax(0, 1fr) 340px;
  gap: 24px;
  align-items: start;
  padding: 12px 24px 24px;
  overflow: auto;
  height: 100%;
}

.assignments {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.device-picker {
  align-self: flex-end;
  width: 200px;
  max-width: 240px;
}

.mappings {
  padding: 16px;
}

.mapping {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 42px;
}

.mapping-divider {
  height: 0;
  border-top: 0.5px solid var(--line);
  margin: 8px 0;
}

.mapping-icon {
  width: 24px;
  color: var(--secondary);
  flex: none;
}

.mapping-icon.configured {
  color: var(--accent);
}

.mapping-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  line-height: 1.3;
}

.mapping-choice {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  min-width: 150px;
  padding: 8px 0;
  color: var(--secondary);
  font-size: 13px;
  text-align: right;
}

.mapping-choice.configured {
  color: var(--accent);
}

.mapping-choice > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.map-hint {
  font-size: 11px;
  color: var(--secondary);
  line-height: 1.5;
}

.combo {
  padding: 0;
  background: transparent;
  border: 0;
  border-radius: 0;
}

.combo summary,
.note-disclosure summary {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 44px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  list-style: none;
}

.combo summary::-webkit-details-marker,
.note-disclosure summary::-webkit-details-marker {
  display: none;
}

.combo summary > span,
.note-disclosure summary > span {
  flex: 1;
}

.chev {
  color: var(--secondary);
  transition: transform 0.15s ease;
}

details[open] > summary .chev {
  transform: rotate(90deg);
}

.combo-body {
  display: grid;
  gap: 8px;
  padding: 0 0 14px;
}

.modifier-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.modifier-row > span {
  flex: 1;
  color: var(--secondary);
}

.keycap.active {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}

.side-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.section-label {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-left: 4px;
  font-size: 13px;
  font-weight: 600;
  color: var(--secondary);
}

aside {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.supported {
  padding: 0 16px;
}

.supported-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 38px;
  padding: 10px 0;
}

.supported-row:first-child {
  padding-top: 14px;
}

.row-divider {
  height: 0;
  border-top: 0.5px solid var(--line);
  margin: 0;
}

.supported-icon {
  width: 20px;
  color: var(--secondary);
  flex: none;
}

.supported-text {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 3px;
}

.supported-text strong {
  font-size: 13px;
  font-weight: 500;
}

.supported-text small {
  font-size: 11px;
  line-height: 1.35;
}

.pill {
  flex: none;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--control);
  color: var(--secondary);
  font-size: 11px;
  font-weight: 600;
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
  padding: 0 0 10px;
  font-size: 11px;
  color: var(--secondary);
}

.wheel-icon {
  width: 20px;
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
  padding: 4px 9px;
  border-radius: 5px;
  font-size: 11px;
  color: var(--secondary);
}

.segmented button.active {
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
}

.unsupported {
  color: var(--secondary);
}

.contribute {
  border-top: 0.5px solid var(--line);
  margin-top: 4px;
  padding: 16px 0;
  display: grid;
  gap: 8px;
  justify-items: start;
}

.contribute h3 {
  font-size: 13px;
  font-weight: 500;
}

.contribute p {
  font-size: 11px;
  color: var(--secondary);
  line-height: 1.5;
}

.notes {
  padding: 0 16px;
}

.note-disclosure + .row-divider {
  margin: 0;
}

.note-body {
  display: grid;
  gap: 10px;
  padding: 0 0 16px;
  font-size: 11px;
  color: var(--secondary);
  line-height: 1.5;
}

.note-body .button {
  justify-self: start;
  font-size: 12px;
}

.footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 4px;
}

.save-note {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--secondary);
}

.menu-backdrop {
  position: fixed;
  inset: 0;
  z-index: 70;
}

.action-menu {
  position: fixed;
  z-index: 71;
  width: 220px;
  max-height: min(440px, calc(100vh - 16px));
  overflow: auto;
  padding: 4px;
  background: var(--surface);
  border: 0.5px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 10px 32px rgba(0, 0, 0, 0.22);
}

.menu-item {
  display: block;
  width: 100%;
  padding: 4px 10px;
  min-height: 26px;
  border-radius: 5px;
  text-align: left;
  font-size: 13px;
}

.menu-item:hover {
  background: var(--accent);
  color: #fff;
}

.menu-divider {
  height: 0;
  border-top: 0.5px solid var(--line);
  margin: 4px 6px;
}

@media (max-width: 1100px) {
  .mouse-page {
    grid-template-columns: 1fr;
  }
  aside {
    display: grid;
    grid-template-columns: 1fr 1fr;
    align-items: start;
  }
  .footer {
    grid-column: 1 / -1;
  }
}
</style>

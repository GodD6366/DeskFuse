<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { backend, errorMessage } from '../../services/backend'
import { friendlyError } from '../features/core'
import { buttonName } from './model'

const props = defineProps<{
  open: boolean
  devices: any[]
  initialDevice?: any | null
}>()
const emit = defineEmits<{ close: [] }>()

type Connection = 'wired' | 'receiver' | 'bluetooth'
const connectionOptions: Array<[Connection, string]> = [
  ['wired', '有线'], ['receiver', 'USB 接收器'], ['bluetooth', '蓝牙'],
]

const state = ref<any>({ capturing: false, samples: [], last_number: null, error: null, app_version: '', app_build: '', system_version: '' })
const modelName = ref('')
const connection = ref<Connection>('wired')
const vendorID = ref('')
const productID = ref('')
const notes = ref('')
const detectedName = ref('')
const isSupported = ref(false)
const selectedDeviceID = ref('manual')
const capturedAt = ref('')
const markdown = ref('')
const filename = ref('')
const status = ref('')
const generating = ref(false)
const probeError = ref('')
let markdownTimer: number | undefined
let pollTimer: number | undefined
let generation = 0

const capturing = computed(() => state.value?.capturing === true)
const samples = computed<any[]>(() => Array.isArray(state.value?.samples) ? state.value.samples : [])
const lastNumber = computed(() => state.value?.last_number ?? null)
const canExport = computed(() => modelName.value.trim().length > 0)

function transportConnection(transport: string): Connection {
  const value = String(transport ?? '').toLowerCase()
  if (value.includes('bluetooth')) return 'bluetooth'
  if (value.includes('usb')) return 'receiver'
  return 'wired'
}
function hexID(value: any): string {
  const number = Number(value)
  return Number.isFinite(number) ? number.toString(16).toUpperCase().padStart(4, '0') : ''
}
function applyDevice(device: any) {
  if (!device) return
  modelName.value = String(device.name ?? '')
  connection.value = transportConnection(String(device.transport ?? ''))
  vendorID.value = hexID(device.vendor_id)
  productID.value = hexID(device.product_id)
  detectedName.value = String(device.name ?? '')
  isSupported.value = device.supported === true
  status.value = ''
}
function selectDevice(id: string) {
  selectedDeviceID.value = id
  if (id === 'manual') return
  applyDevice(props.devices.find((device) => String(device.id) === id))
}

function reset() {
  status.value = ''
  probeError.value = ''
  markdown.value = ''
  filename.value = ''
  capturedAt.value = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z')
  selectedDeviceID.value = props.initialDevice?.id ?? props.devices[0]?.id ?? 'manual'
  if (selectedDeviceID.value !== 'manual') applyDevice(props.devices.find((device) => String(device.id) === selectedDeviceID.value))
  else {
    const device = props.initialDevice ?? props.devices[0]
    if (device) applyDevice(device)
    else {
      modelName.value = ''
      connection.value = 'wired'
      vendorID.value = ''
      productID.value = ''
      detectedName.value = ''
      isSupported.value = false
    }
  }
  notes.value = ''
  void refreshState()
  scheduleMarkdown()
}

function setState(value: any) {
  if (value && typeof value === 'object') state.value = value
  probeError.value = String(value?.error ?? '')
}
async function refreshState() {
  try {
    setState(await backend.invoke('mouse.contribute.state'))
  } catch (error) {
    probeError.value = friendlyError(errorMessage(error))
  }
}
async function startProbe() {
  try {
    setState(await backend.invoke('mouse.contribute.start'))
  } catch (error) {
    probeError.value = friendlyError(errorMessage(error))
  }
}
async function stopProbe() {
  try {
    setState(await backend.invoke('mouse.contribute.stop'))
  } catch {
    /* 关闭对话框时停止失败不打断流程；下次开始会重新获取状态。 */
  }
}
async function clearSamples() {
  try {
    setState(await backend.invoke('mouse.contribute.clear'))
  } catch (error) {
    probeError.value = friendlyError(errorMessage(error))
  }
}
async function removeSample(number: number) {
  try {
    setState(await backend.invoke('mouse.contribute.remove', { number }))
  } catch (error) {
    probeError.value = friendlyError(errorMessage(error))
  }
}
async function renameSample(sample: any) {
  try {
    setState(await backend.invoke('mouse.contribute.rename', { number: sample.number, name: sample.name }))
  } catch (error) {
    probeError.value = friendlyError(errorMessage(error))
  }
}

function scheduleMarkdown() {
  window.clearTimeout(markdownTimer)
  markdownTimer = window.setTimeout(() => { void refreshMarkdown() }, 160)
}
async function refreshMarkdown() {
  const current = ++generation
  generating.value = true
  try {
    const result = await backend.mouseContribution({
      model_name: modelName.value,
      connection: connection.value,
      vendor_id: vendorID.value,
      product_id: productID.value,
      detected_name: detectedName.value,
      is_supported: isSupported.value,
      notes: notes.value,
      app_version: state.value?.app_version ?? '',
      app_build: state.value?.app_build ?? '',
      system_version: state.value?.system_version ?? '',
      captured_at: capturedAt.value,
      samples: samples.value.map((sample) => ({
        number: Number(sample.number), name: String(sample.name ?? ''), count: Number(sample.count ?? 0),
      })),
    })
    if (current !== generation) return
    markdown.value = String(result?.markdown ?? '')
    filename.value = String(result?.filename ?? '')
  } catch (error) {
    if (current === generation) probeError.value = friendlyError(errorMessage(error))
  } finally {
    if (current === generation) generating.value = false
  }
}

async function copyMarkdown() {
  if (!canExport.value || !markdown.value) return
  status.value = ''
  try {
    await backend.invoke('mouse.contribute.copy', { markdown: markdown.value })
    status.value = '已复制 Markdown'
  } catch (error) {
    status.value = friendlyError(errorMessage(error))
  }
}
async function saveMarkdown() {
  if (!canExport.value || !markdown.value) return
  status.value = ''
  try {
    const result = await backend.invoke('mouse.contribute.save', { markdown: markdown.value, filename: filename.value })
    status.value = result?.saved ? `已保存 ${String(result.path ?? '').split('/').pop()}` : '已取消保存'
  } catch (error) {
    status.value = friendlyError(errorMessage(error))
  }
}

function close() {
  if (capturing.value) void stopProbe()
  emit('close')
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !props.open) return
  event.preventDefault()
  if (capturing.value) void stopProbe()
  else close()
}

watch(
  () => props.open,
  (open) => {
    window.clearInterval(pollTimer)
    if (open) {
      reset()
      window.addEventListener('keydown', onKeydown)
      pollTimer = window.setInterval(() => {
        if (props.open && capturing.value) void refreshState()
      }, 500)
    } else {
      window.removeEventListener('keydown', onKeydown)
      void stopProbe()
    }
  },
)
watch(
  () => [modelName.value, connection.value, vendorID.value, productID.value, detectedName.value,
    isSupported.value, notes.value, capturedAt.value,
    state.value?.app_version, state.value?.app_build, state.value?.system_version,
    samples.value.map((sample) => `${sample.number}:${sample.name}:${sample.count}`).join('|')],
  scheduleMarkdown,
)
onBeforeUnmount(() => {
  window.clearTimeout(markdownTimer)
  window.clearInterval(pollTimer)
  window.removeEventListener('keydown', onKeydown)
  if (capturing.value) void stopProbe()
})
</script>

<template>
  <div v-if="open" class="dialog-backdrop" @click.self="close">
    <section class="mouse-dialog" role="dialog" aria-modal="true" aria-labelledby="mouse-contribution-title">
      <header class="dialog-head">
        <div>
          <h2 id="mouse-contribution-title">提供鼠标数据</h2>
          <p>记录型号和 macOS 按键编号，生成可粘贴到适配 Issue 的 Markdown。</p>
        </div>
        <button type="button" class="icon-button" aria-label="关闭" @click="close">✕</button>
      </header>

      <div class="dialog-body">
        <section class="block">
          <h3>鼠标信息</h3>
          <div class="field">
            <span class="label">检测到的设备</span>
            <select :value="selectedDeviceID" class="df-select grow" :disabled="capturing" @change="selectDevice(($event.target as HTMLSelectElement).value)">
              <option value="manual">手动填写</option>
              <option v-for="device in devices" :key="device.id" :value="String(device.id)">
                {{ device.name }}{{ device.supported ? '（已适配）' : '' }}
              </option>
            </select>
          </div>
          <div class="field">
            <span class="label">鼠标型号</span>
            <input v-model="modelName" class="grow" type="text" placeholder="例如 MX Master 4" :disabled="capturing" />
          </div>
          <div class="field">
            <span class="label">连接方式</span>
            <div class="segmented" role="group" aria-label="连接方式">
              <button
                v-for="[value, label] in connectionOptions" :key="value" type="button"
                :class="{ active: connection === value }" :disabled="capturing"
                @click="connection = value"
              >{{ label }}</button>
            </div>
          </div>
          <div class="field">
            <span class="label">Vendor ID</span>
            <input v-model="vendorID" class="id-input" type="text" placeholder="046D" :disabled="capturing" />
            <span class="label inline">Product ID</span>
            <input v-model="productID" class="id-input" type="text" placeholder="C548" :disabled="capturing" />
          </div>
          <div class="field column">
            <span class="label">补充说明</span>
            <textarea v-model="notes" rows="3" :disabled="capturing" placeholder="可留空" />
          </div>
        </section>

        <section class="block">
          <h3>按键识别</h3>
          <div class="capture-row">
            <div>
              <strong>{{ capturing ? '正在识别按键' : '依次记录鼠标按键' }}</strong>
              <p>{{ capturing ? '按下每个按键，DeskFuse 会记录编号；按 Esc 结束。' : '开始后只记录标准鼠标事件；未被 macOS 识别的按键可能不会出现。' }}</p>
            </div>
            <button type="button" class="button" :class="{ primary: !capturing }" :disabled="capturing" @click="startProbe">
              {{ capturing ? '正在识别…' : '开始识别按键' }}
            </button>
          </div>
          <p v-if="probeError" class="danger">{{ probeError }}</p>
          <p v-if="lastNumber !== null" class="accent">最近按下：编号 {{ lastNumber }} · {{ buttonName(Number(lastNumber)) }}</p>
          <p v-if="!samples.length" class="muted">尚未记录按键</p>
          <div v-else class="samples">
            <div v-for="sample in samples" :key="sample.number" class="sample">
              <span class="number">编号 {{ sample.number }}</span>
              <input v-model="sample.name" type="text" :disabled="capturing" @change="renameSample(sample)" />
              <span class="count">{{ sample.count }} 次</span>
              <button type="button" class="icon-button" :disabled="capturing" aria-label="删除" @click="removeSample(sample.number)">⌫</button>
            </div>
            <button type="button" class="link-button" :disabled="capturing" @click="clearSamples">清空记录</button>
          </div>
        </section>

        <section class="block">
          <h3>Markdown 预览</h3>
          <pre class="preview" aria-live="polite">{{ markdown || (generating ? '正在生成…' : '填写型号后生成') }}</pre>
        </section>
      </div>

      <footer class="dialog-foot">
        <span class="muted">{{ status || (!canExport ? '填写型号后可导出；建议至少记录一个按键。' : '') }}</span>
        <div class="actions">
          <button type="button" class="button" @click="close">关闭</button>
          <button type="button" class="button" :disabled="!canExport || generating" @click="copyMarkdown">复制 Markdown</button>
          <button type="button" class="button primary" :disabled="!canExport || generating" @click="saveMarkdown">保存 .md…</button>
        </div>
      </footer>
    </section>
  </div>
</template>

<style scoped>
.dialog-backdrop {
  position: fixed; inset: 0; z-index: 90; display: flex; align-items: center; justify-content: center;
  padding: 24px; background: rgba(0, 0, 0, 0.28); backdrop-filter: blur(2px);
}
.mouse-dialog {
  width: min(680px, 100%); max-height: min(720px, calc(100vh - 48px)); display: flex; flex-direction: column;
  background: var(--surface); border: 0.5px solid var(--line); border-radius: 16px; overflow: hidden;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.3);
}
.dialog-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; padding: 20px 20px 14px; }
.dialog-head h2 { font-size: 18px; font-weight: 700; }
.dialog-head p { margin-top: 4px; font-size: 12px; color: var(--secondary); }
.icon-button { width: 24px; height: 24px; border-radius: 6px; color: var(--secondary); }
.icon-button:hover { background: var(--control); }
.dialog-body { flex: 1; overflow: auto; padding: 0 20px 16px; display: flex; flex-direction: column; gap: 16px; }
.block { display: flex; flex-direction: column; gap: 9px; padding-top: 14px; border-top: 0.5px solid var(--line); }
.block h3 { font-size: 12px; font-weight: 600; color: var(--secondary); }
.field { display: flex; align-items: center; gap: 10px; font-size: 12px; }
.field.column { align-items: stretch; flex-direction: column; gap: 6px; }
.label { width: 92px; flex: none; color: var(--secondary); }
.label.inline { width: auto; margin-left: 8px; }
.grow { flex: 1; min-width: 0; }
.field input:not(.id-input), .field select, .field textarea, .sample input {
  min-height: 28px; padding: 5px 8px; border-radius: 7px; font-size: 12px;
  background: var(--control); border: 0.5px solid var(--line); color: var(--text);
}
.field textarea { resize: vertical; }
.id-input { width: 96px; min-height: 28px; padding: 5px 8px; border-radius: 7px; font-size: 12px; background: var(--control); border: 0.5px solid var(--line); color: var(--text); }
.segmented { display: inline-flex; padding: 2px; border-radius: 7px; background: var(--control); }
.segmented button { padding: 4px 12px; border-radius: 5px; font-size: 11px; color: var(--secondary); }
.segmented button.active { background: var(--surface); color: var(--text); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12); }
.segmented button:disabled { opacity: 0.6; }
.capture-row { display: flex; align-items: center; justify-content: space-between; gap: 14px; }
.capture-row strong { display: block; font-size: 12px; font-weight: 600; }
.capture-row p { margin-top: 3px; font-size: 11px; color: var(--secondary); line-height: 1.45; }
.samples { display: flex; flex-direction: column; gap: 6px; }
.sample { display: flex; align-items: center; gap: 8px; }
.sample .number { width: 66px; flex: none; font-size: 11px; font-weight: 600; font-variant-numeric: tabular-nums; }
.sample input { flex: 1; min-width: 0; }
.sample .count { width: 40px; flex: none; text-align: right; font-size: 11px; color: var(--secondary); }
.preview {
  min-height: 160px; max-height: 220px; overflow: auto; padding: 12px; border-radius: 8px;
  font-family: var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace); font-size: 11px; line-height: 1.55;
  white-space: pre-wrap; word-break: break-word; background: var(--control); border: 0.5px solid var(--line);
}
.dialog-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 20px; border-top: 0.5px solid var(--line); }
.actions { display: flex; align-items: center; gap: 8px; }
.muted { font-size: 11px; color: var(--secondary); }
.danger { font-size: 11px; color: var(--danger, #d93025); }
.accent { font-size: 11px; color: var(--accent); }
.link-button { align-self: flex-start; font-size: 11px; color: var(--accent); }
.link-button:hover { text-decoration: underline; }
</style>

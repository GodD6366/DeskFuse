<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import { Keyboard } from 'lucide-vue-next'
import { backend } from '../../services/backend'
import Keycaps from './Keycaps.vue'
import {
  CODE_TO_KEY, MOD_COMMAND, MOD_CONTROL, MOD_OPTION, MOD_PRIMARY, MOD_SHIFT, errorText, type Key,
} from './util'

const props = withDefaults(defineProps<{
  modelValue: Key | null
  /** card：工作流/剪贴板的大按钮；inline：键鼠快捷键行里的组合键按钮。 */
  variant?: 'card' | 'inline'
  /** 至少需要的修饰键位；0 表示不要求。 */
  modifierMask?: number
  modifierMessage?: string
  validate?: (key: Key) => string | null
  removable?: boolean
  disabled?: boolean
  hint?: string
}>(), { variant: 'card', modifierMask: MOD_PRIMARY, modifierMessage: '请同时按住 ⌘、⌥ 或 ⌃', removable: true })

const emit = defineEmits<{ 'update:modelValue': [value: Key | null]; recording: [value: boolean] }>()

const recording = ref(false)
const message = ref<string | null>(null)

async function setEngineRecording(enabled: boolean) {
  // 录制期间暂停引擎与剪贴板全局快捷键，避免要录的组合被抢先处理。
  await backend.invoke('input.set_recording', { enabled, suspend_shortcut: true })
}

async function begin() {
  if (props.disabled || recording.value) return
  message.value = null
  try {
    await setEngineRecording(true)
  } catch (e) {
    message.value = `引擎尚未就绪，请稍后重试（${errorText(e)}）`
    return
  }
  recording.value = true
  emit('recording', true)
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', cancel)
}

function stop() {
  if (!recording.value) return
  recording.value = false
  window.removeEventListener('keydown', onKey, true)
  window.removeEventListener('blur', cancel)
  emit('recording', false)
  setEngineRecording(false).catch((e) => { message.value = errorText(e) })
}

function cancel() {
  stop()
  message.value = null
}

function onKey(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
  if (/^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/.test(event.code)) return
  const flags =
    (event.shiftKey ? MOD_SHIFT : 0) | (event.ctrlKey ? MOD_CONTROL : 0) |
    (event.altKey ? MOD_OPTION : 0) | (event.metaKey ? MOD_COMMAND : 0)
  if (event.code === 'Escape' && flags === 0) return cancel()
  const code = CODE_TO_KEY[event.code]
  if (code === undefined) { message.value = '这个按键不能用作快捷键'; return }
  if (!(flags & props.modifierMask)) { message.value = props.modifierMessage; return }
  const key: Key = { key_code: code, modifiers: flags }
  const conflict = props.validate?.(key)
  if (conflict) { message.value = conflict; return }
  message.value = null
  emit('update:modelValue', key)
  stop()
}

function remove() {
  emit('update:modelValue', null)
  message.value = null
}

onBeforeUnmount(stop)
</script>

<template>
  <div v-if="variant === 'inline'" class="inline">
    <button type="button" class="chip" :disabled="disabled" title="更改快捷键" @click="begin">
      <span v-if="recording" class="rec">● 按下组合键…</span>
      <Keycaps v-else-if="modelValue" :shortcut="modelValue" />
      <span v-else class="muted">未设置</span>
    </button>
    <small v-if="message" class="err" role="alert">{{ message }}</small>
  </div>

  <div v-else class="card-rec">
    <div class="head">
      <strong>全局快捷键</strong>
      <button v-if="recording" type="button" @click="cancel">取消录制</button>
      <button v-else-if="modelValue && removable" type="button" class="link" :disabled="disabled" @click="remove">移除绑定</button>
    </div>
    <button type="button" class="surface" :class="{ active: recording }" :disabled="disabled"
      :aria-label="recording ? '正在录制快捷键，按 Esc 取消' : '录制全局快捷键'" @click="begin">
      <span class="icon"><span v-if="recording" class="rec-dot">●</span><Keyboard v-else :size="18" /></span>
      <span class="copy">
        <span class="t">{{ recording ? '现在按下组合键' : modelValue ? '已设置快捷键' : '点击录制快捷键' }}</span>
        <span class="d">{{ recording ? '按 Esc 取消，原绑定保持不变' : modelValue ? '点击此处重新录制' : '例如 ⌃ ⌥ V，运行整个工作流' }}</span>
      </span>
      <Keycaps v-if="!recording && modelValue" :shortcut="modelValue" />
    </button>
    <p v-if="message" class="err" role="alert">{{ message }}</p>
    <p v-else class="hint">{{ hint ?? '包含 ⌘、⌥ 或 ⌃ 中至少一个修饰键。绑定在保存后生效。' }}</p>
  </div>
</template>

<style scoped>
.inline { display: inline-flex; flex-direction: column; align-items: flex-end; gap: 4px; }
.chip { display: inline-flex; align-items: center; min-height: 24px; padding: 0; border: 0; background: transparent; cursor: pointer; }
.rec { color: var(--accent); font-size: 12px; }
.muted { color: var(--secondary); font-size: 12px; }
.card-rec { display: grid; gap: 10px; padding: 4px 0; }
.head { display: flex; align-items: center; justify-content: space-between; line-height: 16px; }
.head button { font-size: 12px; line-height: 16px; }
.link { border: 0; background: transparent; cursor: pointer; font-size: 12px; color: var(--accent); }
.surface { display: flex; align-items: center; gap: 12px; width: 100%; padding: 14px; border: 1px solid color-mix(in srgb, var(--text) 12%, transparent); border-radius: 8px; background: color-mix(in srgb, var(--text) 3.5%, transparent); text-align: left; cursor: pointer; }
.surface.active { border: 2px solid var(--accent); background: color-mix(in srgb, var(--accent) 8%, transparent); }
.icon { display: inline-flex; color: var(--secondary); font-size: 16px; }
.rec-dot { color: var(--accent); font-size: 14px; }
.active .icon { color: var(--accent); }
.copy { display: grid; flex: 1; gap: 4px; }
.t { font-size: 13px; font-weight: 500; line-height: 16px; }
.d { font-size: 11px; line-height: 13px; color: var(--secondary); }
.hint { margin: 0; font-size: 11px; line-height: 14px; color: var(--secondary); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

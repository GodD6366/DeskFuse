<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Copy, Images, Keyboard, Layers, Lock, Mouse } from 'lucide-vue-next'
import { backend } from '../../services/backend'
import ConfirmDialog from './ConfirmDialog.vue'
import Keycaps from './Keycaps.vue'
import SettingsSection from './SettingsSection.vue'
import ShortcutRecorder from './ShortcutRecorder.vue'
import { DEFAULT_CLIPBOARD_SHORTCUT, useClipboardSettings } from './useClipboardSettings'
import { useWorkflows } from './useWorkflows'
import {
  MOD_ANY, MOD_COMMAND, MOD_CONTROL, MOD_OPTION, MOD_PRIMARY, MOD_SHIFT, errorText, friendlyError, get,
  inputShortcuts, preference, reservedConflict, sameKey, useAction, type Key, type Snapshot,
} from './util'
import { actionTitle, type WorkflowDefinition } from './workflow'

const props = defineProps<{ snapshot: Snapshot }>()
const emit = defineEmits<{ navigate: [target: string] }>()

const wf = useWorkflows()
const library = useClipboardSettings()
const workflowsEnabled = computed(() => preference(props.snapshot, 'workflows_enabled') === true)

// ---- 剪贴板唤起快捷键：修改后立即提交 ----
const clipLoaded = ref(false)
const clipLoadError = ref<string | null>(null)
const clipSaving = ref(false)
const clipMessage = ref<string | null>(null)
const clipFailed = ref(false)

async function loadClipboard() {
  clipLoadError.value = null
  try {
    await library.load()
    clipLoaded.value = true
  } catch (e) {
    clipLoadError.value = errorText(e)
  }
}

function clipConflict(key: Key | null): string | null {
  if (!key) return null
  if (key.key_code === 9 && key.modifiers === MOD_COMMAND) return '⌘V 保留给普通粘贴。'
  if (key.key_code === 53 && (key.modifiers & MOD_PRIMARY) === MOD_PRIMARY) return '这组按键用于紧急收回。'
  if (key.key_code === 2 && key.modifiers === (MOD_SHIFT | MOD_COMMAND)) return '这组按键用于文件暂存区。'
  if (inputShortcuts(props.snapshot).some((s) => sameKey(s, key))) return '这组按键已用于键鼠控制。'
  const other = wf.definitions.value.find((f) => f.enabled && sameKey(f.shortcut, key))
  return other ? `这组按键已绑定到“${other.name}”。` : null
}

async function applyClipboard(key: Key | null) {
  const conflict = clipConflict(key)
  if (conflict) { clipFailed.value = true; clipMessage.value = conflict; return }
  clipSaving.value = true
  clipFailed.value = false
  clipMessage.value = null
  try {
    // 只改快捷键：其余设置以引擎中的最新值为准。
    const latest = await library.load()
    const next = { ...latest, shortcut: { enabled: key !== null, key_code: key?.key_code ?? 9, modifiers: key?.modifiers ?? 0x180000 } }
    await library.save(next)
    try {
      await backend.invoke('clipboard.configure_shortcut', { shortcut: key })
      clipMessage.value = key ? '已生效' : '已关闭全局唤起'
    } catch (e) {
      clipFailed.value = true
      clipMessage.value = `快捷键已保存，但${errorText(e)}`
    }
  } catch (e) {
    clipFailed.value = true
    clipMessage.value = errorText(e)
  } finally {
    clipSaving.value = false
  }
}

// ---- 工作流快捷键 ----
const editing = ref<WorkflowDefinition | null>(null)
const editingRecording = ref(false)
const editSaving = ref(false)
const editFailure = ref<string | null>(null)

function startEdit(flow: WorkflowDefinition) {
  editing.value = JSON.parse(JSON.stringify(flow))
  editFailure.value = null
}

function validateWorkflowKey(key: Key): string | null {
  const reserved = reservedConflict(key)
  if (reserved) return `${reserved}，请换一组按键`
  if (sameKey(library.shortcutKey.value, key)) return '这组按键已用于唤起剪贴板，请换一组按键'
  if (inputShortcuts(props.snapshot).some((s) => sameKey(s, key))) return '这组按键已用于键鼠控制，请换一组按键'
  const other = wf.definitions.value.find((f) => f.id !== editing.value?.id && f.enabled && sameKey(f.shortcut, key))
  return other ? `已绑定到“${other.name}”，请换一组按键` : null
}

async function saveEdit() {
  if (!editing.value) return
  editSaving.value = true
  editFailure.value = null
  try {
    await wf.save(editing.value)
    editing.value = null
  } catch (e) {
    editFailure.value = errorText(e)
  } finally {
    editSaving.value = false
  }
}

// ---- 键鼠控制快捷键：编辑后立即提交引擎 ----
const TITLES = ['向左切换', '向右切换', '向上切换', '向下切换', '锁定当前设备', '紧急收回']
const input = computed(() => get<any>(props.snapshot, 'input'))
const inputSettings = computed(() => get<any>(input.value, 'settings'))
const draft = ref<Key[] | null>(null)
const shortcuts = computed<Key[]>(() => draft.value ?? inputShortcuts(props.snapshot))
const inputError = ref<string | null>(null)
const reclaim = useAction()

function validateInput(index: number, key: Key): string | null {
  if (sameKey(library.shortcutKey.value, key)) return '这组按键已用于唤起剪贴板，请换一组按键'
  if ((index !== 5 && key.key_code === 53 && key.modifiers === MOD_PRIMARY) ||
      (key.key_code === 2 && key.modifiers === (MOD_SHIFT | MOD_COMMAND))) {
    return '这组按键是 DeskFuse 保留快捷键，请换一组按键'
  }
  if (index === 5 && !(key.modifiers & MOD_PRIMARY)) return '紧急收回需包含 Command、Option 或 Control'
  if (shortcuts.value.some((s, i) => i !== index && sameKey(s, key))) return '这组按键已用于其他键鼠操作，请换一组按键'
  return null
}

async function commitShortcuts(next: Key[]) {
  const settings = inputSettings.value
  if (!settings) return
  const previous = draft.value
  draft.value = next
  inputError.value = null
  try {
    await backend.invoke('set_input', { settings: { ...settings, shortcuts: next } })
  } catch (e) {
    draft.value = previous
    inputError.value = errorText(e)
  }
}

function changeShortcut(index: number, key: Key | null) {
  if (!key) return
  const next = shortcuts.value.map((s, i) => (i === index ? key : s))
  commitShortcuts(next)
}

function restoreEmergency() {
  const next = shortcuts.value.map((s, i) => (i === 5 ? { key_code: 53, modifiers: MOD_CONTROL | MOD_OPTION | MOD_COMMAND } : s))
  commitShortcuts(next)
}

const FIXED = [
  { title: '紧急收回（保底）', detail: '自定义按键之外，此组合始终可用', key: { key_code: 53, modifiers: 0x1c0000 } },
  { title: '文件暂存区', detail: '打开或收起暂存区', key: { key_code: 2, modifiers: 0x120000 } },
]

onMounted(() => {
  loadClipboard()
  wf.reload()
})
</script>

<template>
  <div class="page">
    <div class="intro">
      <Keyboard :size="20" class="intro-icon" />
      <p>DeskFuse 的全部全局快捷键都在这里，点组合键即可修改，立即生效。</p>
    </div>
    <div class="scroll">
      <div class="content">
        <SettingsSection title="剪贴板" :symbol="Copy">
          <template v-if="clipLoaded">
            <div class="clip" :aria-busy="clipSaving">
              <div class="clip-title">唤起剪贴板</div>
              <p class="caption">默认 ⌥⌘V。面板内双击、回车或 ⌘1–9 粘贴回原应用；移除绑定可关闭全局唤起。</p>
              <ShortcutRecorder
                :model-value="library.shortcutKey.value" :disabled="clipSaving"
                hint="包含 ⌘、⌥ 或 ⌃ 中至少一个修饰键。绑定在保存后生效。"
                :validate="(key: Key) => clipConflict(key)" @update:model-value="applyClipboard" />
              <div><button type="button" class="button" :disabled="clipSaving" @click="applyClipboard({ ...DEFAULT_CLIPBOARD_SHORTCUT })">恢复默认</button></div>
            </div>
            <p v-if="clipMessage" class="msg" :class="{ err: clipFailed }">{{ clipFailed ? friendlyError(clipMessage) : clipMessage }}</p>
          </template>
          <div v-else-if="clipLoadError" class="line">
            <span class="err">{{ friendlyError(clipLoadError) }}</span>
            <button type="button" class="button" @click="loadClipboard">重试</button>
          </div>
          <p v-else class="caption">正在读取剪贴板快捷键…</p>
        </SettingsSection>

        <SettingsSection v-if="workflowsEnabled" title="工作流" :symbol="Images" footer="点击右侧组合键即可修改。停用的流程不会响应快捷键；全局触发需要辅助功能权限。">
          <div v-for="flow in wf.definitions.value" :key="flow.id" class="wf-row">
            <Layers :size="14" class="wf-icon" />
            <div class="wf-text">
              <div class="wf-name">{{ flow.name }}<span v-if="!flow.enabled" class="caption"> 已停用</span></div>
              <div class="caption">{{ flow.steps.map((s) => actionTitle(s.kind)).join(' → ') }}</div>
            </div>
            <button type="button" class="chip" :title="`编辑“${flow.name}”的快捷键`" @click="startEdit(flow)">
              <Keycaps v-if="flow.shortcut" :shortcut="flow.shortcut" />
              <span v-else class="set-key">＋ 设置快捷键</span>
            </button>
          </div>
          <div v-if="!wf.definitions.value.length" class="empty">
            <strong>还没有工作流</strong>
            <p class="empty-note">先组合压缩、转格式和粘贴，再为整个流程设置快捷键。</p>
          </div>
          <p v-if="wf.loadError.value" class="err">{{ wf.loadError.value }}</p>
          <div><button type="button" class="button" @click="emit('navigate', 'workflows')">管理工作流…</button></div>
        </SettingsSection>

        <SettingsSection title="键鼠控制" :symbol="Mouse" footer="点右侧组合键即可修改，立即生效。方向切换与锁定遵循键鼠共享状态。">
          <template v-if="input && inputSettings">
            <div class="keys">
              <div v-for="(shortcut, index) in shortcuts" :key="index" class="key-row">
                <span>{{ TITLES[index] ?? `快捷键 ${index + 1}` }}</span>
                <ShortcutRecorder
                  variant="inline" :model-value="shortcut" :modifier-mask="index === 5 ? MOD_PRIMARY : MOD_ANY"
                  :modifier-message="index === 5 ? '紧急收回需包含 Command、Option 或 Control' : '需要至少一个修饰键'"
                  :validate="(key: Key) => validateInput(index, key)" @update:model-value="(key: Key | null) => changeShortcut(index, key)" />
              </div>
              <button v-if="shortcuts.length > 5" type="button" class="caption-button" @click="restoreEmergency">恢复默认紧急收回快捷键</button>
              <p v-if="inputError" class="err" role="alert">{{ inputError }}</p>
              <p class="caption">紧急收回支持自定义；⌃⌥⌘ Esc 始终保留为保底快捷键，无需找到鼠标。</p>
              <div v-if="!get(input, 'managed')"><button type="button" class="button" @click="emit('navigate', 'input')">打开键鼠共享</button></div>
              <template v-else>
                <p class="caption">{{ get(input, 'summary') }}</p>
                <div><button type="button" class="button" :disabled="get(input, 'managed_paused') || reclaim.pending.value" @click="reclaim.run('input.reclaim')">停止被控</button></div>
                <p v-if="reclaim.error.value" class="err" role="alert">{{ reclaim.error.value }}</p>
              </template>
            </div>
          </template>
          <p v-else class="caption">{{ snapshot ? '键鼠设置不可用' : '后端未连接，暂时无法读取键鼠设置' }}</p>
        </SettingsSection>

        <SettingsSection title="固定快捷键" :symbol="Lock" footer="这些组合键由 DeskFuse 保留，不能分配给工作流。">
          <div v-for="item in FIXED" :key="item.title" class="fixed-row">
            <div class="wf-text">
              <div>{{ item.title }}</div>
              <div class="caption">{{ item.detail }}</div>
            </div>
            <span class="fixed"><Keycaps :shortcut="item.key" /><Lock :size="11" class="lock" aria-label="已锁定" /></span>
          </div>
        </SettingsSection>
      </div>
    </div>

    <ConfirmDialog
      :open="!!editing" title="设置快捷键" :message="editing?.name" confirm-text="保存绑定" :busy="editSaving"
      :confirm-disabled="editingRecording" :error="editFailure" :width="500" @confirm="saveEdit" @cancel="editing = null">
      <ShortcutRecorder
        v-if="editing" :model-value="editing.shortcut" :disabled="editSaving" :validate="validateWorkflowKey"
        @update:model-value="(key: Key | null) => { if (editing) editing.shortcut = key }" @recording="editingRecording = $event" />
      <p v-if="editing && !editing.enabled" class="caption">流程已停用；保存绑定不会自动启用流程。</p>
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.intro { display: flex; align-items: center; gap: 12px; padding: 10px 26px 12px; }
.intro-icon { flex: none; color: var(--secondary); }
.intro p { margin: 0; font-size: 12px; color: var(--secondary); }
.scroll { flex: 1; min-height: 0; overflow: auto; padding: 4px 24px 24px; }
.content { display: grid; align-content: start; gap: 24px; max-width: 720px; }
.clip { display: grid; gap: 12px; }
.clip-title { font-size: 13px; font-weight: 600; line-height: 16px; }
.caption { margin: 0; font-size: 11px; line-height: 14px; color: var(--secondary); }
.wf-row { display: flex; align-items: center; gap: 12px; padding: 5px 0; }
.wf-icon { flex: none; width: 24px; color: var(--secondary); }
.wf-text { display: grid; flex: 1; gap: 4px; min-width: 0; }
.wf-name { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chip { display: inline-flex; align-items: center; min-height: 24px; padding: 0; border: 0; background: transparent; cursor: pointer; font-size: 13px; }
.set-key { font-size: 12px; color: var(--secondary); }
.empty { display: grid; gap: 6px; padding: 6px 0; }
.empty strong { font-weight: 500; }
.empty-note { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.keys { display: grid; gap: 8px; }
.key-row { display: flex; align-items: center; gap: 12px; min-height: 24px; font-size: 13px; }
.key-row > :last-child { margin-left: auto; }
.caption-button {
  justify-self: start; min-height: 32px; padding: 0 12px; border: 1px solid var(--line); border-radius: 8px;
  background: var(--surface); color: var(--text); font-size: 11px; cursor: pointer;
}
.fixed-row { display: flex; align-items: center; gap: 12px; padding: 3px 0; font-size: 13px; }
.fixed { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
.lock { color: var(--secondary); opacity: 0.55; }
.line { display: flex; align-items: center; gap: 8px; }
.msg { margin: 0; font-size: 11px; color: var(--secondary); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

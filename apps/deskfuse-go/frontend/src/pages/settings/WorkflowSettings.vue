<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { Layers, Minus, Plus } from 'lucide-vue-next'
import ConfirmDialog from './ConfirmDialog.vue'
import ShortcutRecorder from './ShortcutRecorder.vue'
import SettingsSection from './SettingsSection.vue'
import WorkflowStepEditor from './WorkflowStepEditor.vue'
import { useWorkflows } from './useWorkflows'
import { useClipboardSettings } from './useClipboardSettings'
import { errorText, get, inputShortcuts, reservedConflict, sameKey, type Key, type Snapshot } from './util'
import {
  ACTIONS, CONTENT_TITLES, chatImageExample, incompatibilities, isTerminal, newStep, newWorkflow,
  summary, websiteImageExample, type WorkflowAction, type WorkflowContent, type WorkflowDefinition,
} from './workflow'

const props = defineProps<{ snapshot: Snapshot }>()
const wf = useWorkflows()
const clipboard = useClipboardSettings()

const selectedID = ref<string | null>(null)
const draft = ref<WorkflowDefinition | null>(null)
const original = ref<WorkflowDefinition | null>(null)
const saving = ref(false)
const failure = ref<string | null>(null)
const syncFailure = ref<string | null>(null)
const deleteRequested = ref(false)
const switchID = ref<string | null>(null)
const confirmSwitch = ref(false)
const menuOpen = ref(false)
const addOpen = ref(false)

const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value))
const dirty = computed(() => JSON.stringify(draft.value) !== JSON.stringify(original.value))
const issues = computed(() => (draft.value ? incompatibilities(draft.value) : []))
const canSave = computed(() => dirty.value && issues.value.length === 0 && !!draft.value?.name.trim() && !saving.value)
const stale = computed(() => {
  const saved = original.value
  if (!saved) return false
  const latest = wf.definitions.value.find((f) => f.id === saved.id)
  return JSON.stringify(latest) !== JSON.stringify(saved)
})

function load(id: string | null) {
  selectedID.value = id
  const found = wf.definitions.value.find((f) => f.id === id)
  original.value = found ? clone(found) : null
  draft.value = found ? clone(found) : null
  failure.value = null
}

function select(id: string) {
  if (dirty.value) { switchID.value = id; confirmSwitch.value = true } else load(id)
}

function newDraft(template: WorkflowDefinition = newWorkflow()) {
  if (dirty.value) { failure.value = '请先保存或撤销当前流程修改'; return }
  menuOpen.value = false
  selectedID.value = template.id
  original.value = null
  draft.value = template
  failure.value = null
}

async function saveDraft() {
  if (!draft.value || !canSave.value) return
  const flow = clone(draft.value)
  saving.value = true
  failure.value = null
  try {
    await wf.save(flow)
    load(flow.id)
  } catch (e) {
    failure.value = errorText(e)
  } finally {
    saving.value = false
  }
}

async function duplicate() {
  if (!draft.value || dirty.value) return
  const copy = clone(draft.value)
  copy.id = crypto.randomUUID()
  copy.name += ' 副本'
  copy.shortcut = null
  for (const step of copy.steps) {
    if (step.local_shortcut_id) step.shortcut_id = step.local_shortcut_id
  }
  failure.value = null
  try {
    await wf.save(copy)
    load(copy.id)
  } catch (e) {
    failure.value = errorText(e)
  }
}

async function removeSelected() {
  deleteRequested.value = false
  const id = original.value?.id
  if (!id) return
  failure.value = null
  try {
    await wf.remove(id)
    selectedID.value = null; draft.value = null; original.value = null
  } catch (e) {
    failure.value = errorText(e)
  }
}

function revert() { draft.value = original.value ? clone(original.value) : null }

function addStep(kind: WorkflowAction) {
  addOpen.value = false
  if (!draft.value) return
  draft.value.steps.splice(Math.max(0, draft.value.steps.length - 1), 0, newStep(kind))
}

function move(index: number, offset: number) {
  const steps = draft.value?.steps
  if (!steps) return
  ;[steps[index], steps[index + offset]] = [steps[index + offset], steps[index]]
}

const setShortcut = (key: Key | null) => { if (draft.value) draft.value.shortcut = key }

function validateShortcut(key: Key): string | null {
  const reserved = reservedConflict(key)
  if (reserved) return `${reserved}，请换一组按键`
  if (sameKey(clipboard.shortcutKey.value, key)) return '这组按键已用于唤起剪贴板，请换一组按键'
  if (inputShortcuts(props.snapshot).some((s) => sameKey(s, key))) return '这组按键已用于键鼠控制，请换一组按键'
  const other = wf.definitions.value.find((f) => f.id !== draft.value?.id && f.enabled && sameKey(f.shortcut, key))
  return other ? `已绑定到“${other.name}”，请换一组按键` : null
}

function issueFor(id: string) {
  return wf.catalog.value.issues.find((i) => i.workflow_id === id)?.message
}

const syncTitle = (status: string) =>
  ({ synced: '同步完成', pending: '等待同步', waiting_for_devices: '等待已配对设备连接', error: '同步失败，已保留本地修改' } as Record<string, string>)[status] ?? status

const peerName = (id: string) =>
  (get<any[]>(props.snapshot, 'devices') ?? []).find((d) => d.id === id)?.name ?? id.slice(0, 8)

async function restore(id: string) {
  syncFailure.value = null
  try { await wf.restoreConflict(id) } catch (e) { syncFailure.value = errorText(e) }
}

const visibleRuns = computed(() => wf.runs.value.slice(0, 3))
const runName = (run: any) => wf.definitions.value.find((f) => f.id === get(run, 'workflow_id'))?.name ?? '工作流'
function runTitle(run: any) {
  const name = runName(run)
  switch (run.phase) {
    case 'running': return `${name} · 第 ${(get<number>(run, 'step_index') ?? 0) + 1} 步`
    case 'completed': return `${name} · 已完成`
    case 'cancelled': return `${name} · 已取消`
    case 'failed': return `${name} · 失败`
    default: return name
  }
}

onMounted(async () => {
  clipboard.load().catch(() => undefined)
  await wf.reload()
  if (!draft.value) load(wf.definitions.value[0]?.id ?? null)
})
watch(wf.definitions, (latest) => {
  if (!draft.value) load(latest[0]?.id ?? null)
  else if (!dirty.value) load(selectedID.value)
})
// 运行状态随 1.5 秒快照轮询更新；workflow.list 不返回运行列表。
watch(
  () => props.snapshot?.workflow_runs,
  (runs) => { if (Array.isArray(runs)) wf.runs.value = runs },
  { immediate: true, deep: true },
)
</script>

<template>
  <div class="wf">
    <aside class="list">
      <div class="list-head"><strong>工作流</strong><span class="count">{{ wf.definitions.value.length }}</span></div>

      <div v-if="wf.catalog.value.sync_status || wf.catalog.value.peers.length || wf.catalog.value.conflicts.length || wf.catalog.value.error || syncFailure" class="sync">
        <p v-if="wf.catalog.value.sync_status" class="note">↻ {{ syncTitle(wf.catalog.value.sync_status) }}</p>
        <p v-for="peer in wf.catalog.value.peers" :key="peer.device_id" class="note">{{ peerName(peer.device_id) }}：{{ syncTitle(peer.status) }}</p>
        <div v-for="conflict in wf.catalog.value.conflicts" :key="conflict.id" class="conflict">
          <div><strong>{{ conflict.name }}</strong><p class="note">{{ conflict.message }}</p></div>
          <button type="button" @click="restore(conflict.id)">恢复保留版本</button>
        </div>
        <p v-if="wf.catalog.value.error" class="err">{{ wf.catalog.value.error }}</p>
        <p v-if="syncFailure" class="err">{{ syncFailure }}</p>
      </div>

      <ul class="items" role="listbox" aria-label="工作流">
        <li v-for="flow in wf.definitions.value" :key="flow.id" role="option" :aria-selected="flow.id === selectedID"
          :class="{ on: flow.id === selectedID }" tabindex="0" @click="select(flow.id)" @keydown.enter="select(flow.id)">
          <Layers :size="14" class="item-icon" />
          <div class="item-text">
            <strong class="name">{{ flow.name }}</strong>
            <span class="note">{{ summary(flow) }}</span>
            <span v-if="issueFor(flow.id)" class="warn">⚠ {{ issueFor(flow.id) }}</span>
            <span v-if="flow.steps.some((s) => s.kind === 'SystemShortcut' && !s.local_shortcut_id)" class="warn">⚠ 本机快捷指令待绑定</span>
          </div>
        </li>
        <li v-if="!wf.definitions.value.length && !draft" class="none">{{ wf.loadError.value ?? (wf.loading.value ? '正在读取…' : '尚未保存流程') }}</li>
      </ul>

      <div class="tools">
        <div class="menu">
          <button type="button" class="tool-icon" title="新建流程或使用示例" aria-label="新建流程或使用示例" aria-haspopup="menu" :aria-expanded="menuOpen" @click="menuOpen = !menuOpen"><Plus :size="14" /></button>
          <div v-if="menuOpen" class="pop card" role="menu">
            <button type="button" role="menuitem" @click="newDraft()">新建流程</button>
            <button type="button" role="menuitem" @click="newDraft(chatImageExample())">示例：聊天图片</button>
            <button type="button" role="menuitem" @click="newDraft(websiteImageExample())">示例：网站配图</button>
          </div>
        </div>
        <button type="button" class="tool-icon" :disabled="!original" title="删除流程" aria-label="删除流程" @click="deleteRequested = true"><Minus :size="14" /></button>
        <span class="spacer" />
        <button type="button" class="button" :disabled="!draft || dirty || !original" @click="duplicate">复制</button>
      </div>
    </aside>

    <section v-if="draft" class="editor">
      <div class="scroll">
        <SettingsSection v-if="stale">
          <p class="warn">其他设备已更新或删除此工作流。当前草稿已保留；保存会创建新修订。</p>
          <div><button type="button" class="button" @click="load(selectedID)">放弃草稿并载入最新版本</button></div>
        </SettingsSection>

        <SettingsSection title="流程">
          <input v-model="draft.name" class="field" placeholder="名称" aria-label="名称" />
          <label class="line">接受内容
            <select v-model="draft.input_type" class="field">
              <option v-for="(title, value) in CONTENT_TITLES" :key="value" :value="value as WorkflowContent">{{ title }}</option>
            </select>
          </label>
          <label class="line"><span>启用流程</span>
            <input v-model="draft.enabled" class="toggle" type="checkbox" role="switch" aria-label="启用流程" />
          </label>
          <ShortcutRecorder :model-value="draft.shortcut" :validate="validateShortcut" @update:model-value="setShortcut" />
        </SettingsSection>

        <SettingsSection
          title="依次处理"
          footer="中间结果不会写入剪贴板，源文件保持不变。图片步骤不遍历文件夹，混合非图片文件会明确失败；目标体积约束最终输出。用每行右侧的箭头调整步骤顺序，终点始终保留在末尾。">
          <WorkflowStepEditor
            v-for="(step, index) in draft.steps" :key="index" v-model="draft.steps[index]" :index="index"
            :can-move-up="index > 0 && !isTerminal(step.kind)" :can-move-down="index + 2 < draft.steps.length"
            @move="(offset: number) => move(index, offset)" @remove="draft.steps.splice(index, 1)" />
          <div class="add-wrap">
            <button type="button" class="button" :disabled="draft.steps.length >= 32" :aria-expanded="addOpen" @click="addOpen = !addOpen">＋ 添加动作</button>
            <div v-if="addOpen" class="pop card add-pop" role="menu">
              <button v-for="a in ACTIONS.filter((x) => !isTerminal(x.kind))" :key="a.kind" type="button" role="menuitem" @click="addStep(a.kind)">{{ a.title }}</button>
            </div>
          </div>
          <p v-for="issue in issues" :key="issue" class="warn">⚠ {{ issue }}</p>
        </SettingsSection>
      </div>

      <div v-if="visibleRuns.length || wf.catalog.value.error" class="runs">
        <div v-for="run in visibleRuns" :key="run.id" class="run">
          <div>
            <strong>{{ runTitle(run) }}</strong>
            <p v-if="run.message" class="note">{{ run.message }}</p>
          </div>
          <button v-if="run.phase === 'running'" type="button" @click="wf.cancelRun(run.id)">取消</button>
        </div>
      </div>

      <footer class="foot-bar">
        <div class="bar">
          <span v-if="dirty" class="note">未保存</span>
          <span class="spacer" />
          <button type="button" class="button" :disabled="!dirty || !original" @click="revert">撤销修改</button>
          <button type="button" class="button primary" :disabled="!canSave" @click="saveDraft">{{ saving ? '保存中…' : '保存流程' }}</button>
        </div>
        <p v-if="failure" class="err" role="alert">{{ failure }}</p>
      </footer>
    </section>

    <section v-else class="intro">
      <Layers :size="40" class="intro-icon" />
      <div class="intro-copy">
        <h2>组合你的内容处理动作</h2>
        <p>整理文字、重命名文件、打包或处理图片，<br />也可在共享给其他设备时自动运行。</p>
      </div>
      <button type="button" class="button primary" @click="newDraft()">新建流程</button>
      <p v-if="failure" class="err" role="alert">{{ failure }}</p>
    </section>

    <ConfirmDialog :open="deleteRequested" title="删除这个工作流？" confirm-text="删除" destructive @confirm="removeSelected" @cancel="deleteRequested = false" />
    <ConfirmDialog :open="confirmSwitch" title="舍弃未保存的修改？" confirm-text="舍弃修改" destructive
      @confirm="() => { confirmSwitch = false; load(switchID) }" @cancel="confirmSwitch = false" />
  </div>
</template>

<style scoped>
.wf { display: flex; height: 100%; min-height: 0; padding-top: 8px; }
.list { display: flex; flex-direction: column; width: 220px; flex-shrink: 0; border-right: 0.5px solid var(--line); }
.list-head { display: flex; align-items: center; justify-content: space-between; padding: 12px; }
.list-head strong { font-size: 13px; font-weight: 600; }
.count { font-size: 11px; color: var(--secondary); }
.sync { display: grid; gap: 6px; max-height: 115px; padding: 0 12px 8px; overflow: auto; border-bottom: 0.5px solid var(--line); }
.conflict { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
.items { flex: 1; margin: 0; padding: 4px; overflow: auto; list-style: none; }
.items li { display: flex; align-items: flex-start; gap: 12px; padding: 8px; border-radius: 8px; cursor: pointer; }
.items li.on { background: var(--control); }
.items li.none { color: var(--secondary); font-size: 13px; cursor: default; text-align: center; justify-content: center; }
.item-icon { flex: none; margin-top: 2px; color: var(--secondary); }
.item-text { display: grid; gap: 2px; min-width: 0; }
.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
.tools { display: flex; align-items: center; gap: 6px; padding: 10px; border-top: 0.5px solid var(--line); }
.menu { position: relative; }
.pop { position: absolute; bottom: 36px; left: 0; z-index: 10; display: grid; min-width: 150px; padding: 4px; background: var(--surface); }
.pop button { padding: 6px 8px; border: 0; background: transparent; text-align: left; cursor: pointer; }
.spacer { flex: 1; }
.tool-icon { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border-radius: 6px; background: transparent; color: var(--text); }
.tool-icon:hover:not(:disabled) { background: var(--control); }
.editor { display: flex; flex: 1; flex-direction: column; min-width: 360px; min-height: 0; }
.scroll { display: grid; flex: 1; align-content: start; gap: 24px; padding: 4px 24px 24px; overflow: auto; }
.line { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 13px; white-space: nowrap; }
.line select.field { flex: 0 1 240px; min-width: 160px; margin-left: auto; }
.add-wrap { position: relative; justify-self: start; }
.add-pop { bottom: auto; top: 36px; min-width: 200px; }
.runs { max-height: 150px; padding: 8px 12px; overflow: auto; border-top: 0.5px solid var(--line); }
.run { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; padding: 4px 0; }
.foot-bar { display: grid; gap: 6px; padding: 12px; border-top: 0.5px solid var(--line); }
.bar { display: flex; align-items: center; gap: 8px; }
.bar .button.primary:disabled { background: var(--control); color: var(--secondary); border-color: transparent; opacity: 1; }
.intro {
  display: flex; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 16px;
  padding: 32px; background: var(--surface); text-align: center;
}
.intro-icon { color: var(--secondary); stroke-width: 1.2; }
.intro-copy { display: grid; gap: 8px; }
.intro h2 { margin: 0; font-size: 15px; font-weight: 700; }
.intro p { margin: 0; color: var(--secondary); font-size: 12px; line-height: 1.6; }
.note { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.warn { margin: 0; font-size: 12px; line-height: 1.5; color: var(--warning); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

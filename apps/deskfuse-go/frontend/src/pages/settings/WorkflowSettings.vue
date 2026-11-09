<script setup lang="ts">
import { computed, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue'
import { CheckCircle2, Circle, CircleAlert, CircleSlash, Layers, Loader2, Plus, TriangleAlert } from 'lucide-vue-next'
import ConfirmDialog from './ConfirmDialog.vue'
import SettingsSection from './SettingsSection.vue'
import WorkflowChain from './WorkflowChain.vue'
import WorkflowConflicts from './WorkflowConflicts.vue'
import WorkflowEditorHeader from './WorkflowEditorHeader.vue'
import WorkflowList from './WorkflowList.vue'
import WorkflowPipeline from './WorkflowPipeline.vue'
import { useWorkflows } from './useWorkflows'
import { useClipboardSettings } from './useClipboardSettings'
import { errorText, get, inputShortcuts, reservedConflict, sameKey, type Key, type Snapshot } from './util'
import {
  chatImageExample, incompatibilities, newWorkflow, websiteImageExample, type WorkflowDefinition,
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

const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value))
const dirty = computed(() => JSON.stringify(draft.value) !== JSON.stringify(original.value))
const issues = computed(() => (draft.value ? incompatibilities(draft.value) : []))
const unnamed = computed(() => !draft.value?.name.trim())
const canSave = computed(() => dirty.value && issues.value.length === 0 && !unnamed.value && !saving.value)
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

// 已保存的流程：撤销回到保存时的样子；新建但未保存的草稿：放弃并回到列表第一项或空状态。
const isNewDraft = computed(() => !!draft.value && !original.value)
function undo() {
  if (original.value) draft.value = clone(original.value)
  else load(wf.definitions.value[0]?.id ?? null)
  failure.value = null
}

function create(template: 'blank' | 'chat' | 'website') {
  newDraft(template === 'chat' ? chatImageExample() : template === 'website' ? websiteImageExample() : newWorkflow())
}

function validateShortcut(key: Key): string | null {
  const reserved = reservedConflict(key)
  if (reserved) return `${reserved}，请换一组按键`
  if (sameKey(clipboard.shortcutKey.value, key)) return '这组按键已用于唤起剪贴板，请换一组按键'
  if (inputShortcuts(props.snapshot).some((s) => sameKey(s, key))) return '这组按键已用于键鼠控制，请换一组按键'
  const other = wf.definitions.value.find((f) => f.id !== draft.value?.id && f.enabled && sameKey(f.shortcut, key))
  return other ? `已绑定到“${other.name}”，请换一组按键` : null
}

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
const RUN_ICONS: Record<string, { icon: any; tone: string }> = {
  running: { icon: Loader2, tone: 'run' },
  completed: { icon: CheckCircle2, tone: 'ok' },
  failed: { icon: CircleAlert, tone: 'bad' },
  cancelled: { icon: CircleSlash, tone: 'idle' },
}
const runIcon = (run: any) => RUN_ICONS[run.phase] ?? { icon: Circle, tone: 'idle' }

const examples = [
  { key: 'chat' as const, flow: chatImageExample() },
  { key: 'website' as const, flow: websiteImageExample() },
].map(({ key, flow }) => ({ key, name: flow.name, kinds: flow.steps.map((s) => s.kind) }))

// ⌘S / Ctrl+S 保存；只在本页可见时监听（设置页用 KeepAlive 缓存，切走后会收到 deactivated）。
function onKeydown(event: KeyboardEvent) {
  if (event.defaultPrevented || event.altKey || event.shiftKey || !(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== 's') return
  event.preventDefault()
  if (confirmSwitch.value || deleteRequested.value) return
  void saveDraft()
}
const listen = () => window.addEventListener('keydown', onKeydown)
const unlisten = () => window.removeEventListener('keydown', onKeydown)
onActivated(listen)
onDeactivated(unlisten)
onBeforeUnmount(unlisten)

onMounted(async () => {
  listen()
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
    <WorkflowList :snapshot="snapshot" :selected-id="selectedID" :has-draft="!!draft" :sync-failure="syncFailure"
      @select="select" @create="create" @restore="restore" />

    <section v-if="draft" class="editor">
      <WorkflowEditorHeader v-model="draft" :can-duplicate="!dirty && !!original" :can-delete="!!original" :validate="validateShortcut"
        @duplicate="duplicate" @delete="deleteRequested = true" />

      <div v-if="stale || wf.catalog.value.conflicts.length" class="banner" role="status">
        <div v-if="stale" class="banner-row">
          <TriangleAlert :size="14" class="banner-icon" />
          <p>其他设备已更新或删除此工作流。当前草稿已保留；保存会创建新修订。</p>
          <button type="button" class="button" @click="load(selectedID)">放弃草稿并载入最新版本</button>
        </div>
        <WorkflowConflicts :conflicts="wf.catalog.value.conflicts" @restore="restore" />
      </div>

      <div class="scroll">
        <div class="inner">
          <WorkflowPipeline v-model="draft" />

          <SettingsSection v-if="visibleRuns.length" title="最近运行">
            <div v-for="run in visibleRuns" :key="run.id" class="run">
              <component :is="runIcon(run).icon" :size="14" class="run-icon" :class="[runIcon(run).tone, { loading: run.phase === 'running' }]" />
              <div class="run-copy">
                <strong>{{ runTitle(run) }}</strong>
                <p v-if="run.message">{{ run.message }}</p>
              </div>
              <button v-if="run.phase === 'running'" type="button" class="button" @click="wf.cancelRun(run.id)">取消</button>
            </div>
          </SettingsSection>
        </div>
      </div>

      <Transition name="bar">
        <div v-if="dirty || failure" class="savebar">
          <div class="msg">
            <span v-if="issues.length" class="warn">有 {{ issues.length }} 个步骤需要处理</span>
            <span v-else-if="dirty && unnamed" class="warn">请填写流程名称</span>
            <span v-else-if="dirty">有未保存的修改</span>
            <span v-if="failure" class="err" role="alert">{{ failure }}</span>
          </div>
          <template v-if="dirty">
            <button type="button" class="button" :disabled="saving" @click="undo">{{ isNewDraft ? '放弃' : '撤销修改' }}</button>
            <button type="button" class="button primary" :disabled="!canSave" @click="saveDraft">{{ saving ? '保存中…' : '保存流程' }}</button>
          </template>
          <button v-else type="button" class="button" @click="failure = null">知道了</button>
        </div>
      </Transition>
    </section>

    <section v-else class="intro">
      <Layers :size="40" class="intro-icon" />
      <div class="intro-copy">
        <h2>组合你的内容处理动作</h2>
        <p>整理文字、重命名文件、打包或处理图片，<br />也可在共享给其他设备时自动运行。</p>
      </div>
      <div class="starts">
        <button type="button" class="start primary" @click="create('blank')"><Plus :size="18" /><strong>新建流程</strong></button>
        <button v-for="example in examples" :key="example.key" type="button" class="start card" @click="create(example.key)">
          <WorkflowChain :kinds="example.kinds" :size="16" /><strong>{{ example.name }}</strong>
        </button>
      </div>
      <p v-if="failure" class="err" role="alert">{{ failure }}</p>
    </section>

    <ConfirmDialog :open="deleteRequested" title="删除这个工作流？" confirm-text="删除" destructive @confirm="removeSelected" @cancel="deleteRequested = false" />
    <ConfirmDialog :open="confirmSwitch" title="舍弃未保存的修改？" confirm-text="舍弃修改" destructive
      @confirm="() => { confirmSwitch = false; load(switchID) }" @cancel="confirmSwitch = false" />
  </div>
</template>

<style scoped>
.wf { display: flex; height: 100%; min-height: 0; padding-top: 8px; }
.editor { position: relative; display: flex; flex: 1; flex-direction: column; min-width: 360px; min-height: 0; }
.banner { display: grid; gap: 8px; padding: 10px 24px; border-bottom: 0.5px solid var(--line); background: color-mix(in srgb, var(--warning) 10%, var(--surface)); }
.banner-row { display: flex; align-items: center; gap: 8px; }
.banner-row p { flex: 1; min-width: 0; margin: 0; font-size: 12px; line-height: 1.5; }
.banner-icon { flex: none; color: var(--warning); }
.banner .button { min-height: 28px; flex: none; font-size: 12px; }
.scroll { flex: 1; min-height: 0; padding: 16px 24px 88px; overflow: auto; }
.inner { display: grid; gap: 24px; max-width: 680px; }

.run { display: flex; align-items: flex-start; gap: 10px; }
.run + .run { padding-top: 8px; border-top: 0.5px solid var(--line); }
.run-icon { flex: none; margin-top: 2px; }
.run-icon.ok { color: var(--success); }
.run-icon.bad { color: var(--danger); }
.run-icon.run { color: var(--accent); }
.run-icon.idle { color: var(--secondary); }
.run-copy { flex: 1; min-width: 0; }
.run-copy strong { font-size: 13px; font-weight: 500; }
.run-copy p { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.run .button { min-height: 28px; font-size: 12px; }

.savebar {
  position: absolute; right: 24px; bottom: 12px; left: 24px; z-index: 5; display: flex; align-items: center; gap: 8px;
  padding: 8px 8px 8px 16px; border-radius: 12px; background: var(--surface);
  box-shadow: inset 0 0 0 0.5px var(--line), 0 6px 20px rgba(0, 0, 0, 0.14);
}
.msg { display: grid; flex: 1; min-width: 0; gap: 2px; font-size: 12px; color: var(--secondary); }
.msg .warn { color: var(--warning); }
.msg .err { color: var(--danger); }
.savebar .button.primary:disabled { background: var(--control); color: var(--secondary); border-color: transparent; opacity: 1; }
.bar-enter-active, .bar-leave-active { transition: transform 150ms var(--ease-out), opacity 150ms var(--ease-out); }
.bar-enter-from, .bar-leave-to { transform: translateY(12px); opacity: 0; }

.intro {
  display: flex; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 16px;
  padding: 32px; background: var(--surface); text-align: center;
}
.intro-icon { color: var(--secondary); stroke-width: 1.2; }
.intro-copy { display: grid; gap: 8px; }
.intro h2 { margin: 0; font-size: 15px; font-weight: 700; }
.intro p { margin: 0; color: var(--secondary); font-size: 12px; line-height: 1.6; }
.starts { display: flex; flex-wrap: wrap; justify-content: center; gap: 12px; }
.start {
  display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; width: 136px; min-height: 84px; padding: 14px 10px;
  border-radius: 12px; font-size: 13px; transition: background-color var(--dur-fast) var(--ease-out), transform var(--dur-fast) var(--ease-out);
}
.start strong { font-weight: 600; }
.start.card { border: 0; box-shadow: inset 0 0 0 0.5px var(--line); }
.start.card:hover { background: var(--control); }
.start:active { transform: scale(0.98); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowDown, ArrowUp, ChevronDown, CircleMinus, GripVertical, TriangleAlert } from 'lucide-vue-next'
import WorkflowStepEditor from './WorkflowStepEditor.vue'
import { ACTION_ICONS, acceptRequirement, actionTitle, contentLabel, stepSummary, type WorkflowContent, type WorkflowStep } from './workflow'

const step = defineModel<WorkflowStep>({ required: true })
const props = defineProps<{
  index: number
  open: boolean
  /** 流入这一步的内容类型；null 表示兼容。 */
  rejected: WorkflowContent | null
  canMoveUp: boolean
  canMoveDown: boolean
  dragging: boolean
}>()
const emit = defineEmits<{ toggle: []; move: [offset: number]; remove: []; dragstart: [event: DragEvent]; dragend: [] }>()

const summary = computed(() => stepSummary(step.value))
const unbound = computed(() => step.value.kind === 'SystemShortcut' && step.value.local_shortcut_id === '')
const reason = computed(() => props.rejected ? `收到的是${contentLabel(props.rejected)}，此动作只处理${acceptRequirement(step.value.kind)}` : null)

// 只有按住拖拽手柄时卡片才可拖，避免参数区里选中文字时误触发拖拽。
const armed = ref(false)
function arm() {
  armed.value = true
  window.addEventListener('mouseup', () => { armed.value = false }, { once: true })
}
function onDragEnd() { armed.value = false; emit('dragend') }
</script>

<template>
  <div class="step" :class="{ open, bad: !!reason, dragging }" :draggable="armed" @dragstart="emit('dragstart', $event)" @dragend="onDragEnd">
    <div class="head" @click="emit('toggle')">
      <button type="button" class="main" :aria-expanded="open" :aria-controls="`step-${index}`" :title="open ? '收起参数' : '展开参数'">
        <component :is="ACTION_ICONS[step.kind]" :size="16" class="kind" />
        <span class="copy">
          <strong>{{ actionTitle(step.kind) }}</strong>
          <span v-if="summary" class="sum">{{ summary }}</span>
        </span>
        <TriangleAlert v-if="unbound" :size="13" class="warn" role="img" title="此设备尚未绑定快捷指令" aria-label="此设备尚未绑定快捷指令" />
        <ChevronDown :size="14" class="chev" aria-hidden="true" />
      </button>
      <div class="actions" @click.stop>
        <span class="grip" aria-hidden="true" title="拖动排序" @mousedown="arm"><GripVertical :size="14" /></span>
        <button type="button" class="icon-button" :disabled="!canMoveUp" title="上移" aria-label="上移" @click="emit('move', -1)"><ArrowUp :size="14" /></button>
        <button type="button" class="icon-button" :disabled="!canMoveDown" title="下移" aria-label="下移" @click="emit('move', 1)"><ArrowDown :size="14" /></button>
        <button type="button" class="icon-button remove" title="移除步骤" aria-label="移除步骤" @click="emit('remove')"><CircleMinus :size="14" /></button>
      </div>
    </div>
    <p v-if="reason" class="reason"><TriangleAlert :size="12" />{{ reason }}</p>
    <div v-if="open" :id="`step-${index}`" class="body"><WorkflowStepEditor v-model="step" /></div>
  </div>
</template>

<style scoped>
.step { min-width: 0; border-radius: 12px; background: var(--surface); box-shadow: inset 0 0 0 0.5px var(--line); transition: box-shadow var(--dur-fast) var(--ease-out), opacity var(--dur-fast) var(--ease-out); }
.step.bad { box-shadow: inset 0 0 0 1px var(--danger); }
.step.dragging { opacity: 0.45; }
.head { display: flex; align-items: center; gap: 4px; min-height: 48px; padding: 6px 8px 6px 4px; border-radius: 12px; cursor: pointer; }
.main { display: flex; flex: 1; align-items: center; gap: 10px; min-width: 0; padding: 4px 8px; border-radius: 8px; text-align: left; }
.main:focus-visible { outline-offset: -2px; }
.kind { flex: none; color: var(--secondary); }
.copy { display: grid; flex: 1; min-width: 0; }
.copy strong { font-size: 13px; font-weight: 600; line-height: 1.4; }
.sum { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; line-height: 1.4; color: var(--secondary); }
.warn { flex: none; color: var(--warning); }
.chev { flex: none; color: var(--secondary); transition: transform var(--dur-base) var(--ease-out); }
.open .chev { transform: rotate(180deg); }
.actions { display: flex; flex: none; align-items: center; gap: 2px; opacity: 0.35; transition: opacity var(--dur-fast) var(--ease-out); }
.step:hover .actions, .step:focus-within .actions { opacity: 1; }
.grip { display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 28px; color: var(--secondary); cursor: grab; }
.grip:active { cursor: grabbing; }
.actions .icon-button { border-color: transparent; background: transparent; }
.actions .icon-button:hover:not(:disabled) { background: var(--control); }
.remove { color: var(--danger); }
.reason { display: flex; align-items: center; gap: 6px; margin: 0; padding: 0 16px 10px 16px; font-size: 12px; line-height: 1.5; color: var(--danger); }
.reason svg { flex: none; }
.body { padding: 12px 16px 16px; border-top: 0.5px solid var(--line); }
</style>

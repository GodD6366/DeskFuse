<script setup lang="ts">
import { computed } from 'vue'
import { Copy, MoreHorizontal, Trash2, X } from 'lucide-vue-next'
import PopMenu from './PopMenu.vue'
import SegmentedControl from './SegmentedControl.vue'
import ShortcutRecorder from './ShortcutRecorder.vue'
import type { Key } from './util'
import { CONTENT_TITLES, type WorkflowContent, type WorkflowDefinition } from './workflow'

const flow = defineModel<WorkflowDefinition>({ required: true })
const props = defineProps<{ canDuplicate: boolean; canDelete: boolean; validate: (key: Key) => string | null }>()
const emit = defineEmits<{ duplicate: []; delete: [] }>()

const INPUTS = (Object.entries(CONTENT_TITLES) as [WorkflowContent, string][]).map(([value, label]) => ({ value, label }))
const menu = computed(() => [
  { key: 'duplicate', label: '复制流程', icon: Copy, disabled: !props.canDuplicate },
  { key: 'delete', label: '删除流程', icon: Trash2, disabled: !props.canDelete, danger: true },
])
const unnamed = computed(() => !flow.value.name.trim())
</script>

<template>
  <header class="head">
    <div class="row1">
      <input v-model="flow.name" class="title" :class="{ empty: unnamed }" placeholder="流程名称" aria-label="名称" />
      <label class="enable">启用
        <input v-model="flow.enabled" class="toggle" type="checkbox" role="switch" aria-label="启用流程" />
      </label>
      <PopMenu :items="menu" label="更多操作" :icon="MoreHorizontal" align="right" @select="$event === 'duplicate' ? emit('duplicate') : emit('delete')" />
    </div>
    <div class="row2">
      <span class="label">接受</span>
      <SegmentedControl v-model="flow.input_type" :options="INPUTS" label="接受内容" />
      <span class="spacer" />
      <span class="label">快捷键</span>
      <ShortcutRecorder variant="inline" :model-value="flow.shortcut" :validate="validate" @update:model-value="flow.shortcut = $event" />
      <button v-if="flow.shortcut" type="button" class="icon-button clear" title="移除快捷键" aria-label="移除快捷键" @click="flow.shortcut = null"><X :size="13" /></button>
    </div>
  </header>
</template>

<style scoped>
.head { display: grid; gap: 8px; padding: 4px 24px 12px; border-bottom: 0.5px solid var(--line); }
.row1, .row2 { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; min-width: 0; }
.row1 { flex-wrap: nowrap; gap: 12px; }
.title {
  flex: 1; min-width: 0; min-height: 32px; margin-left: -9px; padding: 0 8px; border: 1px solid transparent; border-radius: 8px;
  background: transparent; font-size: 15px; font-weight: 600; letter-spacing: -0.005em;
}
.title:hover:not(:focus) { background: var(--control); }
.title:focus { border-color: var(--line); background: var(--surface); }
.title.empty { border-color: var(--danger); }
.enable { display: inline-flex; align-items: center; gap: 8px; flex: none; font-size: 12px; color: var(--secondary); }
.label { font-size: 12px; color: var(--secondary); }
.spacer { flex: 1; }
.clear { width: 22px; height: 22px; min-height: 22px; padding: 0; border-color: transparent; background: transparent; color: var(--secondary); }
.clear:hover { background: var(--control); }
</style>

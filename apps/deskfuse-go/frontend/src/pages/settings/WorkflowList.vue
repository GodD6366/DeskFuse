<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { Plus, TriangleAlert } from 'lucide-vue-next'
import Keycaps from './Keycaps.vue'
import PopMenu from './PopMenu.vue'
import WorkflowChain from './WorkflowChain.vue'
import WorkflowConflicts from './WorkflowConflicts.vue'
import { useWorkflows } from './useWorkflows'
import { get, type Snapshot } from './util'
import type { WorkflowDefinition } from './workflow'

const props = defineProps<{ snapshot: Snapshot; selectedId: string | null; hasDraft: boolean; syncFailure: string | null }>()
const emit = defineEmits<{ select: [id: string]; create: [template: 'blank' | 'chat' | 'website']; restore: [id: string] }>()

const wf = useWorkflows()
const list = ref<HTMLElement | null>(null)

const MENU = [
  { key: 'blank', label: '新建空白流程' },
  { key: 'chat', label: '示例：聊天图片' },
  { key: 'website', label: '示例：网站配图' },
]

const syncTitle = (status: string) =>
  ({ synced: '同步完成', pending: '等待同步', waiting_for_devices: '等待已配对设备连接', error: '同步失败，已保留本地修改' } as Record<string, string>)[status] ?? status
const peerName = (id: string) =>
  (get<any[]>(props.snapshot, 'devices') ?? []).find((d) => d.id === id)?.name ?? id.slice(0, 8)

// 冲突在打开编辑器时显示在页头下方的横幅里；没有编辑器时才放在这里，保证始终能恢复。
const conflicts = computed(() => (props.hasDraft ? [] : wf.catalog.value.conflicts))
const showSync = computed(() => !!(wf.catalog.value.sync_status || wf.catalog.value.peers.length || conflicts.value.length || wf.catalog.value.error || props.syncFailure))

function warning(flow: WorkflowDefinition): string | null {
  const parts: string[] = []
  const issue = wf.catalog.value.issues.find((i) => i.workflow_id === flow.id)?.message
  if (issue) parts.push(issue)
  if (flow.steps.some((s) => s.kind === 'SystemShortcut' && !s.local_shortcut_id)) parts.push('本机快捷指令待绑定')
  return parts.length ? parts.join('\n') : null
}

// listbox：选中项是唯一的 tab 停靠点，↑/↓ 在列表内移动选择。
const stopId = computed(() => {
  const ids = wf.definitions.value.map((f) => f.id)
  return props.selectedId && ids.includes(props.selectedId) ? props.selectedId : ids[0]
})

async function onArrow(event: KeyboardEvent, index: number) {
  const next = wf.definitions.value[index + (event.key === 'ArrowDown' ? 1 : -1)]
  if (!next) return
  emit('select', next.id)
  await nextTick()
  list.value?.querySelector<HTMLElement>(`[data-id="${CSS.escape(next.id)}"]`)?.focus()
}
</script>

<template>
  <aside class="list">
    <div class="list-head">
      <strong>工作流</strong>
      <span class="count">{{ wf.definitions.value.length }}</span>
      <span class="spacer" />
      <PopMenu :items="MENU" label="新建流程或使用示例" :icon="Plus" align="right" @select="emit('create', $event as 'blank' | 'chat' | 'website')" />
    </div>

    <div v-if="showSync" class="sync">
      <p v-if="wf.catalog.value.sync_status" class="note">↻ {{ syncTitle(wf.catalog.value.sync_status) }}</p>
      <p v-for="peer in wf.catalog.value.peers" :key="peer.device_id" class="note">{{ peerName(peer.device_id) }}：{{ syncTitle(peer.status) }}</p>
      <WorkflowConflicts :conflicts="conflicts" @restore="emit('restore', $event)" />
      <p v-if="wf.catalog.value.error" class="err">{{ wf.catalog.value.error }}</p>
      <p v-if="syncFailure" class="err">{{ syncFailure }}</p>
    </div>

    <ul ref="list" class="items" role="listbox" aria-label="工作流">
      <li v-for="(flow, index) in wf.definitions.value" :key="flow.id" role="option" :data-id="flow.id" :aria-selected="flow.id === selectedId"
        :class="{ on: flow.id === selectedId, off: !flow.enabled }" :tabindex="flow.id === stopId ? 0 : -1"
        @click="emit('select', flow.id)" @keydown.enter.prevent="emit('select', flow.id)" @keydown.space.prevent="emit('select', flow.id)"
        @keydown.down.prevent="onArrow($event, index)" @keydown.up.prevent="onArrow($event, index)">
        <div class="top">
          <strong class="name">{{ flow.name }}</strong>
          <TriangleAlert v-if="warning(flow)" :size="13" class="warn" role="img" :title="warning(flow)!" :aria-label="warning(flow)!" />
          <Keycaps v-if="flow.shortcut" :shortcut="flow.shortcut" class="keys" />
        </div>
        <div class="sub">
          <WorkflowChain :kinds="flow.steps.map((s) => s.kind)" />
          <span v-if="!flow.enabled" class="note">已停用</span>
        </div>
      </li>
      <li v-if="!wf.definitions.value.length && !hasDraft" class="none">{{ wf.loadError.value ?? (wf.loading.value ? '正在读取…' : '尚未保存流程') }}</li>
    </ul>
  </aside>
</template>

<style scoped>
.list { display: flex; flex-direction: column; width: 240px; flex-shrink: 0; border-right: 0.5px solid var(--line); }
.list-head { display: flex; align-items: center; gap: 6px; padding: 8px 8px 8px 16px; }
.list-head strong { font-size: 13px; font-weight: 600; }
.count { font-size: 11px; color: var(--secondary); }
.spacer { flex: 1; }
.sync { display: grid; gap: 6px; max-height: 140px; padding: 0 16px 8px; overflow: auto; border-bottom: 0.5px solid var(--line); }
.items { flex: 1; margin: 0; padding: 4px 8px 8px; overflow: auto; list-style: none; }
.items li { display: grid; gap: 4px; padding: 8px; border-radius: 8px; cursor: pointer; }
.items li + li { margin-top: 2px; }
.items li:hover:not(.on) { background: color-mix(in srgb, var(--control) 60%, transparent); }
.items li.on { background: var(--control); }
.items li:focus-visible { outline-offset: -2px; }
.items li.off .top, .items li.off .sub :deep(.chain) { opacity: 0.55; }
.items li.none { color: var(--secondary); font-size: 13px; cursor: default; text-align: center; }
.top { display: flex; align-items: center; gap: 6px; min-width: 0; }
.name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 500; }
.warn { flex: none; color: var(--warning); }
.keys { flex: none; gap: 2px; }
.keys :deep(kbd) { min-width: 16px; min-height: 18px; padding: 0 4px; border-radius: 5px; font-size: 11px; }
.sub { display: flex; align-items: center; gap: 8px; min-width: 0; }
.note { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

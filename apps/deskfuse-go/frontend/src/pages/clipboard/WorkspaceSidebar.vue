<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import type { LibraryList } from '../../types/clipboard'
import ContextMenu from './ContextMenu.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import Icon from './Icon.vue'
import ListEditorDialog from './ListEditorDialog.vue'
import { loadWorkflows } from './actions'
import { isDefaultTypeGroup, listSymbol } from './format'
import { deleteList, errorMessage, library, newList, reorderLists } from './library'
import type { MenuItem } from './menu'

const selected = defineModel<string | null>('selectedListId', { default: null })
const emit = defineEmits<{ openWorkflows: []; error: [message: string] }>()

const editing = ref<LibraryList | null>(null)
const pendingDelete = ref<LibraryList | null>(null)
async function confirmDelete() {
  const list = pendingDelete.value
  pendingDelete.value = null
  if (!list) return
  try {
    await deleteList(list.id)
    if (selected.value === list.id) selected.value = null
  } catch (e) { emit('error', errorMessage(e)) }
}
const menu = ref<{ items: MenuItem[]; x: number; y: number } | null>(null)
const workflowCount = ref<number | null>(null)

const typeLists = computed(() => library.lists.filter(isDefaultTypeGroup))
const myLists = computed(() => library.lists.filter((l) => !isDefaultTypeGroup(l) && l.kind !== 'smart'))

onMounted(async () => {
  try { workflowCount.value = (await loadWorkflows()).length } catch { workflowCount.value = null }
})

const symbolIcon = (list: LibraryList) => {
  switch (listSymbol(list)) {
    case 'photo': return 'photo'
    case 'folder': return 'folder'
    case 'text': return 'text'
    case 'smart': return 'smart'
    default: return 'list'
  }
}

async function run(task: Promise<unknown>) {
  try { await task } catch (e) { emit('error', errorMessage(e)) }
}

function move(list: LibraryList, step: number) {
  const ids = library.lists.map((l) => l.id)
  const index = ids.indexOf(list.id)
  const target = index + step
  if (index < 0 || target < 0 || target >= ids.length) return
  ;[ids[index], ids[target]] = [ids[target], ids[index]]
  void run(reorderLists(ids))
}

function openListMenu(event: MouseEvent, list: LibraryList) {
  event.preventDefault()
  const first = library.lists[0]?.id === list.id
  const last = library.lists[library.lists.length - 1]?.id === list.id
  menu.value = {
    x: event.clientX,
    y: event.clientY,
    items: [
      ...(list.kind !== 'smart' ? [{ label: '重命名…', run: () => { editing.value = list } }] : []),
      { label: '向前移动', disabled: first, run: () => move(list, -1) },
      { label: '向后移动', disabled: last, run: () => move(list, 1) },
      { separator: true },
      {
        label: '删除列表', danger: true,
        run: () => { pendingDelete.value = list },
      },
    ],
  }
}

function openNewMenu(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  menu.value = { x: rect.left, y: rect.bottom + 2, items: [{ label: '新建列表…', run: () => { editing.value = newList() } }] }
}

// 当前选中的列表被删除（包括其他设备同步删除）后回到“全部”。
watch(() => library.lists, (lists) => {
  if (selected.value && !lists.some((l) => l.id === selected.value)) selected.value = null
})
</script>

<template>
  <aside class="sidebar">
    <h2>分组与列表</h2>

    <nav class="scroll" aria-label="分组与列表">
      <button type="button" class="tab" :class="{ on: selected === null }" :aria-current="selected === null || undefined" @click="selected = null">
        <span class="glyph"><Icon name="stack" :size="14" /></span>
        <span class="name">全部</span>
        <span v-if="library.counts['all'] !== undefined" class="count">{{ library.counts['all'] }}</span>
      </button>

      <div class="section">类型</div>
      <button
        v-for="list in typeLists" :key="list.id" type="button" class="tab"
        :class="{ on: selected === list.id, empty: library.counts[list.id] === 0 && selected !== list.id }"
        :aria-current="selected === list.id || undefined"
        @click="selected = list.id" @contextmenu="openListMenu($event, list)"
      >
        <span class="glyph"><Icon :name="symbolIcon(list)" :size="14" /></span>
        <span class="name">{{ list.name }}</span>
        <span v-if="library.counts[list.id] !== undefined" class="count">{{ library.counts[list.id] }}</span>
      </button>

      <hr />
      <div class="section">我的列表</div>
      <button
        v-for="list in myLists" :key="list.id" type="button" class="tab"
        :class="{ on: selected === list.id, empty: library.counts[list.id] === 0 && selected !== list.id }"
        :aria-current="selected === list.id || undefined"
        @click="selected = list.id" @contextmenu="openListMenu($event, list)"
      >
        <span class="glyph"><Icon :name="symbolIcon(list)" :size="14" /></span>
        <span class="name">{{ list.name }}</span>
        <span v-if="library.counts[list.id] !== undefined" class="count">{{ library.counts[list.id] }}</span>
      </button>

      <hr class="wide" />
      <button type="button" class="new" title="新建列表" @click="openNewMenu">
        <Icon name="plusCircle" :size="16" />
        <span>新建列表</span>
      </button>
    </nav>

    <button type="button" class="workflow" title="在设置中管理工作流" @click="emit('openWorkflows')">
      <span class="glyph"><Icon name="stack" :size="14" /></span>
      <span class="name">工作流</span>
      <span v-if="workflowCount !== null" class="count">{{ workflowCount }}</span>
      <Icon name="chevronRight" :size="11" class="chev" />
    </button>

    <ConfirmDialog v-if="pendingDelete" title="删除列表？" :message="`确定删除“${pendingDelete.name}”？此操作无法撤销。`" confirm-label="删除列表" @confirm="confirmDelete" @cancel="pendingDelete = null" />
    <ListEditorDialog
      v-if="editing" :initial="editing"
      @saved="selected = $event; editing = null" @close="editing = null"
    />
    <ContextMenu v-if="menu" :items="menu.items" :x="menu.x" :y="menu.y" @close="menu = null" />
  </aside>
</template>

<style scoped>
.sidebar { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--canvas, #f4f4f6); }
h2 { margin: 0; padding: 0 20px; height: 56px; display: flex; align-items: center; font-size: 15px; font-weight: 600; }
.scroll { flex: 1; min-height: 0; overflow-y: auto; padding: 8px 10px 0; display: flex; flex-direction: column; gap: 4px; scrollbar-width: none; }
.scroll::-webkit-scrollbar { display: none; }
.section { padding: 8px 8px 4px; font-size: 11px; font-weight: 500; color: var(--secondary, #595959); }
hr { border: 0; height: 1px; margin: 6px 0; background: var(--line, rgba(0, 0, 0, 0.09)); flex: none; }
hr.wide { margin: 8px 0; }
.tab {
  display: flex; align-items: center; gap: 9px; flex: none;
  height: 32px; padding: 0 8px; border: 0; border-radius: 8px; background: transparent;
  color: var(--text, #222); font: inherit; font-size: 13px; text-align: left; cursor: default;
  transition: background 0.16s ease;
}
.tab .glyph { width: 18px; flex: none; display: inline-flex; justify-content: center; color: inherit; }
.tab .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tab .count { font-size: 11px; font-variant-numeric: tabular-nums; color: var(--secondary, #595959); }
.tab.empty { color: var(--secondary, #595959); }
.tab.on { background: var(--surface, #fff); color: var(--text, #222); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.07); }
.tab.on .count { color: var(--secondary, #595959); }
.tab:focus-visible, .new:focus-visible, .workflow:focus-visible { outline: 2px solid var(--accent, #3276f3); outline-offset: -2px; }
.new {
  display: flex; align-items: center; gap: 10px; flex: none; min-height: 34px; padding: 0 10px; border: 0; border-radius: 8px;
  background: transparent; color: var(--accent, #3276f3); font: inherit; font-size: 12px; cursor: default;
}
.new:hover { background: color-mix(in srgb, var(--accent, #3276f3) 6%, transparent); }
.workflow {
  display: flex; align-items: center; gap: 9px; margin: 12px; height: 36px; padding: 0 10px;
  border: 0; border-radius: 8px; background: transparent; color: var(--text, #222); font: inherit; font-size: 13px; cursor: default;
}
.workflow .glyph { width: 18px; flex: none; display: inline-flex; justify-content: center; color: var(--secondary, #595959); }
.workflow .name { flex: 1; text-align: left; }
.workflow .count { font-size: 11px; color: var(--secondary, #595959); }
.workflow .chev { color: var(--secondary, #595959); opacity: 0.6; }
</style>

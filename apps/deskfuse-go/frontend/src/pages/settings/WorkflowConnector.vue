<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { Plus } from 'lucide-vue-next'
import WorkflowActionPicker from './WorkflowActionPicker.vue'
import { useDismiss } from './useDismiss'
import { contentLabel, type WorkflowAction, type WorkflowContent } from './workflow'

// 两个节点之间的轨道连线：标出流过的内容类型，并在此处插入动作。
// 弹层的根包含触发按钮，所以同一时间只会有一个选择器打开（点别处即关闭）。
const props = defineProps<{
  content: WorkflowContent
  /** 下方步骤不接受这段内容。 */
  bad: boolean
  /** 终点前的连线：常驻“添加动作”按钮，而不是 hover 才出现的 +。 */
  persistent?: boolean
  full: boolean
  /** 拖拽排序时的插入位置指示线。 */
  drop: boolean
}>()
const emit = defineEmits<{ pick: [kind: WorkflowAction] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
useDismiss(open, root, () => { open.value = false })

async function toggle() {
  open.value = !open.value
  if (!open.value) return
  await nextTick()
  const picker = root.value?.querySelector<HTMLElement>('.picker')
  picker?.querySelector<HTMLElement>('[role=menuitem]:not(:disabled)')?.focus()
  picker?.scrollIntoView({ block: 'nearest' })
}

function pick(kind: WorkflowAction) {
  open.value = false
  emit('pick', kind)
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  event.preventDefault()
  const nodes = [...root.value!.querySelectorAll<HTMLElement>('.picker [role=menuitem]:not(:disabled)')]
  const at = nodes.indexOf(document.activeElement as HTMLElement)
  nodes[(at + (event.key === 'ArrowDown' ? 1 : -1) + nodes.length) % nodes.length]?.focus()
}
</script>

<template>
  <div ref="root" class="conn" :class="{ bad, drop, persistent, open }" @keydown="onKey">
    <div class="rail">
      <button v-if="!persistent" type="button" class="plus" :disabled="full" title="在此处插入动作" aria-label="在此处插入动作" aria-haspopup="menu" :aria-expanded="open" @click="toggle">
        <Plus :size="12" />
      </button>
    </div>
    <div class="side">
      <span class="flow">{{ contentLabel(content) }}</span>
      <button v-if="persistent" type="button" class="button add" :disabled="full" :title="full ? '最多 32 个步骤' : undefined" aria-haspopup="menu" :aria-expanded="open" @click="toggle">
        <Plus :size="13" />添加动作
      </button>
    </div>
    <WorkflowActionPicker v-if="open" :content="content" @pick="pick" />
  </div>
</template>

<style scoped>
.conn { position: relative; display: grid; grid-template-columns: 28px minmax(0, 1fr); gap: 8px; min-height: 32px; }
.conn.persistent { min-height: 44px; }
.rail { position: relative; display: flex; align-items: center; justify-content: center; }
.rail::before { content: ''; position: absolute; top: 0; bottom: 0; left: 50%; width: 1px; transform: translateX(-0.5px); background: var(--rail, color-mix(in srgb, var(--text) 18%, transparent)); }
.conn.bad .rail::before { background: var(--danger); }
.plus {
  position: relative; display: inline-flex; align-items: center; justify-content: center; width: 18px; height: 18px; padding: 0; border-radius: 50%;
  background: var(--surface); color: var(--accent); box-shadow: inset 0 0 0 0.5px var(--accent); opacity: 0;
  transition: opacity var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out);
}
.conn:hover .plus, .plus:focus-visible, .conn.open .plus { opacity: 1; }
.plus:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 10%, var(--surface)); }
.side { display: flex; align-items: center; gap: 12px; min-width: 0; }
.flow { font-size: 11px; color: var(--secondary); }
.conn.bad .flow { color: var(--danger); }
.add { min-height: 28px; font-size: 12px; }
.conn.drop::after { content: ''; position: absolute; left: 36px; right: 0; top: 50%; height: 2px; transform: translateY(-1px); border-radius: 1px; background: var(--accent); pointer-events: none; }
</style>

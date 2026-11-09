<script setup lang="ts">
import { computed } from 'vue'
import { ACTION_GROUPS, ACTION_ICONS, acceptRequirement, actionTitle, stepAccepts, type WorkflowAction, type WorkflowContent } from './workflow'

// 选择要插入的动作；与流入内容不兼容的动作置灰并写明原因。
const props = defineProps<{ content: WorkflowContent }>()
const emit = defineEmits<{ pick: [kind: WorkflowAction] }>()

const groups = computed(() =>
  ACTION_GROUPS.map((group) => ({
    title: group.title,
    items: group.kinds.map((kind) => ({ kind, ok: stepAccepts(kind, props.content), need: acceptRequirement(kind) })),
  })),
)
</script>

<template>
  <div class="picker card" role="menu" aria-label="选择动作">
    <section v-for="group in groups" :key="group.title" class="group">
      <h4>{{ group.title }}</h4>
      <button v-for="item in group.items" :key="item.kind" type="button" role="menuitem" class="item" :disabled="!item.ok" @click="emit('pick', item.kind)">
        <component :is="ACTION_ICONS[item.kind]" :size="14" class="ico" />
        <span class="t">{{ actionTitle(item.kind) }}</span>
        <small v-if="!item.ok">需要{{ item.need }}</small>
      </button>
    </section>
  </div>
</template>

<style scoped>
.picker {
  position: absolute; top: calc(100% - 4px); left: 36px; z-index: 30; display: grid; gap: 4px; width: 260px; padding: 6px;
  border: 0; box-shadow: inset 0 0 0 0.5px var(--line), 0 8px 24px rgba(0, 0, 0, 0.14);
}
.group { display: grid; }
h4 { margin: 0; padding: 6px 8px 2px; font-size: 11px; font-weight: 600; color: var(--secondary); }
.item { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; text-align: left; font-size: 13px; }
.item:hover:not(:disabled), .item:focus-visible { background: var(--control); }
.item:focus-visible { outline-offset: -2px; }
.item:disabled { opacity: 1; cursor: default; }
.item:disabled .ico, .item:disabled .t { opacity: 0.45; }
.ico { flex: none; color: var(--secondary); }
.t { flex: 1; min-width: 0; }
.item small { flex: none; }
</style>

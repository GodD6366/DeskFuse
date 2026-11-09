<script setup lang="ts">
import { computed } from 'vue'
import { ACTION_ICONS, type WorkflowAction } from './workflow'

// 步骤图标链：最多显示 max 个，其余折叠为 +N。
const props = withDefaults(defineProps<{ kinds: WorkflowAction[]; max?: number; size?: number }>(), { max: 5, size: 12 })
const shown = computed(() => props.kinds.slice(0, props.max))
const extra = computed(() => Math.max(0, props.kinds.length - props.max))
</script>

<template>
  <span class="chain" aria-hidden="true">
    <template v-for="(kind, index) in shown" :key="index">
      <span v-if="index > 0" class="sep">›</span>
      <component :is="ACTION_ICONS[kind]" :size="size" />
    </template>
    <span v-if="extra" class="extra">+{{ extra }}</span>
  </span>
</template>

<style scoped>
.chain { display: inline-flex; align-items: center; gap: 3px; min-width: 0; color: var(--secondary); line-height: 1; }
.chain > :deep(svg) { flex: none; }
.sep { font-size: 11px; opacity: 0.6; }
.extra { font-size: 11px; }
</style>

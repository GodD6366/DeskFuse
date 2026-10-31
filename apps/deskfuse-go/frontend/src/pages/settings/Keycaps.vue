<script setup lang="ts">
import { computed } from 'vue'
import { shortcutParts, type Key } from './util'

const props = defineProps<{ shortcut: Key }>()
const parts = computed(() => shortcutParts(props.shortcut))
</script>

<template>
  <span class="keycaps" :aria-label="parts.join(' ')">
    <kbd v-for="(part, index) in parts" :key="index">{{ part }}</kbd>
  </span>
</template>

<style scoped>
.keycaps { display: inline-flex; gap: 4px; }
kbd { display: inline-grid; place-items: center; min-width: 18px; min-height: 24px; padding: 0 5px; border: 1px solid color-mix(in srgb, var(--text) 12%, transparent); border-radius: 8px; background: color-mix(in srgb, var(--text) 4.5%, transparent); font-family: ui-rounded, 'SF Pro Rounded', -apple-system, sans-serif; font-size: 13px; font-weight: 500; }
</style>

<script setup lang="ts">
import { nextTick, ref, type Component } from 'vue'
import { useDismiss } from './useDismiss'

export interface PopMenuItem { key: string; label: string; icon?: Component; disabled?: boolean; danger?: boolean }

defineProps<{ items: PopMenuItem[]; label: string; icon: Component; align?: 'left' | 'right' }>()
const emit = defineEmits<{ select: [key: string] }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
useDismiss(open, root, () => { open.value = false })

async function toggle() {
  open.value = !open.value
  if (!open.value) return
  await nextTick()
  root.value?.querySelector<HTMLElement>('[role=menuitem]:not(:disabled)')?.focus()
}

function choose(key: string) {
  open.value = false
  emit('select', key)
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  event.preventDefault()
  const nodes = [...root.value!.querySelectorAll<HTMLElement>('[role=menuitem]:not(:disabled)')]
  const at = nodes.indexOf(document.activeElement as HTMLElement)
  nodes[(at + (event.key === 'ArrowDown' ? 1 : -1) + nodes.length) % nodes.length]?.focus()
}
</script>

<template>
  <div ref="root" class="menu">
    <button type="button" class="icon-button" :title="label" :aria-label="label" aria-haspopup="menu" :aria-expanded="open" @click="toggle">
      <component :is="icon" :size="15" />
    </button>
    <div v-if="open" class="pop card" :class="align ?? 'left'" role="menu" @keydown="onKey">
      <button v-for="item in items" :key="item.key" type="button" role="menuitem" :disabled="item.disabled" :class="{ danger: item.danger }" @click="choose(item.key)">
        <component :is="item.icon" v-if="item.icon" :size="14" />
        {{ item.label }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.menu { position: relative; flex: none; }
.pop {
  position: absolute; top: calc(100% + 4px); z-index: 20; display: grid; min-width: 160px; padding: 4px;
  border: 0; box-shadow: inset 0 0 0 0.5px var(--line), 0 8px 24px rgba(0, 0, 0, 0.14);
}
.pop.left { left: 0; }
.pop.right { right: 0; }
.pop button { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; text-align: left; font-size: 13px; white-space: nowrap; }
.pop button:hover:not(:disabled), .pop button:focus-visible { background: var(--control); }
.pop button:focus-visible { outline-offset: -2px; }
</style>

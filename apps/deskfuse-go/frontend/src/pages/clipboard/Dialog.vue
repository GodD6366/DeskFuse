<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(defineProps<{ width?: number; label?: string; dismissible?: boolean }>(), {
  width: 360, label: undefined, dismissible: true,
})
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)
let previous: Element | null = null

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape' && props.dismissible) { event.stopPropagation(); emit('close') }
  if (event.key === 'Tab' && panel.value) {
    const focusable = panel.value.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [contenteditable="true"]',
    )
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus() }
  }
}

onMounted(() => {
  previous = document.activeElement
  window.addEventListener('keydown', onKey, true)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey, true)
  if (previous instanceof HTMLElement) previous.focus()
})
</script>

<template>
  <Teleport to="body">
    <div class="scrim" @mousedown.self="dismissible && emit('close')">
      <div ref="panel" class="sheet" role="dialog" aria-modal="true" :aria-label="label" :style="{ width: width + 'px' }">
        <slot />
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 900;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 12vh;
  background: rgba(0, 0, 0, 0.28);
}
.sheet {
  max-width: calc(100vw - 32px);
  max-height: 80vh;
  overflow: auto;
  border-radius: 16px;
  background: var(--surface, #fff);
  color: var(--text, #1d1d1f);
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.12));
  box-shadow: 0 24px 60px rgba(0, 0, 0, 0.28);
}
</style>

<script setup lang="ts">
import { onBeforeUnmount, watch } from 'vue'

const props = defineProps<{
  open: boolean
  title: string
  message?: string
  confirmText?: string
  cancelText?: string
  destructive?: boolean
  busy?: boolean
  error?: string | null
  confirmDisabled?: boolean
  width?: number
}>()
const emit = defineEmits<{ confirm: []; cancel: [] }>()

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape' && !props.busy) emit('cancel')
}
watch(
  () => props.open,
  (open) => {
    if (open) window.addEventListener('keydown', onKey)
    else window.removeEventListener('keydown', onKey)
  },
  { immediate: true },
)
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="scrim" @mousedown.self="!busy && emit('cancel')">
      <div class="dialog card" role="dialog" aria-modal="true" :aria-label="title" :style="{ width: (width ?? 420) + 'px' }">
        <h2>{{ title }}</h2>
        <p v-if="message" class="msg">{{ message }}</p>
        <slot />
        <p v-if="error" class="err" role="alert">{{ error }}</p>
        <div class="actions">
          <button type="button" :disabled="busy" @click="emit('cancel')">{{ cancelText ?? '取消' }}</button>
          <button
            type="button"
            class="primary"
            :class="{ danger: destructive }"
            :disabled="busy || confirmDisabled"
            @click="emit('confirm')"
          >
            {{ confirmText ?? '确定' }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.scrim { position: fixed; inset: 0; z-index: 1000; display: grid; place-items: center; background: rgba(0, 0, 0, 0.32); }
.dialog { display: grid; gap: 14px; max-width: calc(100vw - 48px); max-height: calc(100vh - 48px); overflow: auto; padding: 22px; background: var(--surface); }
h2 { margin: 0; font-size: 15px; font-weight: 600; }
.msg { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.err { margin: 0; font-size: 11px; color: var(--danger); }
.actions { display: flex; justify-content: flex-end; gap: 8px; }
.actions button { min-height: 28px; padding: 0 12px; border-radius: 6px; border: 1px solid var(--line); background: var(--surface); }
.actions button:hover { background: var(--control); }
.actions button.primary { background: var(--accent); color: #fff; border-color: transparent; }
.actions button.primary:hover { filter: brightness(1.04); }
.danger { background: var(--danger); border-color: var(--danger); color: #fff; }
</style>

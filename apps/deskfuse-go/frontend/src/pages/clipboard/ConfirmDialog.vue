<script setup lang="ts">
import Dialog from './Dialog.vue'

defineProps<{ title: string; message: string; confirmLabel: string }>()
defineEmits<{ confirm: []; cancel: [] }>()
</script>

<template>
  <Dialog :width="320" :label="title" @close="$emit('cancel')">
    <div class="body">
      <h2>{{ title }}</h2>
      <p>{{ message }}</p>
      <div class="actions">
        <button type="button" class="destructive" @click="$emit('confirm')">{{ confirmLabel }}</button>
        <button type="button" autofocus @click="$emit('cancel')">取消</button>
      </div>
    </div>
  </Dialog>
</template>

<style scoped>
.body { padding: 20px; display: flex; flex-direction: column; gap: 10px; }
h2 { margin: 0; font-size: 15px; font-weight: 600; }
p { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary, #6e6e73); }
.actions { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
button {
  height: 32px;
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.12));
  border-radius: 8px;
  background: var(--control, rgba(0, 0, 0, 0.04));
  color: var(--text, #1d1d1f);
  font: inherit;
  font-size: 13px;
  cursor: default;
}
button.destructive { color: var(--danger, #ff3b30); }
button:focus-visible { outline: 2px solid var(--accent, #3276f3); outline-offset: 1px; }
</style>

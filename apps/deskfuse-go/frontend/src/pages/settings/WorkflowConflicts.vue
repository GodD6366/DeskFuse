<script setup lang="ts">
defineProps<{ conflicts: { id: string; name: string; message: string }[] }>()
defineEmits<{ restore: [id: string] }>()
</script>

<template>
  <div v-for="conflict in conflicts" :key="conflict.id" class="conflict">
    <div class="copy"><strong>{{ conflict.name }}</strong><p>{{ conflict.message }}</p></div>
    <button type="button" class="button" @click="$emit('restore', conflict.id)">恢复保留版本</button>
  </div>
</template>

<style scoped>
.conflict { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.copy { display: grid; gap: 2px; min-width: 0; }
strong { font-size: 12px; font-weight: 600; }
p { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.button { min-height: 28px; flex: none; font-size: 12px; }
</style>

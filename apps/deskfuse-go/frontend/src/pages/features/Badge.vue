<script setup lang="ts">
withDefaults(defineProps<{ text?: string; tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'beta' }>(), {
  text: 'Beta',
  tone: 'beta',
})
</script>

<template>
  <span class="badge" :class="`tone-${tone}`">{{ text }}</span>
</template>

<style scoped>
.badge {
  --tone: var(--warning, #ff9500);
  display: inline-block;
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  line-height: 16px;
  color: var(--tone);
  background: color-mix(in srgb, var(--tone) 12%, transparent);
}
.tone-neutral { --tone: var(--secondary); }
.tone-accent { --tone: var(--accent); }
.tone-success { --tone: var(--success, #34c759); }
.tone-danger { --tone: var(--danger, #ff3b30); }
</style>

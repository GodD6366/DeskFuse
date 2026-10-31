<script setup lang="ts">
import HelpFeedback from './settings/HelpFeedback.vue'
defineProps<{snapshot:Record<string,any>|null}>()
</script>
<template><div class="support-page"><h1>帮助与反馈</h1><HelpFeedback :snapshot="snapshot"/></div></template>
<style scoped>.support-page{padding:24px;max-width:768px;width:100%;margin:0 auto;overflow:auto}.support-page h1{margin-bottom:24px}</style>

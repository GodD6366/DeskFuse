<script setup lang="ts">
import type { Component } from 'vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { useAction, type Snapshot } from './util'

defineProps<{ snapshot: Snapshot; symbol?: Component }>()

// 源码、发行版、问题反馈都在同一个公开仓库，与 config/distribution.json 的 source_repository 一致。
const REPOSITORY = 'GodD6366/DeskFuse'
const issuesURL = `https://github.com/${REPOSITORY}/issues`
const releasesURL = `https://github.com/${REPOSITORY}/releases`

const report = useAction()
const exporting = useAction()
const link = useAction()
</script>

<template>
  <SettingsSection
    title="帮助与反馈"
    :symbol="symbol"
    footer="诊断文件只包含运行日志和设备状态，不包含剪贴板内容。随时可以在菜单「帮助」或侧栏底部的问号找到这些入口。">
    <SettingsRow title="反馈问题" detail="自动导出诊断并打开反馈页，版本和系统信息已经填好，只需描述发生了什么。">
      <button type="button" class="button primary" :disabled="report.pending.value" @click="report.run('support.report', {})">
        {{ report.pending.value ? '正在准备…' : '反馈问题…' }}
      </button>
      <template #extra><p v-if="report.error.value" class="err" role="alert">{{ report.error.value }}</p></template>
    </SettingsRow>
    <SettingsRow title="导出诊断信息" detail="保存到指定位置，方便通过其他渠道发给开发者。">
      <button type="button" class="button" :disabled="exporting.pending.value || !snapshot" @click="exporting.run('support.export_diagnostics', {})">
        {{ exporting.pending.value ? '正在导出…' : '导出…' }}
      </button>
      <template #extra><p v-if="exporting.error.value" class="err" role="alert">{{ exporting.error.value }}</p></template>
    </SettingsRow>
    <SettingsRow title="已知问题与发行说明" detail="先看看别人是否遇到过同样的问题，或新版本是否已修复。">
      <button type="button" class="button" @click="link.run('system.open_url', { url: issuesURL })">已报告的问题</button>
      <button type="button" class="button" @click="link.run('system.open_url', { url: releasesURL })">发行说明</button>
      <template #extra><p v-if="link.error.value" class="err" role="alert">{{ link.error.value }}</p></template>
    </SettingsRow>
  </SettingsSection>
</template>

<style scoped>
.err { margin: 0; font-size: 11px; color: var(--danger); }
</style>

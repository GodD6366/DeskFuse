<script setup lang="ts">
import { computed, ref } from 'vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import UninstallDialog from './UninstallDialog.vue'
import { get, preference, useAction, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()

const CHANNELS: Record<string, string> = {
  github: 'GitHub 发行版', manual: '手动安装', lan: '局域网内部版', development: '开发版',
}

const channel = computed(() => (preference(props.snapshot, 'channel') ?? get(get(props.snapshot, 'update'), 'channel')) as string | undefined)
const isInternal = computed(() => channel.value === 'lan' || channel.value === 'development')
const build = computed(() => get(get(props.snapshot, 'update'), 'build'))

const restart = useAction()
const logs = useAction()
const uninstalling = ref(false)
</script>

<template>
  <div class="page">
    <SettingsSection title="版本信息">
      <SettingsRow compact subdued title="更新通道"><span>{{ channel ? (CHANNELS[channel] ?? channel) : '未知' }}</span></SettingsRow>
      <SettingsRow v-if="isInternal" compact subdued title="构建号"><span>{{ build ?? '未知' }}</span></SettingsRow>
    </SettingsSection>

    <!-- 诊断只在需要时展开，默认不占版面。 -->
    <SettingsSection title="诊断" footer="导出诊断或反馈问题，请前往「关于与更新」。重走初次设置会沿用当前的功能开关与权限，不会清除任何数据。">
      <details class="disclosure">
        <summary><span>维护与诊断</span></summary>
        <div class="rows">
          <SettingsRow title="重走初次设置" detail="重新显示权限说明与功能选择引导。">
            <button type="button" class="button" :disabled="restart.pending.value" @click="restart.run('onboarding.restart', {})">开始</button>
            <template #extra><p v-if="restart.error.value" class="err" role="alert">{{ restart.error.value }}</p></template>
          </SettingsRow>
          <SettingsRow title="日志文件夹" detail="~/Library/Logs/DeskFuse">
            <button type="button" class="button" :disabled="logs.pending.value" @click="logs.run('support.reveal_logs', {})">在访达中显示</button>
            <template #extra><p v-if="logs.error.value" class="err" role="alert">{{ logs.error.value }}</p></template>
          </SettingsRow>
        </div>
      </details>
    </SettingsSection>

    <SettingsSection title="卸载" footer="可选择删除剪贴板历史与本机数据、缓存、偏好设置、钥匙串账号、登录项、系统权限记录与麦克风驱动，并把应用移到废纸篓。剪贴板历史默认保留，执行前会再次确认。">
      <SettingsRow title="彻底卸载 DeskFuse" detail="删除所选内容后退出，无法恢复。">
        <button type="button" class="button destructive" @click="uninstalling = true">卸载…</button>
      </SettingsRow>
    </SettingsSection>

    <UninstallDialog :open="uninstalling" @close="uninstalling = false" />
  </div>
</template>

<style scoped>
.page { display: grid; align-content: start; gap: 24px; }
.err { margin: 0; font-size: 11px; color: var(--danger); }
.disclosure summary {
  display: flex; align-items: center; gap: 8px; min-height: 28px;
  font-size: 13px; color: var(--text); cursor: pointer; list-style: none;
}
.disclosure summary::-webkit-details-marker { display: none; }
.disclosure summary > span::before { content: '›'; display: inline-block; width: 12px; color: var(--secondary); transition: transform 0.15s; }
.disclosure[open] summary > span::before { transform: rotate(90deg); }
.rows { display: grid; gap: 8px; padding-top: 8px; }
.rows > :deep(.s-row + .s-row) { padding-top: 10px; border-top: 0.5px solid var(--line); }
</style>

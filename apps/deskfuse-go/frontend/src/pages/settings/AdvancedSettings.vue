<script setup lang="ts">
import { computed, ref } from 'vue'
import AsyncToggle from './AsyncToggle.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import UninstallDialog from './UninstallDialog.vue'
import { backend } from '../../services/backend'
import { errorText, get, preference, useAction, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()
const emit = defineEmits<{ navigate: [target: string] }>()

const CHANNELS: Record<string, string> = {
  github: 'GitHub 发行版', manual: '手动安装', lan: '局域网内部版', development: '开发版',
}

const channel = computed(() => (preference(props.snapshot, 'channel') ?? get(get(props.snapshot, 'update'), 'channel')) as string | undefined)
const isInternal = computed(() => channel.value === 'lan' || channel.value === 'development')
const build = computed(() => get(get(props.snapshot, 'update'), 'build'))
const remoteEnabled = computed(() => get<boolean>(get(props.snapshot, 'features'), 'remote_applications'))

const clearHistory = useAction()
const restart = useAction()
const logs = useAction()
const confirmClear = ref(false)
const uninstalling = ref(false)

async function doClear() {
  confirmClear.value = false
  clearHistory.pending.value = true
  clearHistory.error.value = null
  try {
    await backend.mutateLibrary({ op: 'clear_history', confirm: true })
    clearHistory.status.value = '已清空'
  } catch (e) { clearHistory.error.value = errorText(e) }
  finally { clearHistory.pending.value = false }
}
</script>

<template>
  <div class="page">
    <SettingsSection title="功能" footer="进阶功能默认关闭。关闭工作流后所有入口隐藏，已注册的全局快捷键也会停用；关闭 Windows 远程应用会断开已打开的 Windows 应用窗口。">
      <SettingsRow title="启用工作流" detail="在剪贴板里串联多个动作并绑定全局快捷键。">
        <AsyncToggle :value="preference(snapshot, 'workflows_enabled') === true" action="settings.set"
          :params="(value) => ({ key: 'workflows_enabled', value })" label="启用工作流" :disabled="!snapshot" />
      </SettingsRow>
      <SettingsRow v-if="remoteEnabled !== undefined" title="启用 Windows 远程应用" badge="Beta"
        detail="在远程应用中打开已配对 Windows 电脑上的应用。功能仍在完善中。">
        <AsyncToggle :value="remoteEnabled === true" action="feature.setEnabled"
          :params="(enabled) => ({ feature: 'windows_remote_applications', enabled })" label="启用 Windows 远程应用" />
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title="剪贴板">
      <SettingsRow title="跨设备索引与排除 App" detail="同步标题与识别文字、管理不保存历史的应用。">
        <button type="button" class="button" @click="emit('navigate', 'clipboard')">打开剪贴板设置</button>
      </SettingsRow>
      <SettingsRow title="清空剪贴板历史" detail="删除本机保存的全部剪贴板历史。">
        <button type="button" class="button destructive" :disabled="clearHistory.pending.value" @click="confirmClear = true">清空…</button>
        <template #extra>
          <p v-if="clearHistory.error.value" class="err" role="alert">{{ clearHistory.error.value }}</p>
          <p v-else-if="clearHistory.status.value" class="note">{{ clearHistory.status.value }}</p>
        </template>
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title="版本信息" footer="导出诊断或反馈问题，请前往「关于与更新」。">
      <SettingsRow compact subdued title="更新通道"><span>{{ channel ? (CHANNELS[channel] ?? channel) : '未知' }}</span></SettingsRow>
      <SettingsRow v-if="isInternal" compact subdued title="构建号"><span>{{ build ?? '未知' }}</span></SettingsRow>
    </SettingsSection>

    <SettingsSection title="调试" footer="重走初次设置会沿用当前的功能开关与权限，不会清除任何数据。">
      <SettingsRow title="重走初次设置" detail="重新显示权限说明与功能选择引导。">
        <button type="button" class="button" :disabled="restart.pending.value" @click="restart.run('onboarding.restart', {})">开始</button>
        <template #extra><p v-if="restart.error.value" class="err" role="alert">{{ restart.error.value }}</p></template>
      </SettingsRow>
      <SettingsRow title="日志文件夹" detail="~/Library/Logs/DeskFuse">
        <button type="button" class="button" :disabled="logs.pending.value" @click="logs.run('support.reveal_logs', {})">在访达中显示</button>
        <template #extra><p v-if="logs.error.value" class="err" role="alert">{{ logs.error.value }}</p></template>
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title="卸载" footer="可选择删除剪贴板历史与本机数据、缓存、偏好设置、钥匙串账号、登录项、系统权限记录与麦克风驱动，并把应用移到废纸篓。剪贴板历史默认保留，执行前会再次确认。">
      <SettingsRow title="彻底卸载 DeskFuse" detail="删除所选内容后退出，无法恢复。">
        <button type="button" class="button destructive" @click="uninstalling = true">卸载…</button>
      </SettingsRow>
    </SettingsSection>

    <ConfirmDialog :open="confirmClear" title="清空所有剪贴板历史？" message="手动列表成员也会被清空。剪贴板内容不会重新同步回来。"
      confirm-text="清空历史" destructive @confirm="doClear" @cancel="confirmClear = false" />
    <UninstallDialog :open="uninstalling" @close="uninstalling = false" />
  </div>
</template>

<style scoped>
.page { display: grid; align-content: start; gap: 24px; }
.note { margin: 0; font-size: 12px; color: var(--secondary); }
.err { margin: 0; font-size: 11px; color: var(--danger); }
</style>

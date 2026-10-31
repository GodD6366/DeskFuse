<script setup lang="ts">
import { computed } from 'vue'
import { ArrowDownCircle, LifeBuoy } from 'lucide-vue-next'
import HelpFeedback from './HelpFeedback.vue'
import SettingsSection from './SettingsSection.vue'
import { formatBytes, get, preference, useAction, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()

const update = computed(() => get<any>(props.snapshot, 'update'))
const channel = computed(() => (get<string>(update.value, 'channel') ?? preference(props.snapshot, 'channel')) as string | undefined)
const isManual = computed(() => channel.value === 'manual')
const check = useAction()
const install = useAction()
const link = useAction()

const u = (name: string) => get<any>(update.value, name)
const manualRequested = computed(() => u('manual_requested') === true)
const checking = computed(() => check.pending.value || u('checking') === true || preference(props.snapshot, 'checking_updates') === true)
const hasTarget = computed(() => !!u('target_version') || u('ready') === true || u('downloading') === true)

const actionTitle = computed(() => {
  if (manualRequested.value) return u('ready') ? '正在安装…' : '正在下载…'
  if (u('ready')) return '安装并重新打开'
  if (u('downloading')) return '下载完成后安装'
  return '下载并安装'
})

const statusText = computed(() => {
  if (isManual.value) return '此版本需要手动下载新版安装包。'
  if (u('target_version')) return `发现新版本 ${u('target_version')}。`
  if (checking.value) return '正在检查更新…'
  return u('message') || '已是最新版本。'
})

const versionLabel = computed(() => (u('version') || preference(props.snapshot, 'version_label') || '') as string)
</script>

<template>
  <div class="page">
    <header class="brand">
      <div class="brand-mark" role="img" aria-label="DeskFuse" />
      <div class="brand-text">
        <h2>DeskFuse</h2>
        <p class="note">一套键鼠、一个剪贴板，在多台电脑之间无缝工作。</p>
      </div>
    </header>

    <SettingsSection title="更新" :symbol="ArrowDownCircle" :footer="isManual ? undefined : 'DeskFuse 每 12 小时自动检查一次更新。'">
      <div v-if="update" class="update">
        <div class="line">
          <div class="text">
            <div class="title">DeskFuse {{ versionLabel }}</div>
            <p class="note">{{ statusText }}</p>
          </div>
          <div v-if="!isManual" class="actions">
            <button v-if="hasTarget" type="button" class="button primary" :disabled="!u('available') || manualRequested || install.pending.value"
              @click="install.run('download_and_install_update', {})">
              {{ actionTitle }}
            </button>
            <button type="button" class="button" :disabled="checking || u('ready') === true || manualRequested" @click="check.run('updates.check', {})">
              {{ checking ? '正在检查…' : '检查更新' }}
            </button>
          </div>
        </div>
        <template v-if="u('downloading') && Number(u('total_bytes')) > 0">
          <progress :value="Number(u('received_bytes'))" :max="Number(u('total_bytes'))" />
          <p class="note num">{{ formatBytes(Number(u('received_bytes'))) }} / {{ formatBytes(Number(u('total_bytes'))) }}</p>
        </template>
        <p v-if="u('ready')" class="note">安装时会暂停正在进行的传输和键鼠共享，并自动重新打开 DeskFuse。</p>
        <p v-if="u('error')" class="err" role="alert">{{ u('error') }}</p>
        <p v-if="check.error.value" class="err" role="alert">{{ check.error.value }}</p>
        <p v-if="install.error.value" class="err" role="alert">{{ install.error.value }}</p>
        <button v-if="u('release_url')" type="button" class="plain" @click="link.run('system.open_url', { url: u('release_url') })">查看更新说明</button>
        <p v-if="link.error.value" class="err" role="alert">{{ link.error.value }}</p>
      </div>
      <p v-else class="note">{{ snapshot ? '正在读取版本信息…' : '后端未连接，暂时无法读取版本信息。' }}</p>
    </SettingsSection>

    <HelpFeedback :snapshot="snapshot" :symbol="LifeBuoy" />
  </div>
</template>

<style scoped>
.page { display: grid; align-content: start; gap: 24px; }
.brand { display: flex; align-items: center; gap: 16px; }
.brand-mark {
  width: 48px; height: 48px; flex: none;
  background: url(/deskfuse-icon-light.png) center/contain no-repeat;
}
.brand-text { display: grid; gap: 2px; }
.brand h2 { margin: 0; font-size: 20px; font-weight: 600; }
.update { display: grid; gap: 12px; }
.line { display: flex; align-items: center; gap: 12px; }
.text { flex: 1; min-width: 0; display: grid; gap: 2px; }
.title { font-size: 13px; font-weight: 500; }
.actions { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
progress { width: 100%; height: 4px; accent-color: var(--accent); }
.plain { justify-self: start; border: 0; padding: 0; background: transparent; color: var(--accent); font-size: 12px; cursor: pointer; }
.note { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
.num { font-variant-numeric: tabular-nums; }
.err { margin: 0; font-size: 12px; line-height: 1.5; color: var(--danger); }
</style>

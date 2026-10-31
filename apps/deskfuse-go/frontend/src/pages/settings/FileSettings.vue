<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import ConfirmDialog from './ConfirmDialog.vue'
import SaveFooter from './SaveFooter.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { errorText, formatBytes, get, useAction, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()

const MIB = 1_048_576
const saved = ref<Record<string, any> | null>(null)
const cacheLimit = ref(1024)
const downloadDirectory = ref('')
const saving = ref(false)
const status = ref<string | null>(null)
const failure = ref<string | null>(null)
const pickError = ref<string | null>(null)
const confirmClear = ref(false)
const clear = useAction()

const cache = computed(() => get<any>(props.snapshot, 'cache'))
const cacheMessage = computed(() => String(get(cache.value, 'message') ?? ''))

const dirty = computed(() =>
  !!saved.value &&
  (cacheLimit.value !== Number(get(saved.value, 'maximum')) / MIB || downloadDirectory.value !== (get(saved.value, 'download_directory') ?? '')))

function load(files: Record<string, any>) {
  saved.value = files
  cacheLimit.value = Number(get(files, 'maximum')) / MIB
  downloadDirectory.value = get<string>(files, 'download_directory') ?? ''
  status.value = null
  failure.value = null
}

watch(
  () => get<Record<string, any>>(props.snapshot, 'files'),
  (files) => {
    if (files && JSON.stringify(files) !== JSON.stringify(saved.value) && (!saved.value || (!dirty.value && !saving.value))) load(files)
  },
  { immediate: true, deep: true },
)
watch([cacheLimit, downloadDirectory], () => { if (dirty.value) { status.value = null; failure.value = null } })

async function chooseDirectory() {
  pickError.value = null
  try {
    const path = (await backend.invoke('system.choose_directory', {
      title: '选择暂存区发送文件的保存位置', start: downloadDirectory.value || null,
    })) as string | null
    if (path) { downloadDirectory.value = path; status.value = null; failure.value = null }
  } catch (e) {
    pickError.value = errorText(e)
  }
}

async function save() {
  const base = saved.value
  if (saving.value || !base) return
  const limit = cacheLimit.value
  if (!Number.isFinite(limit) || limit < 1 || limit > 16_777_216) {
    failure.value = '请输入有效的缓存上限（1–16 777 216 MiB）'
    return
  }
  // 传输阈值不暴露给用户：沿用引擎默认的提前传输策略，只提交缓存与接收位置。
  const submitted = {
    enabled: get(base, 'enabled'), prefetch: get(base, 'prefetch'), threshold: get(base, 'threshold'),
    maximum: Math.floor(limit * MIB), download_directory: downloadDirectory.value,
  }
  saving.value = true
  failure.value = null
  status.value = null
  try {
    await backend.invoke('set_files', { settings: submitted })
    load(submitted)
    status.value = '已保存'
  } catch (e) {
    failure.value = errorText(e)
  } finally {
    saving.value = false
  }
}

async function clearCache() {
  confirmClear.value = false
  await clear.run('clear_cache', undefined, '已清除')
}
</script>

<template>
  <div class="wrap">
    <div class="scroll">
      <div class="content" :class="{ off: !saved || saving }">
        <SettingsSection title="接收位置" footer="从文件暂存区发送到此设备的文件会直接保存到该位置。">
          <div class="dir">
            <span class="path" :title="downloadDirectory || undefined">{{ downloadDirectory || '下载文件夹' }}</span>
            <button type="button" class="button" :disabled="saving || !saved" @click="chooseDirectory">选择…</button>
          </div>
          <button v-if="downloadDirectory" type="button" class="plain" :disabled="saving || !saved" @click="downloadDirectory = ''">恢复下载文件夹</button>
          <p v-if="pickError" class="err" role="alert">{{ pickError }}</p>
        </SettingsSection>

        <SettingsSection title="缓存">
          <SettingsRow v-if="cache" compact subdued title="已使用" :detail="cacheMessage || undefined">
            <span class="value">{{ formatBytes(Number(get(cache, 'used'))) }} / {{ formatBytes(Number(get(cache, 'maximum'))) }}</span>
          </SettingsRow>
          <SettingsRow title="缓存上限" detail="传输过程中临时占用的空间上限；超出后复用最早的文件。">
            <input v-model.number="cacheLimit" class="field limit" type="number" min="1" :disabled="saving || !saved" aria-label="缓存上限（MiB）" />
            <span class="unit">MiB</span>
          </SettingsRow>
          <SettingsRow title="历史传输文件" detail="保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。">
            <button type="button" class="button destructive" :disabled="saving || !saved || clear.pending.value" @click="confirmClear = true">清除…</button>
            <template #extra>
              <p v-if="clear.error.value" class="err" role="alert">{{ clear.error.value }}</p>
              <p v-else-if="clear.status.value" class="note">{{ clear.status.value }}</p>
            </template>
          </SettingsRow>
        </SettingsSection>

        <p v-if="!saved" class="caption">{{ snapshot ? '文件设置不可用。' : '后端未连接，暂时无法读取文件设置。' }}</p>
      </div>
    </div>

    <SaveFooter :dirty="dirty" :saving="saving" :can-save="!!saved && (dirty || !!failure)" :can-revert="dirty"
      :status="status" :error="failure" @revert="() => saved && load(saved)" @save="save" />

    <ConfirmDialog :open="confirmClear" title="清除可回收的传输缓存？" message="只清理已完成的传输副本，不删除原始文件或已粘贴副本。"
      confirm-text="清除缓存" destructive @confirm="clearCache" @cancel="confirmClear = false" />
  </div>
</template>

<style scoped>
.wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.scroll { flex: 1; min-height: 0; overflow: auto; padding: 12px 24px 24px; }
.content { display: grid; align-content: start; gap: 24px; max-width: 720px; }
.content.off { opacity: 0.55; pointer-events: none; }
.dir { display: flex; align-items: center; gap: 12px; min-height: 32px; }
.path {
  flex: 1; min-width: 0; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  font-size: 12px; line-height: 15px; color: var(--secondary); overflow-wrap: anywhere; user-select: text;
}
.plain { justify-self: start; border: 0; padding: 0; background: transparent; color: var(--accent); font-size: 13px; cursor: pointer; }
.limit { width: 90px; }
.unit { font-size: 13px; color: var(--secondary); }
.value { font-variant-numeric: tabular-nums; }
.caption { margin: 0; font-size: 12px; color: var(--secondary); }
.note { margin: 0; font-size: 12px; color: var(--secondary); }
.err { margin: 0; font-size: 12px; line-height: 1.5; color: var(--danger); }
</style>

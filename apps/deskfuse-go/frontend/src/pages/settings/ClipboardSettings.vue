<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import SaveFooter from './SaveFooter.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { useClipboardSettings, type LibraryApp } from './useClipboardSettings'
import { errorText, preference, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()
const library = useClipboardSettings()

interface Draft { retention_days: number; excluded_apps: LibraryApp[]; automatic_ocr: boolean }

const original = ref<Draft | null>(null)
const draft = ref<Draft>({ retention_days: 30, excluded_apps: [], automatic_ocr: true })
const loading = ref(false)
const saving = ref(false)
const status = ref<string | null>(null)
const failure = ref<string | null>(null)
const pickError = ref<string | null>(null)
const clearPending = ref(false)
const clearError = ref<string | null>(null)
const clearStatus = ref<string | null>(null)

// 快捷键不在本页编辑：比较时只使用本页字段，避免快捷键页的修改让本页显示“未保存”。
const dirty = computed(() => !!original.value && JSON.stringify(draft.value) !== JSON.stringify(original.value))

async function load() {
  loading.value = true
  failure.value = null
  try {
    const settings = await library.load()
    const value: Draft = {
      retention_days: settings.retention_days,
      excluded_apps: settings.excluded_apps.map((a) => ({ ...a })),
      automatic_ocr: preference(props.snapshot, 'automatic_ocr') !== false,
    }
    original.value = value
    draft.value = JSON.parse(JSON.stringify(value))
  } catch (e) {
    failure.value = errorText(e)
  } finally {
    loading.value = false
  }
}

async function save() {
  if (saving.value || !original.value) return
  const days = draft.value.retention_days
  if (!Number.isInteger(days) || days < 1 || days > 4294967295) {
    failure.value = '请输入有效的保留天数（1–4294967295）。'
    return
  }
  const submitted = JSON.parse(JSON.stringify(draft.value)) as Draft
  saving.value = true
  failure.value = null
  status.value = null
  try {
    // 快捷键可能已在快捷键页单独修改：沿用引擎中的最新值，避免旧草稿把它覆盖回去。
    const latest = await library.load()
    await library.save({ ...latest, retention_days: submitted.retention_days, excluded_apps: submitted.excluded_apps })
    if (submitted.automatic_ocr !== original.value.automatic_ocr) {
      await backend.invoke('settings.set', { key: 'automatic_ocr', value: submitted.automatic_ocr })
    }
    original.value = submitted
    status.value = '已保存'
  } catch (e) {
    failure.value = errorText(e)
  } finally {
    saving.value = false
  }
}

function revert() {
  if (original.value) draft.value = JSON.parse(JSON.stringify(original.value))
  failure.value = null
  status.value = null
}

async function addApp() {
  pickError.value = null
  try {
    // 宿主返回 {bundle_id: string | null}，null 表示用户取消。
    const result = (await backend.invoke('clipboard.choose_app')) as { bundle_id?: string | null } | null
    const id = result?.bundle_id ?? null
    if (!id || draft.value.excluded_apps.some((a) => a.bundle_id === id)) return
    const known = library.apps.value.find((a) => a.bundle_id === id)
    draft.value.excluded_apps.push({ bundle_id: id, name: known?.name ?? id })
  } catch (e) {
    pickError.value = errorText(e)
  }
}

async function clearIndex() {
  clearPending.value = true
  clearError.value = null
  clearStatus.value = null
  try {
    await library.clearRemoteIndex()
    clearStatus.value = '已清除'
  } catch (e) {
    clearError.value = errorText(e)
  } finally {
    clearPending.value = false
  }
}

watch(draft, () => { status.value = null; failure.value = null }, { deep: true })
onMounted(load)
</script>

<template>
  <div class="wrap">
    <div class="scroll">
      <div class="content" :class="{ off: !original || saving }">
        <SettingsSection title="普通历史">
          <SettingsRow title="保留天数" detail="默认保留 30 天，不限制条数。手动列表成员长期保留；智能列表不会延长历史保留期。">
            <input v-model.number="draft.retention_days" class="field days" type="number" min="1" aria-label="保留天数" />
          </SettingsRow>
        </SettingsSection>

        <SettingsSection title="跨设备搜索">
          <SettingsRow title="同步标题与识别文字到其他设备" detail="始终开启：只同步标题、识别文字和元数据，方便在其他设备搜索；图片和文件正文按需获取。" />
          <SettingsRow title="已同步的远端索引" detail="清除其他设备同步过来的索引行，不影响本机历史、手动列表或已获取的正文。">
            <button type="button" class="button" :disabled="clearPending" @click="clearIndex">清除远端索引</button>
            <template #extra>
              <p v-if="clearError" class="err" role="alert">{{ clearError }}</p>
              <p v-else-if="clearStatus" class="note">{{ clearStatus }}</p>
            </template>
          </SettingsRow>
        </SettingsSection>

        <SettingsSection title="图片文字识别">
          <SettingsRow title="自动识别图片文字" detail="默认开启，识别在可自动退出的独立进程中运行。关闭后保留已有识别结果；图片预览和复制始终可用。">
            <input v-model="draft.automatic_ocr" class="toggle" type="checkbox" role="switch" aria-label="自动识别图片文字" />
          </SettingsRow>
        </SettingsSection>

        <SettingsSection title="排除 App">
          <div v-for="app in draft.excluded_apps" :key="app.bundle_id" class="app">
            <span class="sel">{{ app.name }}</span>
            <span class="spacer" />
            <button type="button" class="button destructive" @click="draft.excluded_apps = draft.excluded_apps.filter((a) => a.bundle_id !== app.bundle_id)">移除</button>
          </div>
          <SettingsRow title="不保存历史的应用" detail="新复制仍正常实时同步，但发送端和接收端不保存历史。不会删除已有历史或手动列表内容；此配置仅属于当前设备。">
            <button type="button" class="button" @click="addApp">添加应用…</button>
            <template #extra><p v-if="pickError" class="err" role="alert">{{ pickError }}</p></template>
          </SettingsRow>
        </SettingsSection>

        <p class="caption">本页修改统一在点击底部“保存”后生效。</p>
      </div>
    </div>

    <div v-if="!original" class="load">
      <template v-if="loading"><span class="spinner" aria-hidden="true" />正在读取设置…</template>
      <template v-else>
        <span v-if="!failure">{{ snapshot ? '' : '后端未连接，无法读取剪贴板设置。' }}</span>
        <button type="button" class="button" @click="load">重新读取设置</button>
      </template>
    </div>

    <SaveFooter :dirty="dirty" :saving="saving" :can-save="!!original && (dirty || !!failure)" :can-revert="dirty"
      :status="status" :error="failure" @revert="revert" @save="save" />
  </div>
</template>

<style scoped>
.wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.scroll { flex: 1; min-height: 0; overflow: auto; padding: 12px 24px 24px; }
.content { display: grid; align-content: start; gap: 24px; max-width: 720px; }
.content.off { opacity: 0.55; pointer-events: none; }
.days { width: 80px; }
.app { display: flex; align-items: center; min-height: 32px; }
.sel { user-select: text; }
.spacer { flex: 1; }
.load { display: flex; align-items: center; gap: 8px; padding: 0 24px 12px; font-size: 13px; color: var(--secondary); }
.caption { margin: 0; padding: 0 4px; font-size: 11px; color: var(--secondary); }
.note { margin: 0; font-size: 12px; color: var(--secondary); }
.err { margin: 0; font-size: 12px; line-height: 1.5; color: var(--danger); }
.spinner {
  width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line);
  border-top-color: var(--accent); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>

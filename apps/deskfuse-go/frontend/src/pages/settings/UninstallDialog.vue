<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import ConfirmDialog from './ConfirmDialog.vue'
import { errorText, get } from './util'
import { permissionId } from './usePermissions'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: [] }>()

const CONFIRMATION_WORD = '卸载'

const TITLES: Record<string, string> = {
  user_data: '剪贴板历史与本机数据',
  caches: '缓存、日志与更新下载',
  preferences: '偏好设置',
  keychain: '钥匙串中的 Windows 远程应用账号',
  login_item: '开机自动启动项',
  privacy: '系统权限记录',
  microphone_driver: '麦克风驱动',
  application: 'DeskFuse 应用本身',
}
const DETAILS: Record<string, string> = {
  user_data: '剪贴板历史、导入的文件、配对记录与设备身份（~/Library/Application Support/DeskFuse）。三者存放在同一数据库中，只能一起保留或删除；保留后重装可继续使用。',
  caches: '可重新生成，不含剪贴板历史。',
  preferences: '界面与功能开关等设置。',
  keychain: '重装后需要重新输入 Windows 账号密码。',
  login_item: '登录系统时不再自动打开 DeskFuse。',
  privacy: '辅助功能、屏幕录制、输入监控等，重装后需重新授权。',
  microphone_driver: '需要输入管理员密码，并会重启音频服务。',
  application: '移到废纸篓。',
}

const step = ref<'review' | 'confirm'>('review')
const components = ref<string[]>([])
const selection = ref<Set<string>>(new Set())
const homebrew = ref(false)
const confirmation = ref('')
const loading = ref(false)
const running = ref(false)
const failure = ref<string | null>(null)

const title = (id: string) => TITLES[id] ?? id
const selectedList = computed(() => components.value.filter((id) => selection.value.has(id)))

async function loadPlan() {
  loading.value = true
  failure.value = null
  try {
    const plan = await backend.invoke('uninstall.plan')
    components.value = (get<any[]>(plan, 'components') ?? []).map((c) => permissionId(String(typeof c === 'string' ? c : get(c, 'id'))))
    selection.value = new Set(((get<any[]>(plan, 'default_selection') ?? []) as string[]).map(permissionId))
    homebrew.value = get(plan, 'installed_by_homebrew') === true
  } catch (e) {
    components.value = []
    failure.value = errorText(e)
  } finally {
    loading.value = false
  }
}

watch(() => props.open, (open) => {
  if (!open) return
  step.value = 'review'
  confirmation.value = ''
  running.value = false
  loadPlan()
}, { immediate: true })

function toggle(id: string, on: boolean) {
  const next = new Set(selection.value)
  if (on) next.add(id)
  else next.delete(id)
  selection.value = next
}

async function uninstall() {
  if (running.value) return
  running.value = true
  failure.value = null
  try {
    const result = await backend.invoke('uninstall.perform', { components: selectedList.value })
    // 成功时应用已经开始退出；取消管理员授权则什么都没删除。
    if (get(result, 'completed') === false) {
      running.value = false
      failure.value = '已取消管理员授权，没有删除任何内容。'
    }
  } catch (e) {
    running.value = false
    failure.value = errorText(e)
    await loadPlan()
  }
}

const canContinue = computed(() => selection.value.size > 0 && components.value.length > 0)
const canConfirm = computed(() => confirmation.value.trim() === CONFIRMATION_WORD)
</script>

<template>
  <ConfirmDialog
    :open="open" title="彻底卸载 DeskFuse" :width="500" :busy="running" :error="failure" destructive
    :cancel-text="'取消'"
    :confirm-text="step === 'review' ? '继续…' : running ? '正在卸载…' : '彻底卸载并退出'"
    :confirm-disabled="step === 'review' ? !canContinue || loading : !canConfirm"
    :message="step === 'review' ? '勾选要从这台设备上永久删除的内容，删除后无法恢复。' : '最后确认：卸载完成后 DeskFuse 会立即退出。'"
    @confirm="step === 'review' ? (step = 'confirm') : uninstall()" @cancel="emit('close')">
    <div v-if="step === 'review'" class="card review">
      <p v-if="loading" class="note">正在读取可卸载的内容…</p>
      <label v-for="id in components" :key="id" class="item">
        <input type="checkbox" :checked="selection.has(id)" @change="toggle(id, ($event.target as HTMLInputElement).checked)" />
        <span>
          <span class="t">{{ title(id) }}</span>
          <span class="d">{{ DETAILS[id] }}</span>
        </span>
      </label>
      <p class="note">其他已配对设备上仍会保留这台设备的记录，可在对方的设备页中移除。</p>
      <p v-if="homebrew && selection.has('application')" class="note">检测到通过 Homebrew 安装，卸载后请在终端执行 <code>brew uninstall --cask deskfuse</code> 清理安装记录。</p>
    </div>

    <div v-else class="confirm">
      <div class="notice card" :class="{ danger: selection.has('user_data') }">
        <strong>{{ selection.has('user_data') ? '剪贴板历史将被永久删除' : '剪贴板历史将保留' }}</strong>
        <p class="note">将删除：{{ selectedList.map(title).join('、') }}。</p>
      </div>
      <label class="word">请输入“{{ CONFIRMATION_WORD }}”以确认：
        <input v-model="confirmation" class="field" :placeholder="CONFIRMATION_WORD" :disabled="running" @keydown.enter="canConfirm && uninstall()" />
      </label>
    </div>
  </ConfirmDialog>
</template>

<style scoped>
.review { display: grid; gap: 12px; padding: 14px; }
.item { display: flex; align-items: flex-start; gap: 8px; }
.item span { display: grid; gap: 3px; }
.t { font-size: 13px; }
.d { font-size: 12px; line-height: 1.5; color: var(--secondary); }
.confirm { display: grid; gap: 12px; }
.notice { display: grid; gap: 4px; padding: 12px; }
.notice.danger strong { color: var(--danger); }
.word { display: grid; gap: 6px; font-size: 13px; }
.note { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary); }
</style>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import DeviceIcon from './DeviceIcon.vue'
import Icon from './Icon.vue'
import { localStore } from './core'
import { backend, errorMessage } from '../../services/backend'

// 对应 RemoteApplicationCatalog + RemoteApplicationPicker（含 RemoteRdpPicker）。
// 目录连接的生命周期由本面板负责；打开应用仍交给原生窗口。
const props = defineProps<{ device: Record<string, any> }>()

const rows = ref<any[]>([])
const accounts = ref<any[]>([])
const accountMode = ref<'saved' | 'manual'>('saved')
const accountID = ref('')
const search = ref('')
const searchFocused = ref(false)
const loading = ref(false)
const error = ref('')
const actionError = ref('')
const rdp = ref(false)
const opening = ref<string | null>(null)
const scope = ref<'all' | 'favorites'>('all')
const virtualDisplay = ref(false)
// 与原生 RemoteApplicationCategory 同一套分类规则（macOS 应用目录）。
const CATEGORIES = ['全部', '运行中', '效率与财务', '工具', '开发者工具', '社交', '娱乐', '信息与阅读', '创意', '其他'] as const
type Category = (typeof CATEGORIES)[number]
const category = ref<Category>('全部')
const systemExpanded = ref(false)
const accountMenuOpen = ref(false)
const accountRoot = ref<HTMLElement | null>(null)
const managingAccounts = ref(false)
const managerMode = ref<'list' | 'edit'>('list')
const managerPanel = ref<HTMLElement | null>(null)
const confirmRemoveButton = ref<HTMLButtonElement | null>(null)
const editingAccountID = ref<string | null>(null)
const editorName = ref('')
const editorUsername = ref('')
const editorPassword = ref('')
const removingAccountID = ref('')
const removingAccountBusy = ref(false)
const accountError = ref('')
const savingAccount = ref(false)
const manualUsername = ref('')
const manualPassword = ref('')
const saveManualAccount = ref(true)
const manualAccountName = ref('')

let disposed = false
let revision = 0

const favoritesStore = localStore<string[]>(`deskfuse.remoteapps.favorites.${props.device.id}`, [])
const storedFavorites = favoritesStore.read()
const favorites = ref<Set<string>>(new Set(Array.isArray(storedFavorites) ? storedFavorites.map(String) : []))

const FRIENDLY_NAMES: Record<string, string> = {
  chrome: 'Google Chrome',
  msedge: 'Microsoft Edge',
  notepad: '记事本',
  iexplore: 'Internet Explorer',
  mstsc: '远程桌面连接',
}

function windowsBasename(path: string): string {
  const parts = path.split('\\').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : path
}
function normalizedProgram(value: string): string {
  const lower = value.toLowerCase()
  return lower.endsWith('.exe') ? lower.slice(0, -4) : lower
}
/** 与 RemoteRdpAppPresentation.title 一致：优先友好名称，其次对端展示名。 */
function appTitle(app: any): string {
  if (!rdp.value) return String(app.name ?? '')
  const display = String(app.name ?? '').trim()
  const basename = windowsBasename(String(app.id ?? ''))
  const displayProgram = normalizedProgram(display)
  const pathProgram = normalizedProgram(basename)
  if (FRIENDLY_NAMES[displayProgram]) return FRIENDLY_NAMES[displayProgram]
  if (display) {
    const matchesPath = display.toLowerCase() === basename.toLowerCase() || displayProgram === pathProgram
    if (matchesPath && FRIENDLY_NAMES[pathProgram]) return FRIENDLY_NAMES[pathProgram]
    return display
  }
  return FRIENDLY_NAMES[pathProgram] ?? String(app.name ?? '')
}

/** 与 RemoteRdpAppPresentation.isSystemOrInternal 一致。 */
function isSystemApp(app: any): boolean {
  const path = String(app.id ?? '')
  const components = path.split('\\').filter(Boolean).map((part) => part.toLowerCase())
  const inSystemApps = components.some(
    (part, index) => index + 1 < components.length && part === 'windows' && components[index + 1] === 'systemapps',
  )
  const displayBase = windowsBasename(String(app.name ?? '')).toLowerCase()
  const executable = windowsBasename(path).toLowerCase()
  return inSystemApps || displayBase === 'iediagcmd.exe' || executable === 'iediagcmd.exe'
}

const query = computed(() => search.value.trim().toLowerCase())
/** 与原生 RemoteApplicationCategory.category(for:) 一致。 */
function macCategory(app: any): Category {
  const value = String(app.category ?? '')
  if (value.includes('developer')) return '开发者工具'
  if (['business', 'productivity', 'finance'].some((part) => value.includes(part))) return '效率与财务'
  if (value.includes('utilities')) return '工具'
  if (value.includes('social')) return '社交'
  if (['graphics', 'design', 'photography', 'video'].some((part) => value.includes(part))) return '创意'
  if (['entertainment', 'games', 'music', 'sports'].some((part) => value.includes(part))) return '娱乐'
  if (['news', 'weather', 'education', 'reference', 'books', 'medical', 'health', 'travel'].some((part) => value.includes(part))) return '信息与阅读'
  return '其他'
}
const visible = computed(() => {
  let list = rows.value
  if (rdp.value) {
    if (scope.value === 'favorites') list = list.filter((app) => favorites.value.has(String(app.id)))
  } else if (category.value === '运行中') {
    list = list.filter((app) => app.running === true)
  } else if (category.value !== '全部') {
    list = list.filter((app) => macCategory(app) === category.value)
  }
  if (query.value) {
    list = list.filter((app) =>
      [appTitle(app), app.name, app.id, app.bundle_id, app.category].some((value) =>
        String(value ?? '').toLowerCase().includes(query.value),
      ),
    )
  }
  if (!rdp.value) {
    // 原生 sortedApplications：运行中的排在最前，其余按名称排序。
    list = [...list].sort((a, b) => {
      const running = Number(b.running === true) - Number(a.running === true)
      return running !== 0 ? running : String(a.name ?? '').localeCompare(String(b.name ?? ''), 'zh-CN')
    })
  }
  return list
})
const openedCount = computed(() => rows.value.filter((app) => app.opened === true).length)
const appStatus = (app: any): string => (app.opened === true ? '已打开' : app.running === true ? '运行中' : '')
const regularApps = computed(() => (rdp.value ? visible.value.filter((app) => !isSystemApp(app)) : visible.value))
const systemApps = computed(() => (rdp.value ? visible.value.filter(isSystemApp) : []))
const systemOpen = computed(() => systemExpanded.value || query.value.length > 0)

function accountTitle(account: any): string {
  const name = String(account.name ?? '').trim()
  const user = String(account.username ?? '').trim()
  if (!name) return user || 'Windows 账号'
  return name === user ? user : `${name}（${user}）`
}
const selectedAccount = computed(
  () => accounts.value.find((account) => String(account.id ?? '') === accountID.value) ?? null,
)
const selectedAccountTitle = computed(() => {
  if (accountMode.value === 'manual') return '使用其他账号'
  return selectedAccount.value ? accountTitle(selectedAccount.value) : '选择登录账号'
})
const manualCredentialsReady = computed(
  () => manualUsername.value.trim().length > 0 && manualPassword.value.length > 0,
)
const canOpen = computed(
  () =>
    !rdp.value
    || (accountMode.value === 'saved' ? !!selectedAccount.value : manualCredentialsReady.value),
)
const removingAccount = computed(
  () => accounts.value.find((account) => String(account.id ?? '') === removingAccountID.value) ?? null,
)

function selectAccount(account: any) {
  accountMode.value = 'saved'
  accountID.value = String(account.id ?? '')
  accountMenuOpen.value = false
  actionError.value = ''
}
function selectManualAccount() {
  accountMode.value = 'manual'
  accountMenuOpen.value = false
  actionError.value = ''
}
async function toggleFavorite(app: any) {
  const id = String(app.id)
  const adding = !favorites.value.has(id)
  const optimistic = new Set(favorites.value)
  if (adding) optimistic.add(id)
  else optimistic.delete(id)
  favorites.value = optimistic
  try {
    // 收藏与原生选择器共用同一份数据（UserDefaults），不只在 WebView 内生效。
    const result = await backend.invoke('remoteApps.setFavorite', {
      device_id: props.device.id, application_id: id, favorite: adding,
    })
    if (Array.isArray(result?.favorites)) favorites.value = new Set(result.favorites.map(String))
    favoritesStore.write([...favorites.value].sort())
  } catch (cause) {
    favorites.value = new Set(favorites.value)
    const revert = new Set(favorites.value)
    if (adding) revert.delete(id)
    else revert.add(id)
    favorites.value = revert
    actionError.value = errorMessage(cause)
  }
}

async function setVirtualDisplay(enabled: boolean) {
  virtualDisplay.value = enabled
  try { await backend.invoke('remoteApps.setVirtualDisplay', { enabled }) }
  catch (cause) { virtualDisplay.value = !enabled; actionError.value = errorMessage(cause) }
}
function clearSearch() {
  search.value = ''
}

async function load(refresh = false) {
  const current = ++revision
  loading.value = true
  error.value = ''
  try {
    const result = await backend.invoke('remoteApps.catalog', { device_id: props.device.id, refresh })
    if (disposed || current !== revision) return
    rows.value = result.applications ?? []
    accounts.value = result.accounts ?? []
    rdp.value = result.rdp === true
    virtualDisplay.value = result.virtual_display === true
    if (Array.isArray(result.favorites)) favorites.value = new Set(result.favorites.map(String))
    if (
      accountMode.value === 'saved'
      && !accounts.value.some((account) => String(account.id ?? '') === accountID.value)
    ) {
      const first = accounts.value[0]
      if (first) accountID.value = String(first.id ?? '')
      else {
        // 没有已保存账号时直接进入手动输入，仍可填写后打开应用。
        accountID.value = ''
        accountMode.value = 'manual'
      }
    }
    if (result.error) error.value = String(result.error)
  } catch (cause) {
    if (!disposed && current === revision) error.value = errorMessage(cause)
  } finally {
    if (!disposed && current === revision) loading.value = false
  }
}

/** 主机保存常用账号失败时不阻止打开，只在结果中附带错误；有则展示。 */
function reportedAccountError(result: any): string {
  if (!result || typeof result !== 'object') return ''
  for (const key of ['account_error', 'save_error', 'error']) {
    const value = result[key]
    if (typeof value === 'string' && value.trim()) return value
  }
  return ''
}

async function open(app: any) {
  if (!canOpen.value) {
    actionError.value = accountMode.value === 'manual'
      ? '请填写 Windows 用户名和密码。'
      : '请先选择或添加 Windows 登录账号。'
    return
  }
  opening.value = String(app.id)
  actionError.value = ''
  try {
    const params: Record<string, unknown> = {
      device_id: props.device.id,
      application_id: app.id,
    }
    if (rdp.value) {
      if (accountMode.value === 'manual') {
        params.username = manualUsername.value.trim()
        params.password = manualPassword.value
        if (saveManualAccount.value) {
          params.save_account = true
          params.account_name = manualAccountName.value.trim()
        }
      } else if (selectedAccount.value) {
        params.account_id = selectedAccount.value.id
      }
    }
    const result = await backend.invoke('remoteApps.open', params)
    const reported = reportedAccountError(result)
    if (reported) actionError.value = reported
    // 保存为常用账号后刷新菜单；保存失败不影响本次打开。
    if (rdp.value && accountMode.value === 'manual' && saveManualAccount.value) void load()
  } catch (cause) {
    actionError.value = errorMessage(cause)
  } finally {
    opening.value = null
  }
}

function openAccountManager() {
  accountMenuOpen.value = false
  accountError.value = ''
  removingAccountID.value = ''
  editingAccountID.value = null
  editorName.value = ''
  editorUsername.value = ''
  editorPassword.value = ''
  managerMode.value = 'list'
  managingAccounts.value = true
}

function startAddAccount() {
  editingAccountID.value = null
  editorName.value = ''
  editorUsername.value = ''
  editorPassword.value = ''
  accountError.value = ''
  removingAccountID.value = ''
  managerMode.value = 'edit'
}

function startEditAccount(account: any) {
  const name = String(account.name ?? '').trim()
  editingAccountID.value = String(account.id ?? '')
  editorName.value = name
  editorUsername.value = String(account.username ?? '').trim()
  editorPassword.value = ''
  accountError.value = ''
  removingAccountID.value = ''
  managerMode.value = 'edit'
}

function cancelAccountEditor() {
  managerMode.value = 'list'
  editingAccountID.value = null
  editorPassword.value = ''
  accountError.value = ''
}

function closeAccountManager() {
  managingAccounts.value = false
  managerMode.value = 'list'
  editingAccountID.value = null
  editorName.value = ''
  editorUsername.value = ''
  editorPassword.value = ''
  removingAccountID.value = ''
  removingAccountBusy.value = false
  accountError.value = ''
  savingAccount.value = false
}

async function saveAccount() {
  const id = editingAccountID.value
  const username = editorUsername.value.trim()
  if (!username || !editorPassword.value) {
    // 桥接要求保存时密码必填，因此编辑也必须重新输入密码。
    accountError.value = id
      ? '请填写用户名和密码；编辑账号需要重新输入密码。'
      : '请填写 Windows 用户名和账户密码。'
    return
  }
  savingAccount.value = true
  accountError.value = ''
  try {
    const result = await backend.invoke('remoteApps.saveAccount', {
      ...(id ? { id } : {}),
      name: editorName.value.trim(),
      username,
      password: editorPassword.value,
    })
    const savedID = String(result?.id ?? id ?? '')
    editorPassword.value = ''
    await load()
    managerMode.value = 'list'
    editingAccountID.value = null
    editorName.value = ''
    editorUsername.value = ''
    if (savedID) {
      accountMode.value = 'saved'
      accountID.value = savedID
    }
  } catch (cause) {
    accountError.value = errorMessage(cause)
  } finally {
    savingAccount.value = false
  }
}

function requestRemoveAccount(account: any) {
  removingAccountID.value = String(account.id ?? '')
  accountError.value = ''
}

async function cancelRemoveAccount() {
  removingAccountID.value = ''
  await nextTick()
  managerPanel.value?.focus()
}

async function confirmRemoveAccount() {
  const account = removingAccount.value
  if (!account) return
  removingAccountBusy.value = true
  accountError.value = ''
  try {
    await backend.invoke('remoteApps.removeAccount', { id: account.id, confirm: true })
    removingAccountID.value = ''
    await load()
    await nextTick()
    managerPanel.value?.focus()
  } catch (cause) {
    accountError.value = errorMessage(cause)
  } finally {
    removingAccountBusy.value = false
  }
}

function closeAccountMenu(event: MouseEvent) {
  if (accountRoot.value && !accountRoot.value.contains(event.target as Node)) accountMenuOpen.value = false
}
function tileHelp(app: any): string {
  if (rdp.value) return [app.name, app.id].filter(Boolean).join('\n')
  return `打开 ${appTitle(app)}`
}
const isFavorite = (app: any) => favorites.value.has(String(app.id))

watch(managingAccounts, async (open) => {
  if (!open) return
  await nextTick()
  managerPanel.value?.focus()
})
watch(managerMode, async () => {
  if (!managingAccounts.value) return
  await nextTick()
  managerPanel.value?.focus()
})
watch(removingAccountID, async (id) => {
  if (!id) return
  await nextTick()
  confirmRemoveButton.value?.focus()
})

onMounted(() => {
  document.addEventListener('mousedown', closeAccountMenu)
  void load()
})
onBeforeUnmount(() => {
  disposed = true
  ++revision
  document.removeEventListener('mousedown', closeAccountMenu)
  // 只关闭浏览通道；已打开的远程应用窗口由原生管理器保留。
  void backend.invoke('remoteApps.closeCatalog', { device_id: props.device.id }).catch(() => {})
})
</script>

<template>
  <section class="catalog">
    <header class="catalog-head">
      <DeviceIcon :name="device.name" :src="device.iconUrl" :size="42" />
      <div class="head-text">
        <div class="df-title df-ellipsis">{{ device.name }}</div>
        <div class="df-detail df-secondary">应用列表</div>
      </div>
      <span class="df-spacer" />
      <button type="button" class="refresh-button" aria-label="刷新应用列表" title="刷新应用列表" :disabled="loading" @click="load(true)">
        <Icon name="refresh" :size="15" />
      </button>
    </header>
    <hr class="df-divider" />

    <div v-if="loading && !rows.length" class="state-wrap">
      <div class="state">
        <span class="spinner" role="status" aria-label="正在读取应用列表" />
        <p>{{ rdp ? '正在读取 Windows 应用…' : '正在读取应用列表…' }}</p>
      </div>
    </div>

    <div v-else-if="error && !rows.length" class="state-wrap">
      <div class="state">
        <Icon name="alert" :size="40" />
        <h2>{{ rdp ? '无法读取 Windows 应用' : '无法读取应用列表' }}</h2>
        <p class="selectable">{{ error }}</p>
        <button type="button" class="button primary" @click="load(true)">重试</button>
      </div>
    </div>

    <div v-else-if="!rows.length" class="state-wrap">
      <div class="state">
        <Icon name="grid" :size="40" />
        <h2>{{ rdp ? '没有可用的远程应用' : '暂无可用应用' }}</h2>
        <p>{{ rdp ? 'Windows 尚未提供可用应用。新版服务会自动发现应用并在首次打开时注册；旧版服务仍需手动注册。' : '确认对方已开启远程应用，然后刷新应用列表。' }}</p>
        <button type="button" class="button" @click="load(true)">刷新应用列表</button>
      </div>
    </div>

    <div v-else class="catalog-body">
      <div class="toolbar">
        <div v-if="rdp" ref="accountRoot" class="account">
          <button
            type="button"
            class="account-button"
            :aria-label="`登录账号：${selectedAccountTitle}`"
            aria-haspopup="menu"
            :aria-expanded="accountMenuOpen"
            title="选择用于登录 Windows 的账号；密码仅保存在本机钥匙串"
            @click="accountMenuOpen = !accountMenuOpen"
          >
            <Icon name="user" :size="14" />
            <span class="account-title df-ellipsis">{{ selectedAccountTitle }}</span>
            <Icon name="chevron-down" :size="11" />
          </button>
          <div v-if="accountMenuOpen" class="account-menu" role="menu">
            <button
              v-for="account in accounts"
              :key="account.id"
              type="button"
              class="menu-item"
              role="menuitemradio"
              :aria-checked="accountMode === 'saved' && String(account.id ?? '') === accountID"
              @click="selectAccount(account)"
            >
              <span class="menu-check">
                <Icon v-if="accountMode === 'saved' && String(account.id ?? '') === accountID" name="check" :size="13" />
              </span>
              <span class="df-ellipsis">{{ accountTitle(account) }}</span>
            </button>
            <p v-if="!accounts.length" class="menu-empty df-caption df-secondary">还没有保存的账号</p>
            <hr class="df-divider" />
            <button
              type="button"
              class="menu-item"
              role="menuitemradio"
              :aria-checked="accountMode === 'manual'"
              @click="selectManualAccount"
            >
              <span class="menu-check">
                <Icon v-if="accountMode === 'manual'" name="check" :size="13" />
              </span>
              <span class="df-ellipsis">使用其他账号…</span>
            </button>
            <button type="button" class="menu-item" role="menuitem" @click="openAccountManager">
              <span class="menu-check"><Icon name="users" :size="13" /></span>
              <span class="df-ellipsis">管理账号…</span>
            </button>
          </div>
        </div>
        <span class="df-spacer" />
        <div v-if="rdp" class="scope" role="tablist" aria-label="应用范围">
          <button type="button" role="tab" :aria-pressed="scope === 'all'" @click="scope = 'all'">全部</button>
          <button type="button" role="tab" :aria-pressed="scope === 'favorites'" @click="scope = 'favorites'">收藏</button>
        </div>
        <label class="catalog-search" :class="{ focused: searchFocused }">
          <input
            v-model="search"
            placeholder="搜索应用"
            aria-label="搜索远程应用"
            @focus="searchFocused = true"
            @blur="searchFocused = false"
            @keydown.esc.prevent="clearSearch"
          />
          <button v-if="search" type="button" class="clear-search" aria-label="清除应用搜索" @click="clearSearch">
            <Icon name="x-circle" :size="13" />
          </button>
        </label>
      </div>

      <div v-if="rdp && accountMode === 'manual'" class="manual-account">
        <div class="manual-fields">
          <label class="field-label">
            Windows 用户名或 Microsoft 邮箱
            <input
              v-model="manualUsername"
              class="field"
              autocomplete="username"
              placeholder="例如 name@example.com"
            />
          </label>
          <label class="field-label">
            账户密码（不是 PIN）
            <input
              v-model="manualPassword"
              class="field"
              type="password"
              autocomplete="current-password"
              placeholder="Windows 账户密码"
            />
          </label>
        </div>
        <div class="manual-save">
          <label class="save-option">
            <input v-model="saveManualAccount" class="toggle" type="checkbox" />
            <span>保存为常用账号</span>
          </label>
          <label v-if="saveManualAccount" class="manual-name">
            <span>备注名（可选）</span>
            <input v-model="manualAccountName" class="field" autocomplete="off" placeholder="例如 工作账号" />
          </label>
        </div>
        <p class="df-caption df-secondary">
          {{ manualCredentialsReady ? '点选下方应用即可登录。' : '填写用户名和密码后，点选下方应用即可登录。' }}
        </p>
      </div>

      <div v-if="!rdp" class="categories">
        <div class="category-chips" role="tablist" aria-label="应用分类">
          <button
            v-for="item in CATEGORIES" :key="item" type="button" role="tab"
            :aria-pressed="category === item" :class="{ on: category === item }"
            @click="category = item"
          >{{ item }}</button>
        </div>
        <span class="df-spacer" />
        <label class="virtual-display" title="开启后，每个远程应用使用专属虚拟屏幕；关闭后使用被控 Mac 上的原有应用窗口。下次打开或重新连接时生效。">
          <span>虚拟屏幕</span>
          <span class="badge beta">实验性</span>
          <input class="toggle" type="checkbox" :checked="virtualDisplay" aria-label="虚拟屏幕模式" @change="setVirtualDisplay(($event.target as HTMLInputElement).checked)" />
        </label>
      </div>

      <p v-if="actionError" class="inline-error" role="alert"><Icon name="alert" :size="14" />{{ actionError }}</p>
      <p v-else-if="error" class="inline-error" role="alert"><Icon name="alert" :size="14" />{{ error }}</p>

      <div v-if="!visible.length" class="state inline">
        <template v-if="rdp && scope === 'favorites' && !query">
          <svg class="empty-star" viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
            <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
          </svg>
          <h2>还没有收藏的应用</h2>
          <p>点按应用卡片右上角的星标即可收藏。</p>
        </template>
        <template v-else>
          <Icon name="search" :size="40" />
          <h2>没有找到“{{ search.trim() }}”</h2>
          <p>换一个关键词试试，或清除搜索。</p>
          <button type="button" class="button" @click="clearSearch">清除搜索</button>
        </template>
      </div>

      <template v-else>
        <div v-if="regularApps.length" class="applications">
          <div v-for="app in regularApps" :key="app.id" class="tile-wrap">
            <button
              type="button"
              class="tile"
              :class="{ mac: !rdp }"
              :disabled="!canOpen || opening !== null"
              :title="tileHelp(app)"
              :aria-label="`打开 ${appTitle(app)}`"
              @click="open(app)"
            >
              <img v-if="!rdp && app.icon" class="tile-app-icon" :src="app.icon" alt="" draggable="false" />
              <span v-else class="tile-icon"><Icon name="window" :size="24" /></span>
              <span class="tile-name" :class="{ single: !rdp }">{{ appTitle(app) }}</span>
              <span v-if="!rdp" class="tile-status">
                <i v-if="app.opened === true || app.running === true" :class="{ open: app.opened === true }" />
                {{ appStatus(app) }}
              </span>
            </button>
            <button
              v-if="rdp"
              type="button"
              class="favorite"
              :class="{ on: isFavorite(app) }"
              :aria-label="`${isFavorite(app) ? '取消收藏' : '收藏'} ${appTitle(app)}`"
              :title="isFavorite(app) ? '取消收藏' : '收藏'"
              @click.stop="toggleFavorite(app)"
            >
              <svg viewBox="0 0 24 24" width="16" height="16" :fill="isFavorite(app) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
                <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
              </svg>
            </button>
          </div>
        </div>

        <button v-if="systemApps.length" type="button" class="system-toggle" @click="systemExpanded = !systemExpanded">
          <Icon :name="systemOpen ? 'chevron-down' : 'chevron-right'" :size="13" />
          <span>系统与内部组件</span>
          <span class="system-count">{{ systemApps.length }}</span>
        </button>

        <div v-if="systemApps.length && systemOpen" class="applications">
          <div v-for="app in systemApps" :key="app.id" class="tile-wrap">
            <button
              type="button"
              class="tile"
              :disabled="!canOpen || opening !== null"
              :title="tileHelp(app)"
              :aria-label="`打开 ${appTitle(app)}`"
              @click="open(app)"
            >
              <span class="tile-icon"><Icon name="window" :size="24" /></span>
              <span class="tile-name">{{ appTitle(app) }}</span>
            </button>
            <button
              type="button"
              class="favorite"
              :class="{ on: isFavorite(app) }"
              :aria-label="`${isFavorite(app) ? '取消收藏' : '收藏'} ${appTitle(app)}`"
              :title="isFavorite(app) ? '取消收藏' : '收藏'"
              @click.stop="toggleFavorite(app)"
            >
              <svg viewBox="0 0 24 24" width="16" height="16" :fill="isFavorite(app) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
                <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
              </svg>
            </button>
          </div>
        </div>

        <p v-if="rdp" class="footnote df-caption df-secondary">画面与输入经 DeskFuse 加密隧道转发，Windows 只需开启远程桌面。</p>
        <p v-else class="catalog-foot df-caption df-secondary"><span>{{ device.name }}</span><span>{{ openedCount }} 个已打开</span></p>
      </template>
    </div>

    <div v-if="managingAccounts" class="modal-backdrop" @click.self="closeAccountManager">
      <div
        ref="managerPanel"
        class="modal account-manager"
        role="dialog"
        aria-modal="true"
        aria-labelledby="account-manager-title"
        tabindex="-1"
        @keydown.esc.stop.prevent="closeAccountManager"
      >
        <template v-if="managerMode === 'list'">
          <div class="manager-head">
            <h2 id="account-manager-title">Windows 登录账号</h2>
            <span class="df-spacer" />
            <button type="button" class="button" aria-label="添加 Windows 登录账号" @click="startAddAccount">
              <Icon name="plus" :size="14" />
              添加账号
            </button>
          </div>
          <p class="manager-intro df-secondary">账号在所有 Windows 设备间共用；密码仅保存在本机钥匙串，不会发送到 DeskFuse 服务端。</p>

          <div v-if="!accounts.length" class="manager-empty">
            <Icon name="user-plus" :size="28" />
            <p>还没有保存的账号</p>
            <p class="df-caption df-secondary">添加后可在打开 Windows 应用时直接选择，无需每次输入密码；也可以直接使用其他账号登录。</p>
            <button type="button" class="button" @click="startAddAccount">添加账号</button>
          </div>
          <ul v-else class="account-list">
            <li v-for="account in accounts" :key="account.id" class="account-row">
              <Icon name="user" :size="18" />
              <span class="account-row-text">
                <span class="account-row-name df-ellipsis">{{ String(account.name ?? '').trim() || account.username }}</span>
                <span class="account-row-user df-caption df-secondary df-ellipsis">{{ account.username }}</span>
              </span>
              <button
                type="button"
                class="icon-button"
                :aria-label="`编辑 ${accountTitle(account)}`"
                title="编辑账号"
                @click="startEditAccount(account)"
              >
                <Icon name="pencil" :size="14" />
              </button>
              <button
                type="button"
                class="icon-button danger"
                :aria-label="`删除 ${accountTitle(account)}`"
                title="删除账号"
                @click="requestRemoveAccount(account)"
              >
                <Icon name="trash" :size="14" />
              </button>
            </li>
          </ul>

          <div v-if="removingAccount" class="remove-confirm" role="alertdialog" aria-label="确认删除账号">
            <p>删除账号“{{ accountTitle(removingAccount) }}”？钥匙串中保存的密码会一并删除。</p>
            <div class="modal-actions">
              <button type="button" class="button" :disabled="removingAccountBusy" @click="cancelRemoveAccount">取消</button>
              <button
                ref="confirmRemoveButton"
                type="button"
                class="button destructive"
                :disabled="removingAccountBusy"
                @click="confirmRemoveAccount"
              >
                {{ removingAccountBusy ? '正在删除…' : '删除' }}
              </button>
            </div>
          </div>

          <p v-if="accountError" class="inline-error" role="alert"><Icon name="alert" :size="14" />{{ accountError }}</p>

          <div class="modal-actions">
            <button type="button" class="button primary" @click="closeAccountManager">完成</button>
          </div>
        </template>

        <form v-else class="account-editor" novalidate @submit.prevent="saveAccount">
          <h2 id="account-manager-title">{{ editingAccountID ? '编辑账号' : '添加账号' }}</h2>
          <p class="df-secondary">密码使用 Windows 账户密码（不是 PIN）；Microsoft 账户请填写完整邮箱。密码仅保存在本机钥匙串。</p>
          <label class="field-label">
            备注名（可选，例如“工作账号”）
            <input v-model="editorName" class="field" autocomplete="off" placeholder="留空则显示用户名" />
          </label>
          <label class="field-label">
            用户名（Windows 用户名或 Microsoft 邮箱）
            <input v-model="editorUsername" class="field" autocomplete="username" required placeholder="例如 name@example.com" />
          </label>
          <label class="field-label">
            {{ editingAccountID ? '密码（编辑时必须重新输入）' : '密码' }}
            <input v-model="editorPassword" class="field" type="password" autocomplete="new-password" required />
          </label>
          <p v-if="editingAccountID" class="df-caption df-secondary">编辑账号需要重新输入密码，保存后会替换钥匙串中的旧密码。</p>
          <p v-if="accountError" class="inline-error" role="alert"><Icon name="alert" :size="14" />{{ accountError }}</p>
          <div class="modal-actions">
            <button type="button" class="button" @click="cancelAccountEditor">取消</button>
            <button type="submit" class="button primary" :disabled="savingAccount">{{ savingAccount ? '保存中…' : '保存' }}</button>
          </div>
        </form>
      </div>
    </div>
  </section>
</template>

<style scoped>
.catalog {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  color: var(--text);
}
.catalog-head { flex: none; display: flex; align-items: center; gap: 12px; padding: 20px; }
.head-text { display: grid; gap: 4px; min-width: 0; }
.refresh-button {
  width: 32px;
  height: 32px;
  flex: none;
  display: grid;
  place-items: center;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  color: var(--text);
}
.refresh-button:hover:not(:disabled) { background: var(--control); }

.catalog-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 20px;
  display: grid;
  align-content: start;
  gap: 14px;
}
.toolbar { display: flex; align-items: center; gap: 10px; min-width: 0; }
.account { position: relative; min-width: 0; }
.account-button {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  max-width: 230px;
  padding: 0 6px;
  border: 0;
  border-radius: 6px;
  background: none;
  color: var(--accent);
  font-size: 13px;
  font-weight: 500;
}
.account-button:hover { background: color-mix(in srgb, var(--accent) 8%, transparent); }
.account-title { min-width: 0; }
.account-menu {
  position: absolute;
  left: 0;
  top: calc(100% + 4px);
  z-index: 40;
  min-width: 220px;
  max-width: 320px;
  padding: 4px;
  display: grid;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.22);
}
.menu-item {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  border: 0;
  border-radius: 6px;
  background: none;
  color: var(--text);
  font-size: 13px;
  text-align: left;
}
.menu-item:hover { background: var(--control); }
.menu-check { width: 14px; flex: none; display: grid; place-items: center; color: var(--accent); }
.menu-empty { padding: 6px 8px; }
.scope { display: inline-flex; flex: none; gap: 2px; padding: 2px; border-radius: 8px; background: var(--control); }
.scope > button { height: 22px; padding: 0 10px; border: 0; border-radius: 6px; background: transparent; color: var(--text); font-size: 12px; }
.scope > button[aria-pressed='true'] { background: var(--accent); color: #fff; }
.catalog-search {
  flex: 0 1 200px;
  min-width: 120px;
  display: flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--surface);
}
.catalog-search.focused { border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 22%, transparent); }
.catalog-search input {
  flex: 1;
  min-width: 0;
  height: 22px;
  min-height: 0;
  padding: 0;
  border: 0;
  background: none;
  outline: 0;
  font-size: 12px;
}
.clear-search { width: 16px; height: 16px; flex: none; display: grid; place-items: center; padding: 0; color: var(--secondary); }

.manual-account {
  display: grid;
  gap: 10px;
  padding: 14px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
}
.manual-fields { display: flex; flex-wrap: wrap; gap: 10px; }
.manual-fields .field-label { flex: 1 1 220px; min-width: 0; }
.manual-save { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; }
.save-option { display: inline-flex; align-items: center; gap: 6px; flex: none; font-size: 12px; color: var(--text); }
.manual-name { display: inline-flex; align-items: center; gap: 6px; flex: 1 1 220px; min-width: 0; font-size: 12px; color: var(--secondary); }
.manual-name .field { flex: 1; min-width: 120px; }
.inline-error { display: flex; align-items: flex-start; gap: 6px; margin: 0; font-size: 12px; color: var(--danger); }

.applications { display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: 12px; }
.tile-wrap { position: relative; min-width: 0; }
.tile {
  width: 100%;
  min-height: 86px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  padding: 0 12px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
  color: var(--text);
}
.tile:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 4%, var(--surface)); }
.tile.mac { min-height: 120px; border-color: transparent; background: transparent; padding: 8px 4px; }
.tile.mac:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 6%, transparent); }
.tile-app-icon { width: 64px; height: 64px; object-fit: contain; }
.tile-name.single { -webkit-line-clamp: 1; max-width: 100%; }
.tile-status { display: inline-flex; align-items: center; gap: 4px; min-height: 13px; font-size: 11px; color: var(--secondary); }
.tile-status i { width: 5px; height: 5px; border-radius: 50%; background: var(--secondary); }
.tile-status i.open { background: var(--accent); }
.categories { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.category-chips { display: flex; align-items: center; gap: 5px; overflow-x: auto; min-width: 0; }
.category-chips button { height: 22px; padding: 0 10px; border: 1px solid var(--line); border-radius: 999px; background: transparent; color: var(--secondary); font-size: 11px; white-space: nowrap; flex: none; }
.category-chips button.on { background: var(--accent); border-color: transparent; color: #fff; }
.virtual-display { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--secondary); flex: none; }
.catalog-foot { display: flex; justify-content: space-between; margin: 0; }
.tile-icon {
  width: 42px;
  height: 42px;
  display: grid;
  place-items: center;
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
}
.tile-name {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  max-width: 84px;
  font-size: 12px;
  line-height: 1.2;
  text-align: center;
  overflow-wrap: anywhere;
}
.favorite {
  position: absolute;
  top: 6px;
  right: 6px;
  width: 22px;
  height: 22px;
  display: grid;
  place-items: center;
  padding: 0;
  border: 0;
  background: none;
  color: var(--secondary);
}
.favorite:hover { color: var(--accent); }
.favorite.on { color: var(--accent); }

.system-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0;
  border: 0;
  background: none;
  color: var(--text);
  font-size: 12px;
  font-weight: 500;
  text-align: left;
}
.system-count { color: var(--secondary); font-weight: 400; }
.footnote { margin: 0; }

.state-wrap { flex: 1 1 auto; min-height: 0; display: grid; place-items: center; padding: 32px; }
.state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; text-align: center; color: var(--secondary); }
.state.inline { min-height: 220px; grid-column: 1 / -1; }
.state h2 { margin: 0; font-size: 15px; color: var(--text); }
.state p { margin: 0; max-width: 420px; font-size: 12px; }
.selectable { user-select: text; overflow-wrap: anywhere; }
.spinner { width: 18px; height: 18px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.empty-star { color: var(--secondary); }

.field-label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--secondary); }
.modal h2 { color: var(--text); }
.modal p { color: var(--secondary); font-size: 12px; }
.account-manager { width: min(520px, 100%); display: grid; gap: 14px; }
.account-manager .modal-actions { margin-top: 0; }
.manager-head { display: flex; align-items: center; gap: 10px; }
.manager-head h2 { margin: 0; }
.manager-intro { margin: 0; font-size: 12px; }
.manager-empty {
  display: grid;
  justify-items: center;
  gap: 8px;
  padding: 24px 12px;
  border: 1px dashed var(--line);
  border-radius: 10px;
  color: var(--secondary);
  text-align: center;
}
.manager-empty p { margin: 0; font-size: 12px; max-width: 360px; }
.account-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 280px;
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: 10px;
}
.account-row { display: flex; align-items: center; gap: 10px; padding: 8px 12px; }
.account-row + .account-row { border-top: 0.5px solid var(--line); }
.account-row > svg { color: var(--secondary); }
.account-row-text { flex: 1; min-width: 0; display: grid; gap: 2px; }
.account-row-name { font-size: 13px; color: var(--text); }
.account-row-user { font-size: 11px; }
.account-row .icon-button { flex: none; color: var(--secondary); }
.account-row .icon-button:hover { color: var(--text); }
.account-row .icon-button.danger:hover { color: var(--danger); }
.remove-confirm {
  display: grid;
  gap: 8px;
  padding: 12px;
  border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
  border-radius: 10px;
  background: color-mix(in srgb, var(--danger) 7%, var(--surface));
}
.account-manager .remove-confirm p { margin: 0; font-size: 12px; color: var(--text); overflow-wrap: anywhere; }
.account-editor { display: grid; gap: 12px; }
.button.destructive { color: #fff; background: var(--danger); border-color: transparent; }
.button.destructive:hover { background: color-mix(in srgb, var(--danger) 85%, #000); }
@media (prefers-reduced-motion: reduce) {
  .spinner { animation: none; }
}
</style>

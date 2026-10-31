import { computed, ref } from 'vue'
import { backend } from '../../services/backend'
import { get, keyOf, type Key } from './util'

export interface LibraryApp { bundle_id: string; name: string }

export interface LibrarySettings {
  retention_days: number
  excluded_apps: LibraryApp[]
  shortcut?: { enabled: boolean; key_code: number; modifiers: number } | null
  [key: string]: unknown
}

export const DEFAULT_CLIPBOARD_SHORTCUT: Key = { key_code: 9, modifiers: 0x180000 }

// 剪贴板历史设置与唤起快捷键共用同一份引擎设置，模块级共享。
const settings = ref<LibrarySettings | null>(null)
const apps = ref<LibraryApp[]>([])

const shortcutKey = computed<Key | null>(() => {
  const shortcut = get(settings.value, 'shortcut')
  return shortcut && get(shortcut, 'enabled') !== false ? keyOf(shortcut) : null
})

/** `op: "state"` 返回引擎当前设置与可选应用列表。 */
async function load(): Promise<LibrarySettings> {
  const state = await backend.queryLibrary({ op: 'state' })
  const value = get<LibrarySettings>(state, 'settings')
  if (!value) throw new Error('引擎没有返回剪贴板设置')
  settings.value = value
  apps.value = get<LibraryApp[]>(state, 'apps') ?? []
  return value
}

async function save(next: LibrarySettings): Promise<void> {
  const response = await backend.mutateLibrary({ op: 'settings', settings: next })
  settings.value = get<LibrarySettings>(response, 'settings') ?? next
}

async function clearRemoteIndex(): Promise<void> {
  await backend.mutateLibrary({ op: 'clear_remote_index' })
}

export function useClipboardSettings() {
  return { settings, apps, shortcutKey, load, save, clearRemoteIndex }
}

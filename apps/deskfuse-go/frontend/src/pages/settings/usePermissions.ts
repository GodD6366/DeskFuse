import { computed, onBeforeUnmount, onMounted, ref, type Ref } from 'vue'
import { backend } from '../../services/backend'
import { errorText, get, type Snapshot } from './util'

export interface PermissionItem {
  id: string
  title: string
  purpose: string
  granted: boolean
  requires_relaunch: boolean
}

/** `accessibility`、`Accessibility`、`screenRecording` 统一成 snake_case。 */
export const permissionId = (value: string): string => value.replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase()

/**
 * 权限状态只读自原生快照 `permissions.items`；授权与打开系统设置走 `permissions.open`。
 * 读取不会触发系统弹窗；动作失败（含 unsupported_action）如实展示，不当作已授权。
 */
export function usePermissions(snapshot: Ref<Snapshot>) {
  const error = ref<string | null>(null)
  const busy = ref<string | null>(null)

  const items = computed<PermissionItem[]>(() =>
    (get<any[]>(get(snapshot.value, 'permissions'), 'items') ?? []).map((item) => ({
      id: permissionId(String(get(item, 'id') ?? '')),
      title: String(get(item, 'title') ?? get(item, 'id') ?? ''),
      purpose: String(get(item, 'purpose') ?? ''),
      granted: get(item, 'granted') === true,
      requires_relaunch: get(item, 'requires_relaunch') === true,
    })))

  const pendingRelaunch = computed<string[]>(() =>
    (get<string[]>(get(snapshot.value, 'permissions'), 'pending_relaunch') ?? []).map(permissionId))

  const find = (id: string) => items.value.find((item) => item.id === id)
  const isGranted = (id: string) => find(id)?.granted ?? false
  const titleOf = (id: string) => find(id)?.title ?? id

  async function open(id: string, mode: 'authorize' | 'settings') {
    busy.value = id
    error.value = null
    try {
      await backend.invoke('permissions.open', { permission: id, mode })
    } catch (e) {
      error.value = errorText(e)
    } finally {
      busy.value = null
    }
  }

  /** 已授权则打开系统设置，否则走系统授权流程。 */
  const act = (id: string) => open(id, isGranted(id) ? 'settings' : 'authorize')

  async function relaunch() {
    error.value = null
    try {
      await backend.invoke('permission.relaunch', {})
    } catch (e) {
      error.value = errorText(e)
    }
  }

  async function refresh() {
    try {
      await backend.invoke('refresh_permissions', {})
      error.value = null
    } catch (e) {
      error.value = errorText(e)
    }
  }

  // 回到应用后静默重新读取：授权发生在系统设置里；手动刷新才展示失败。
  const quietRefresh = () => { backend.invoke('refresh_permissions', {}).catch(() => undefined) }
  onMounted(() => window.addEventListener('focus', quietRefresh))
  onBeforeUnmount(() => window.removeEventListener('focus', quietRefresh))

  return { items, pendingRelaunch, find, isGranted, titleOf, open, act, relaunch, refresh, error, busy }
}

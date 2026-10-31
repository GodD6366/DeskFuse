import { computed, reactive, watch } from 'vue'
import { backend } from '../../services/backend'

/**
 * 来源应用的图标与显示名，对应原生 ClipboardAppInfoCache：
 * 结果按 bundle id 缓存，取不到时缓存空结果，让界面稳定回退到类型图标。
 */
export interface AppInfo {
  icon: string | null
  name: string | null
}

const cache = reactive<Record<string, AppInfo>>({})
const pending = new Set<string>()

function fetchAppInfo(bundleID: string | null | undefined): void {
  const id = bundleID
  if (!id || id in cache || pending.has(id)) return
  pending.add(id)
  backend.invoke('clipboard.appInfo', { bundle_id: id })
    .then((raw) => {
      cache[id] = {
        icon: typeof raw?.icon === 'string' && raw.icon ? raw.icon : null,
        name: typeof raw?.name === 'string' && raw.name ? raw.name : null,
      }
    })
    .catch(() => {
      cache[id] = { icon: null, name: null }
    })
    .finally(() => { pending.delete(id) })
}

/** 组件传入 bundle id getter；图标或名称到达后响应式更新，缺失时返回 null。 */
export function useAppInfo(bundleID: () => string | null | undefined) {
  watch(bundleID, (id) => { fetchAppInfo(id) }, { immediate: true })
  return computed<AppInfo | null>(() => {
    const id = bundleID()
    return id ? cache[id] ?? null : null
  })
}

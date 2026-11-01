import { ref, watch, type Ref } from 'vue'
import { backend } from '../../services/backend'

export type DeviceArtTarget = { name: string; model?: string }

// 宿主用 CoreTypes 里的同一份产品图（找不到时用同形 SF Symbol）栅格化成 PNG。
// 同名设备的图标在会话内只解析一次，失败也缓存，避免每次快照刷新都重试。
const cache = new Map<string, string | undefined>()

export function useDeviceArt(target: Ref<DeviceArtTarget>, pixels = 128): Ref<string | undefined> {
  const src = ref<string>()
  let ticket = 0
  watch(
    target,
    async (value) => {
      const name = String(value?.name ?? '').trim()
      if (!name) {
        src.value = undefined
        return
      }
      const model = String(value?.model ?? '').trim()
      const key = `${name}|${model}|${pixels}`
      if (cache.has(key)) {
        src.value = cache.get(key)
        return
      }
      const mine = ++ticket
      try {
        const result = await backend.invoke('devices.icon', { name, model_identifier: model, size: pixels })
        const png = result?.available === true && typeof result.png_base64 === 'string' ? result.png_base64 : ''
        const url = png ? `data:image/png;base64,${png}` : undefined
        cache.set(key, url)
        if (mine === ticket) src.value = url
      } catch {
        // 没有 devices.icon 的宿主（Windows 适配层）保留名称轮廓，不阻塞页面。
        cache.set(key, undefined)
        if (mine === ticket) src.value = undefined
      }
    },
    { immediate: true },
  )
  return src
}

import { computed, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import type { LibraryItem } from '../../types/clipboard'

/**
 * 图片缩略图地址由宿主同步给出（deskfuse://blob/thumbnail/{id}?size=..），
 * 图片本身由浏览器按需加载；加载失败如实标记，不伪造占位数据。
 * 单文件图片也能取到缩略图，其余文件条目失败后回退到类型图标。
 */
export function useThumbnail(item: () => LibraryItem, size: number) {
  const failed = ref(false)
  const loaded = ref(false)
  const src = computed(() => {
    const current = item()
    if (!current.has_image && current.file_count <= 0) return null
    return backend.thumbnailURL(current.id, size)
  })
  watch(() => `${item().id}-${item().copied_at_ms}`, () => { failed.value = false; loaded.value = false })
  return {
    src,
    failed,
    loaded,
    onLoad: () => { loaded.value = true; failed.value = false },
    onError: () => { failed.value = true },
  }
}

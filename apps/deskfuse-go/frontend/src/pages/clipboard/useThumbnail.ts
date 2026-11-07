import { computed, onScopeDispose, ref, watch } from 'vue'
import { backend } from '../../services/backend'
import type { LibraryItem } from '../../types/clipboard'

/** 远端缩略图到达前的探测节奏：先密后疏，约一分钟后才显示“待获取”。 */
const FAST_RETRIES = 20
const FAST_DELAY_MS = 500
const SLOW_DELAY_MS = 2000
const MAX_RETRIES = 40

/**
 * 图片缩略图地址由宿主同步给出（deskfuse://blob/thumbnail/{id}?size=..），
 * 图片本身由浏览器按需加载；加载失败如实标记，不伪造占位数据。
 * 单文件图片也能取到缩略图，其余文件条目失败后回退到类型图标。
 *
 * 远端行的原图可能很大：宿主会先向来源设备要一张小缩略图。这里只在后台
 * 探测缩略图地址，拿到之前不渲染 <img>（避免破图图标），保持加载态。
 */
export function useThumbnail(item: () => LibraryItem, size: number) {
  const failed = ref(false)
  const loaded = ref(false)
  const waiting = ref(false)
  const attempt = ref(0)
  let timer: number | undefined
  let probe: HTMLImageElement | null = null

  const url = computed(() => {
    const current = item()
    if (!current.has_image && current.file_count <= 0) return null
    const base = backend.thumbnailURL(current.id, size)
    return attempt.value ? `${base}&retry=${attempt.value}` : base
  })
  const src = computed(() => (waiting.value ? null : url.value))
  const loading = computed(() => waiting.value || (!!src.value && !loaded.value && !failed.value))

  function stop() {
    window.clearTimeout(timer)
    timer = undefined
    if (probe) { probe.onload = null; probe.onerror = null; probe = null }
  }
  function reset() {
    stop()
    attempt.value = 0
    waiting.value = false
    failed.value = false
    loaded.value = false
  }
  watch(() => `${item().id}-${item().copied_at_ms}-${item().available}`, reset)
  onScopeDispose(stop)

  function schedule() {
    timer = window.setTimeout(() => {
      attempt.value += 1
      const next = url.value
      if (!next) { waiting.value = false; failed.value = true; return }
      probe = new Image()
      probe.onload = () => { stop(); waiting.value = false }
      probe.onerror = () => {
        probe = null
        if (attempt.value >= MAX_RETRIES) { waiting.value = false; failed.value = true; return }
        schedule()
      }
      probe.src = next
    }, attempt.value < FAST_RETRIES ? FAST_DELAY_MS : SLOW_DELAY_MS)
  }

  function onError() {
    const current = item()
    const pending = current.remote && !current.available && current.has_image
    if (!pending || attempt.value >= MAX_RETRIES) {
      failed.value = true
      return
    }
    waiting.value = true
    stop()
    schedule()
  }

  return {
    src,
    failed,
    loaded,
    loading,
    onLoad: () => { loaded.value = true; failed.value = false },
    onError,
  }
}

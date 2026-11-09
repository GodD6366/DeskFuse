import { onBeforeUnmount, watch, type Ref } from 'vue'

/** 弹层打开时，点击 root 之外或按 Esc 即关闭；root 应同时包含触发按钮，避免“点触发器”被当作外部点击。 */
export function useDismiss(open: Ref<boolean>, root: Ref<HTMLElement | null>, close: () => void) {
  const onPointer = (event: PointerEvent) => {
    if (root.value && !root.value.contains(event.target as Node)) close()
  }
  const onKey = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return
    event.stopPropagation()
    close()
  }
  const stop = () => {
    document.removeEventListener('pointerdown', onPointer, true)
    document.removeEventListener('keydown', onKey, true)
  }
  watch(open, (value) => {
    stop()
    if (!value) return
    document.addEventListener('pointerdown', onPointer, true)
    document.addEventListener('keydown', onKey, true)
  }, { immediate: true })
  onBeforeUnmount(stop)
}

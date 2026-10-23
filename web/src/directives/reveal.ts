import type { Directive, DirectiveBinding } from 'vue'

const observers = new WeakMap<HTMLElement, IntersectionObserver>()

/**
 * 滚动进入视口时揭示内容。
 * - v-reveal：淡入并上移
 * - v-reveal.title：标题遮罩展开
 * - v-reveal="120"：设置延迟，单位毫秒
 *
 * 标题本身用 clip-path 隐藏，clip-path 也会把它的
 * IntersectionObserver 交叉面积裁成 0，所以标题改为观察父元素。
 */
export const revealDirective: Directive<HTMLElement, number | undefined> = {
  mounted(el: HTMLElement, binding: DirectiveBinding<number | undefined>) {
    const isTitle = Boolean(binding.modifiers.title)
    el.classList.add(isTitle ? 'reveal-title' : 'reveal')
    if (typeof binding.value === 'number') {
      el.style.setProperty('--reveal-delay', `${binding.value}ms`)
    }

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduced || typeof IntersectionObserver === 'undefined') {
      el.classList.add('is-visible')
      return
    }

    const observed = isTitle ? (el.parentElement ?? el) : el
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          el.classList.add('is-visible')
          observer.disconnect()
          observers.delete(el)
        }
      },
      { threshold: 0.12, rootMargin: '0px 0px -8% 0px' },
    )
    observer.observe(observed)
    observers.set(el, observer)
  },
  unmounted(el: HTMLElement) {
    observers.get(el)?.disconnect()
    observers.delete(el)
  },
}

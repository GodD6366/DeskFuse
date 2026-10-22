import type { Directive } from 'vue'

let observer: IntersectionObserver | null = null
if (typeof IntersectionObserver !== 'undefined') {
  observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible')
          observer?.unobserve(entry.target)
        }
      })
    },
    { threshold: 0.12, rootMargin: '0px 0px -40px 0px' },
  )
}

export const revealDirective: Directive = {
  mounted(el: HTMLElement) {
    el.classList.add('reveal')
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (!observer || reduced) {
      el.classList.add('visible')
      return
    }
    observer.observe(el)
  },
}

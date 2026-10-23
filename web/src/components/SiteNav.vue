<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { asset } from '../modules/asset'
import { ARM64_DISK_IMAGE, REPOSITORY_URL, macAppDownloadUrl } from '../config/distribution'
import { FALLBACK_RELEASE_TAG, useLatestRelease } from '../composables/useLatestRelease'

const links = [
  { id: 'demo', label: '现场' },
  { id: 'abilities', label: '能力' },
  { id: 'architecture', label: '架构' },
  { id: 'compare', label: '对比' },
  { id: 'install', label: '安装' },
]

const { release } = useLatestRelease()
const downloadUrl = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const versionLabel = computed(() => release.value?.tag ?? FALLBACK_RELEASE_TAG)

const scrolled = ref(false)
const menuOpen = ref(false)
const activeId = ref('')
const menuButton = ref<HTMLButtonElement>()
const closeButton = ref<HTMLButtonElement>()
const panel = ref<HTMLElement>()

let sectionObserver: IntersectionObserver | null = null
let lockedScrollY = 0

const onScroll = () => {
  scrolled.value = window.scrollY > 10
}

function lockBody(lock: boolean) {
  if (lock) {
    lockedScrollY = window.scrollY
    document.body.style.position = 'fixed'
    document.body.style.top = `-${lockedScrollY}px`
    document.body.style.left = '0'
    document.body.style.right = '0'
    document.body.style.overflow = 'hidden'
  } else {
    document.body.style.position = ''
    document.body.style.top = ''
    document.body.style.left = ''
    document.body.style.right = ''
    document.body.style.overflow = ''
    window.scrollTo(0, lockedScrollY)
  }
}

async function openMenu() {
  menuOpen.value = true
  lockBody(true)
  await nextTick()
  closeButton.value?.focus()
}

function closeMenu(returnFocus = true) {
  if (!menuOpen.value) return
  menuOpen.value = false
  lockBody(false)
  if (returnFocus) {
    nextTick(() => menuButton.value?.focus())
  }
}

function onKeydown(event: KeyboardEvent) {
  if (!menuOpen.value) return
  if (event.key === 'Escape') {
    event.preventDefault()
    closeMenu()
    return
  }
  if (event.key !== 'Tab' || !panel.value) return

  const focusable = Array.from(
    panel.value.querySelectorAll<HTMLElement>('a[href], button:not([disabled])'),
  ).filter((el) => el.offsetParent !== null)
  if (focusable.length === 0) return
  const first = focusable[0]
  const last = focusable[focusable.length - 1]
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}

onMounted(() => {
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })
  document.addEventListener('keydown', onKeydown)

  if ('IntersectionObserver' in window) {
    sectionObserver = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) activeId.value = entry.target.id
        })
      },
      { rootMargin: '-24% 0px -66% 0px', threshold: 0 },
    )
    links.forEach(({ id }) => {
      const section = document.getElementById(id)
      if (section) sectionObserver?.observe(section)
    })
    const top = document.getElementById('top')
    if (top) {
      const topObserver = new IntersectionObserver(
        (entries) => {
          if (entries[0]?.isIntersecting) activeId.value = ''
        },
        { rootMargin: '-10% 0px -70% 0px' },
      )
      topObserver.observe(top)
    }
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('scroll', onScroll)
  document.removeEventListener('keydown', onKeydown)
  sectionObserver?.disconnect()
  if (menuOpen.value) lockBody(false)
})
</script>

<template>
  <header class="nav" :class="{ 'is-scrolled': scrolled }">
    <div class="nav__inner">
      <a class="brand" href="#top" aria-label="DeskFuse 首页">
        <img :src="asset('assets/icon.png')" alt="" class="brand__icon" width="26" height="26" />
        <span class="brand__name">DeskFuse</span>
      </a>

      <nav class="nav__links" aria-label="页面导航">
        <a
          v-for="link in links"
          :key="link.id"
          class="nav__link"
          :class="{ 'is-active': activeId === link.id }"
          :href="`#${link.id}`"
          :aria-current="activeId === link.id ? 'location' : undefined"
        >
          {{ link.label }}
        </a>
      </nav>

      <div class="nav__actions">
        <span v-if="versionLabel" class="nav__version">{{ versionLabel }}</span>
        <a
          class="nav__github"
          :href="REPOSITORY_URL"
          target="_blank"
          rel="noopener"
          aria-label="在 GitHub 查看 DeskFuse"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true">
            <path
              d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z"
            />
          </svg>
        </a>
        <a class="btn btn--small nav__cta" :href="downloadUrl" aria-label="下载 DeskFuse macOS 版">下载</a>
        <button
          ref="menuButton"
          class="menu-toggle"
          type="button"
          aria-label="打开页面目录"
          :aria-expanded="menuOpen"
          aria-controls="mobile-menu"
          aria-haspopup="dialog"
          @click="openMenu"
        >
          <span class="menu-toggle__bars" aria-hidden="true"><i></i><i></i></span>
          目录
        </button>
      </div>
    </div>
  </header>

  <Teleport to="body">
    <div
      v-if="menuOpen"
      id="mobile-menu"
      ref="panel"
      class="menu"
      role="dialog"
      aria-modal="true"
      aria-label="页面目录"
    >
      <div class="menu__top">
        <span class="menu__brand">DeskFuse</span>
        <button ref="closeButton" class="menu__close" type="button" @click="closeMenu()">
          关闭
          <span aria-hidden="true">×</span>
        </button>
      </div>

      <nav class="menu__links" aria-label="移动端页面导航">
        <a
          v-for="(link, index) in links"
          :key="link.id"
          class="menu__link"
          :href="`#${link.id}`"
          @click="closeMenu(false)"
        >
          <span class="menu__index">{{ String(index + 1).padStart(2, '0') }}</span>
          <span>{{ link.label }}</span>
        </a>
      </nav>

      <div class="menu__foot">
        <a class="btn" :href="downloadUrl" @click="closeMenu(false)">下载 macOS 版</a>
        <div class="menu__meta">
          <span v-if="versionLabel">{{ versionLabel }}</span>
          <a :href="REPOSITORY_URL" target="_blank" rel="noopener" @click="closeMenu(false)">GitHub</a>
          <a :href="`${REPOSITORY_URL}/issues`" target="_blank" rel="noopener" @click="closeMenu(false)">
            反馈问题
          </a>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.nav {
  position: fixed;
  inset: 0 0 auto;
  z-index: 100;
  height: var(--nav-h);
  border-bottom: 1px solid transparent;
  background: rgba(6, 16, 14, 0.56);
  backdrop-filter: blur(18px) saturate(160%);
  -webkit-backdrop-filter: blur(18px) saturate(160%);
  transition:
    background-color 0.3s ease,
    border-color 0.3s ease;
}

.nav.is-scrolled {
  background: rgba(6, 16, 14, 0.88);
  border-bottom-color: var(--line);
}

.nav__inner {
  display: flex;
  align-items: center;
  gap: 28px;
  width: min(100% - var(--gutter) * 2, var(--page-max));
  height: 100%;
  margin-inline: auto;
}

.brand {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  flex: none;
}

.brand__icon {
  width: 26px;
  height: 26px;
  border-radius: 7px;
}

.brand__name {
  font-family: var(--display);
  font-size: 21px;
  line-height: 1;
  letter-spacing: -0.01em;
}

.nav__links {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-inline: auto;
}

.nav__link {
  position: relative;
  padding: 8px 12px;
  color: var(--muted);
  font-size: 13.5px;
  transition: color 0.2s ease;
}

.nav__link::before {
  content: "";
  position: absolute;
  left: 50%;
  bottom: 1px;
  width: 3px;
  height: 3px;
  background: var(--orange);
  transform: translateX(-50%) scale(0);
  transition: transform 0.22s var(--ease-out);
}

.nav__link:hover {
  color: var(--ink);
}

.nav__link.is-active {
  color: var(--aqua);
}

.nav__link.is-active::before {
  transform: translateX(-50%) scale(1);
}

.nav__actions {
  display: flex;
  align-items: center;
  gap: 16px;
  flex: none;
}

.nav__version {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 10.5px;
  letter-spacing: 0.04em;
}

.nav__github {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  color: var(--muted);
  transition: color 0.2s ease;
}

.nav__github:hover {
  color: var(--aqua);
}

.nav__cta {
  min-height: 34px;
  padding-inline: 16px;
  font-size: 13px;
}

.menu-toggle {
  display: none;
  align-items: center;
  gap: 8px;
  height: 36px;
  padding-inline: 4px;
  color: var(--ink);
  font-size: 13px;
  cursor: pointer;
}

.menu-toggle__bars {
  display: grid;
  gap: 5px;
  width: 17px;
}

.menu-toggle__bars i {
  display: block;
  height: 1px;
  background: currentColor;
}

.menu-toggle__bars i:last-child {
  width: 11px;
  margin-left: auto;
}

/* mobile dialog */

.menu {
  position: fixed;
  inset: 0;
  z-index: 150;
  display: flex;
  flex-direction: column;
  padding: 20px var(--gutter) 32px;
  background:
    radial-gradient(circle at 88% 10%, rgba(89, 243, 218, 0.12), transparent 33%),
    var(--bg-deep);
  color: var(--ink);
  animation: menu-in 0.28s var(--ease-out) both;
}

.menu__top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 44px;
  padding-bottom: 18px;
  border-bottom: 1px solid var(--line);
}

.menu__brand {
  font-family: var(--display);
  font-size: 21px;
}

.menu__close {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 36px;
  color: var(--muted);
  font-size: 13px;
  cursor: pointer;
}

.menu__close span {
  font-size: 21px;
  line-height: 1;
  color: var(--ink);
}

.menu__links {
  display: flex;
  flex-direction: column;
  margin-top: 28px;
}

.menu__link {
  display: flex;
  align-items: baseline;
  gap: 18px;
  padding: 11px 0;
  border-bottom: 1px solid var(--line);
  font-family: var(--display);
  font-size: clamp(40px, 12vw, 64px);
  line-height: 1.02;
}

.menu__link:hover,
.menu__link:focus-visible {
  color: var(--aqua);
}

.menu__index {
  color: var(--orange);
  font-family: var(--mono);
  font-size: 10px;
  letter-spacing: 0.06em;
}

.menu__foot {
  display: flex;
  flex-direction: column;
  gap: 22px;
  margin-top: auto;
  padding-top: 28px;
}

.menu__foot .btn {
  width: 100%;
}

.menu__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  color: var(--muted);
  font-family: var(--mono);
  font-size: 11px;
}

.menu__meta a {
  border-bottom: 1px solid var(--line);
}

.menu__meta a:hover {
  color: var(--aqua);
}

@keyframes menu-in {
  from {
    opacity: 0;
    transform: translateY(-10px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

@media (max-width: 1080px) {
  .nav__version {
    display: none;
  }
}

@media (max-width: 900px) {
  .nav__inner {
    gap: 16px;
  }

  .nav__links,
  .nav__github {
    display: none;
  }

  .nav__actions {
    margin-left: auto;
    gap: 14px;
  }

  .menu-toggle {
    display: inline-flex;
  }
}

@media (prefers-reduced-motion: reduce) {
  .menu {
    animation: none;
  }
}
</style>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import GlyphIcon from './GlyphIcon.vue'
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'

const links = [
  { label: '能力', href: '#capabilities' },
  { label: '对比', href: '#compare' },
  { label: '下载', href: '#download' },
  { label: '帮助', href: '#help' },
]

const root = ref<HTMLElement | null>(null)
const open = ref(false)

function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !open.value) return
  open.value = false
  root.value?.querySelector<HTMLButtonElement>('.nav__menu-button')?.focus()
}

function onPointerDown(event: PointerEvent) {
  if (open.value && event.target instanceof Node && !root.value?.contains(event.target)) open.value = false
}

onMounted(() => {
  document.addEventListener('keydown', onKeydown)
  document.addEventListener('pointerdown', onPointerDown)
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', onKeydown)
  document.removeEventListener('pointerdown', onPointerDown)
})
</script>

<template>
  <header ref="root" class="nav">
    <div class="page nav__inner">
      <a class="nav__brand" href="#top" aria-label="DeskFuse 首页" @click="open = false">
        <img :src="asset('assets/icon.png')" alt="" width="22" height="22" />
        <span>DeskFuse</span>
      </a>

      <nav class="nav__links" aria-label="页面导航">
        <a v-for="link in links" :key="link.href" :href="link.href">{{ link.label }}</a>
      </nav>

      <div class="nav__right">
        <a
          class="nav__github"
          :href="REPOSITORY_URL"
          target="_blank"
          rel="noopener"
          aria-label="在 GitHub 查看 DeskFuse"
        >
          <GlyphIcon name="github" :size="18" />
        </a>
        <a class="nav__download" href="#download" @click="open = false">下载</a>
        <button
          class="nav__menu-button"
          type="button"
          :aria-expanded="open"
          aria-controls="mobile-navigation"
          :aria-label="open ? '关闭导航' : '打开导航'"
          @click="open = !open"
        >
          <span :class="{ 'is-open': open }" aria-hidden="true"><i></i><i></i></span>
        </button>
      </div>
    </div>

    <nav v-show="open" id="mobile-navigation" class="nav__mobile" aria-label="移动端页面导航">
      <a v-for="link in links" :key="link.href" :href="link.href" @click="open = false">
        {{ link.label }}<span aria-hidden="true">›</span>
      </a>
      <a :href="REPOSITORY_URL" target="_blank" rel="noopener" @click="open = false">
        GitHub<span aria-hidden="true">↗</span>
      </a>
    </nav>
  </header>
</template>

<style scoped>
.nav {
  position: sticky;
  top: 0;
  z-index: 40;
  border-bottom: 1px solid rgb(0 0 0 / 7%);
  background: rgb(255 255 255 / 82%);
  backdrop-filter: saturate(180%) blur(20px);
  -webkit-backdrop-filter: saturate(180%) blur(20px);
}

.nav__inner {
  display: flex;
  align-items: center;
  height: 48px;
}

.nav__brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--ink);
  font-size: 1.0625rem;
  font-weight: 600;
  line-height: 1;
  text-decoration: none;
}

.nav__brand img {
  border-radius: 5px;
}

.nav__links {
  display: flex;
  align-items: center;
  gap: 34px;
  margin-left: auto;
  margin-right: 36px;
}

.nav__links a {
  display: inline-flex;
  align-items: center;
  min-height: 44px;
  color: #3a3a3c;
  font-size: 0.75rem;
  line-height: 1;
  text-decoration: none;
  transition: color 0.18s var(--ease);
}

.nav__links a:hover {
  color: var(--link);
}

.nav__right {
  display: flex;
  align-items: center;
  gap: 16px;
}

.nav__github {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 40px;
  color: var(--ink);
}

.nav__download {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 30px;
  padding: 0 14px;
  border-radius: 980px;
  background: var(--vermilion);
  color: #fff;
  font-size: 0.75rem;
  line-height: 1;
  text-decoration: none;
  transition: background-color 0.18s var(--ease);
}

.nav__download:hover {
  background: #0077ed;
}

.nav__menu-button,
.nav__mobile {
  display: none;
}

@media (max-width: 760px) {
  .nav__links {
    display: none;
  }

  .nav__right {
    margin-left: auto;
    gap: 10px;
  }

  .nav__menu-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 44px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--ink);
    cursor: pointer;
  }

  .nav__menu-button > span {
    position: relative;
    width: 18px;
    height: 14px;
  }

  .nav__menu-button i {
    position: absolute;
    left: 0;
    width: 18px;
    height: 1.5px;
    background: currentColor;
    font-style: normal;
    transition: transform 0.2s var(--ease);
  }

  .nav__menu-button i:first-child {
    top: 4px;
  }

  .nav__menu-button i:last-child {
    top: 10px;
  }

  .nav__menu-button .is-open i:first-child {
    transform: translateY(3px) rotate(45deg);
  }

  .nav__menu-button .is-open i:last-child {
    transform: translateY(-3px) rotate(-45deg);
  }

  .nav__mobile {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    display: grid;
    padding: 8px 24px 16px;
    border-bottom: 1px solid var(--rule);
    background: #fff;
    box-shadow: 0 16px 28px rgb(0 0 0 / 8%);
  }

  .nav__mobile a {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 46px;
    border-bottom: 1px solid #e8e8ed;
    color: var(--ink);
    font-size: 0.9375rem;
    text-decoration: none;
  }

  .nav__mobile a:last-child {
    border-bottom: 0;
  }

  .nav__mobile a span {
    color: var(--muted);
  }
}

@media (max-width: 420px) {
  .nav__github {
    display: none;
  }
}
</style>

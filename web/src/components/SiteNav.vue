<script setup lang="ts">
import GlyphIcon from './GlyphIcon.vue'
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'

const links = [
  { index: '01', label: '能力', href: '#capabilities' },
  { index: '02', label: '对比', href: '#compare' },
  { index: '03', label: '下载', href: '#download' },
  { index: '04', label: '帮助', href: '#help' },
]
</script>

<template>
  <header class="nav">
    <div class="page nav__inner">
      <a class="nav__brand" href="#top" aria-label="DeskFuse 首页">
        <img :src="asset('assets/icon.png')" alt="" width="24" height="24" />
        <span class="nav__wordmark">DeskFuse</span>
        <span class="nav__issue">ISSUE 01</span>
      </a>

      <nav class="nav__links" aria-label="页面导航">
        <a v-for="link in links" :key="link.href" :href="link.href">
          <span class="nav__index">{{ link.index }}</span>
          {{ link.label }}
        </a>
      </nav>

      <div class="nav__right">
        <a
          class="nav__github"
          :href="REPOSITORY_URL"
          target="_blank"
          rel="noopener"
          aria-label="在 GitHub 查看 DeskFuse"
        >
          <GlyphIcon name="github" :size="17" />
          <span>GitHub</span>
        </a>
        <a class="nav__download" href="#download">下载 <span aria-hidden="true">↓</span></a>
      </div>
    </div>

    <nav class="nav__mobile" aria-label="页面导航">
      <a v-for="link in links" :key="link.href" :href="link.href">
        <span>{{ link.index }}</span>{{ link.label }}
      </a>
    </nav>
  </header>
</template>

<style scoped>
.nav {
  position: sticky;
  top: 0;
  z-index: 40;
  border-bottom: 1px solid var(--rule-strong);
  background: var(--paper);
}

.nav__inner {
  display: flex;
  align-items: center;
  gap: 28px;
  height: 66px;
}

.nav__brand {
  display: flex;
  align-items: center;
  gap: 9px;
  color: var(--ink);
  text-decoration: none;
}

.nav__brand img {
  border-radius: 5px;
}

.nav__wordmark {
  font-family: var(--font-serif-en);
  font-size: 1.35rem;
  font-weight: 500;
  letter-spacing: 0;
  line-height: 1;
}

.nav__issue {
  margin-left: 8px;
  padding-left: 12px;
  border-left: 1px solid var(--rule);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
  line-height: 1.4;
}

.nav__links {
  display: flex;
  align-items: center;
  gap: 24px;
  margin-left: auto;
}

.nav__links a {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  padding: 5px 0;
  border-bottom: 1px solid transparent;
  color: var(--ink);
  font-size: 0.875rem;
  font-weight: 500;
  text-decoration: none;
  transition:
    color 0.18s var(--ease),
    border-color 0.18s var(--ease);
}

.nav__links a:hover {
  border-bottom-color: var(--vermilion);
  color: var(--vermilion);
}

.nav__index {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.04em;
}

.nav__right {
  display: flex;
  align-items: center;
  gap: 18px;
  margin-left: 8px;
}

.nav__github {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  letter-spacing: 0.02em;
  text-decoration: none;
}

.nav__github:hover {
  color: var(--ink);
}

.nav__download {
  padding-bottom: 2px;
  border-bottom: 1px solid var(--vermilion);
  color: var(--vermilion);
  font-size: 0.875rem;
  font-weight: 600;
  text-decoration: none;
}

.nav__download:hover {
  color: var(--vermilion-dark);
}

.nav__mobile {
  display: none;
}

@media (max-width: 960px) {
  .nav__issue,
  .nav__links {
    display: none;
  }

  .nav__inner {
    height: 58px;
  }

  .nav__right {
    margin-left: auto;
    gap: 14px;
  }

  .nav__mobile {
    display: flex;
    gap: 18px;
    overflow-x: auto;
    padding: 0 16px 9px;
    scrollbar-width: none;
  }

  .nav__mobile::-webkit-scrollbar {
    display: none;
  }

  .nav__mobile a {
    display: inline-flex;
    gap: 4px;
    color: var(--ink);
    font-size: 0.75rem;
    font-weight: 500;
    text-decoration: none;
    white-space: nowrap;
  }

  .nav__mobile a span {
    color: var(--vermilion);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
  }
}

@media (max-width: 520px) {
  .nav__github span {
    display: none;
  }

  .nav__wordmark {
    font-size: 1.2rem;
  }

  .nav__github {
    width: 30px;
    height: 30px;
    justify-content: center;
  }
}
</style>

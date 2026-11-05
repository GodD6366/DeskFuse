<script setup lang="ts">
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'

const groups = [
  {
    title: '产品',
    links: [
      { label: '能力', href: '#capabilities' },
      { label: '对比', href: '#compare' },
      { label: '下载', href: '#download' },
      { label: '常见问题', href: '#help' },
    ],
  },
  {
    title: '资源',
    links: [
      { label: '全部版本', href: `${REPOSITORY_URL}/releases` },
      { label: '更新记录', href: `${REPOSITORY_URL}/blob/main/CHANGELOG.md` },
      { label: '第三方声明', href: `${REPOSITORY_URL}/blob/main/THIRD_PARTY_NOTICES.md` },
      { label: '许可说明', href: `${REPOSITORY_URL}/blob/main/LICENSE` },
    ],
  },
  {
    title: '社区',
    links: [
      { label: 'GitHub', href: REPOSITORY_URL },
      { label: '反馈问题', href: `${REPOSITORY_URL}/issues/new` },
      { label: '已报告的问题', href: `${REPOSITORY_URL}/issues` },
      { label: '安全说明', href: `${REPOSITORY_URL}/blob/main/SECURITY.md` },
    ],
  },
]
</script>

<template>
  <footer class="footer">
    <div class="page">
      <div class="footer__grid">
        <div class="footer__brand-col">
          <a class="footer__brand" href="#top" aria-label="回到顶部">
            <img :src="asset('assets/icon.png')" alt="" width="24" height="24" />
            <span>DeskFuse</span>
          </a>
          <p class="footer__tagline">桌上几台电脑，当一台来用。</p>
          <p class="footer__meta">ISSUE 01 · macOS 14+ · LOCAL NETWORK</p>
        </div>

        <nav v-for="group in groups" :key="group.title" class="footer__group" :aria-label="group.title">
          <h2>{{ group.title }}</h2>
          <a
            v-for="link in group.links"
            :key="link.label"
            :href="link.href"
            :target="link.href.startsWith('#') ? undefined : '_blank'"
            :rel="link.href.startsWith('#') ? undefined : 'noopener'"
          >
            {{ link.label }}
          </a>
        </nav>
      </div>

      <div class="footer__bottom">
        <span>© 2026 DeskFuse</span>
        <span>本地直连 · 不经过云端</span>
        <span>
          源代码与许可见
          <a class="link" :href="`${REPOSITORY_URL}/blob/main/LICENSE`" target="_blank" rel="noopener">LICENSE</a>
        </span>
      </div>
    </div>
  </footer>
</template>

<style scoped>
.footer {
  padding-block: 48px 28px;
  border-top: 1px solid var(--rule-strong);
  background: var(--paper);
}

.footer__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.5fr) repeat(3, minmax(0, 0.7fr));
  gap: 36px 28px;
}

.footer__brand {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  color: var(--ink);
  text-decoration: none;
}

.footer__brand img {
  border-radius: 5px;
}

.footer__brand span {
  font-family: var(--font-serif-en);
  font-size: 1.35rem;
  font-weight: 500;
  letter-spacing: 0;
  line-height: 1;
}

.footer__tagline {
  max-width: 22em;
  margin-top: 14px;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.7;
}

.footer__meta {
  margin-top: 10px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.05em;
  line-height: 1.7;
}

.footer__group {
  display: grid;
  align-content: start;
}

.footer__group h2 {
  padding-bottom: 7px;
  border-bottom: 1px solid var(--rule-strong);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  font-weight: 500;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.footer__group a {
  padding: 5px 0;
  color: var(--body);
  font-size: 0.8125rem;
  line-height: 1.6;
  text-decoration: none;
}

.footer__group a:hover {
  color: var(--vermilion);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.footer__bottom {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px 24px;
  margin-top: 40px;
  padding-top: 14px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.02em;
  line-height: 1.7;
}

@media (max-width: 760px) {
  .footer__grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 28px 16px;
  }

  .footer__brand-col {
    grid-column: 1 / -1;
    padding-bottom: 20px;
    border-bottom: 1px solid var(--rule);
  }

  .footer__tagline {
    max-width: 24em;
  }
}
</style>

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
          <p>一套键鼠，连接每台电脑。</p>
        </div>

        <nav v-for="group in groups" :key="group.title" class="footer__group" :aria-label="group.title">
          <h2>{{ group.title }}</h2>
          <a
            v-for="link in group.links"
            :key="link.label"
            :href="link.href"
            :target="link.href.startsWith('#') ? undefined : '_blank'"
            :rel="link.href.startsWith('#') ? undefined : 'noopener'"
          >{{ link.label }}</a>
        </nav>
      </div>

      <div class="footer__bottom">
        <span>© 2026 DeskFuse</span>
        <span>本地直连 · 不经过云端</span>
        <span>源代码与许可见 <a class="link" :href="`${REPOSITORY_URL}/blob/main/LICENSE`" target="_blank" rel="noopener">LICENSE</a></span>
      </div>
    </div>
  </footer>
</template>

<style scoped>
.footer {
  padding-block: 52px 28px;
  border-top: 1px solid var(--rule);
  background: var(--paper-2);
}

.footer__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.5fr) repeat(3, minmax(0, 0.7fr));
  gap: 40px 32px;
}

.footer__brand {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  color: var(--ink);
  font-size: 1.25rem;
  font-weight: 600;
  line-height: 1;
  text-decoration: none;
}

.footer__brand img {
  border-radius: 6px;
}

.footer__brand-col > p {
  margin-top: 14px;
  color: var(--muted);
  font-size: 0.875rem;
  line-height: 1.7;
}

.footer__group {
  display: grid;
  align-content: start;
  gap: 8px;
}

.footer__group h2 {
  margin-bottom: 4px;
  color: var(--ink);
  font-size: 0.75rem;
  font-weight: 600;
  line-height: 1.5;
}

.footer__group a {
  width: fit-content;
  color: var(--body);
  font-size: 0.75rem;
  line-height: 1.6;
  text-decoration: none;
}

.footer__group a:hover {
  color: var(--link);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.footer__bottom {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px 24px;
  margin-top: 44px;
  padding-top: 20px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.7;
}

@media (max-width: 700px) {
  .footer__grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 32px 16px;
  }

  .footer__brand-col {
    grid-column: 1 / -1;
    padding-bottom: 24px;
    border-bottom: 1px solid var(--rule);
  }

  .footer__bottom {
    align-items: flex-start;
    flex-direction: column;
  }
}

@media (max-width: 380px) {
  .footer__grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .footer__group:last-child {
    grid-column: 1 / -1;
  }
}
</style>

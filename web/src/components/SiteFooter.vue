<script setup lang="ts">
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'

const links = [
  { label: 'GitHub', href: REPOSITORY_URL },
  { label: '全部版本', href: `${REPOSITORY_URL}/releases` },
  { label: '更新记录', href: `${REPOSITORY_URL}/blob/main/CHANGELOG.md` },
  { label: '第三方声明', href: `${REPOSITORY_URL}/blob/main/THIRD_PARTY_NOTICES.md` },
]
</script>

<template>
  <footer class="footer">
    <div class="page footer__inner">
      <a class="footer__brand" href="#top" aria-label="回到顶部">
        <img :src="asset('assets/icon.png')" alt="" width="24" height="24" />
        <span>DeskFuse</span>
      </a>
      <nav aria-label="页脚">
        <a v-for="l in links" :key="l.label" :href="l.href" target="_blank" rel="noopener">{{ l.label }}</a>
      </nav>
    </div>
  </footer>
</template>

<style scoped>
.footer {
  padding-block: 32px 40px;
  border-top: 1px solid var(--c-line);
  font-size: var(--t-small);
}

.footer__inner {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 16px 32px;
}

.footer__brand {
  display: flex;
  align-items: center;
  gap: 10px;
  font-weight: 650;
  text-decoration: none;
}

.footer__brand img {
  border-radius: 6px;
}

nav {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 24px;
}

nav a {
  color: var(--c-muted);
  text-decoration: none;
}

nav a:hover {
  color: var(--c-ink);
}
</style>

<script setup lang="ts">
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'
</script>

<template>
  <header class="nav">
    <div class="page nav__inner">
      <a class="nav__brand" href="#top" aria-label="DeskFuse 首页">
        <img :src="asset('assets/icon.png')" alt="" width="28" height="28" />
        <span>DeskFuse</span>
      </a>
      <nav class="nav__links" aria-label="页面导航">
        <a href="#screen-input">功能</a>
        <a href="#screen-remote">远程应用</a>
        <a href="#screen-mic">麦克风共享</a>
        <a href="#install">安装</a>
        <a href="#help">帮助</a>
        <a :href="REPOSITORY_URL" target="_blank" rel="noopener">GitHub</a>
      </nav>
      <a class="btn btn--primary btn--small" href="#install">下载</a>
    </div>
  </header>
</template>

<style scoped>
.nav {
  position: sticky;
  top: 0;
  z-index: 20;
  background: rgba(236, 235, 231, 0.86);
  backdrop-filter: saturate(1.4) blur(14px);
  -webkit-backdrop-filter: saturate(1.4) blur(14px);
  box-shadow: 0 1px 0 rgba(0, 0, 0, 0.06);
}

.nav__inner {
  display: flex;
  align-items: center;
  gap: 32px;
  height: 60px;
}

.nav__brand {
  display: flex;
  align-items: center;
  gap: 10px;
  font-weight: 650;
  text-decoration: none;
}

.nav__brand img {
  border-radius: 7px;
}

.nav__links {
  display: flex;
  gap: 26px;
  margin-left: auto;
  font-size: 0.9375rem;
}

.nav__links a {
  color: var(--c-muted);
  text-decoration: none;
  transition: color 0.15s;
}

.nav__links a:hover {
  color: var(--c-ink);
}

@media (max-width: 720px) {
  .nav__links {
    display: none;
  }

  .nav__inner .btn {
    margin-left: auto;
  }
}
</style>

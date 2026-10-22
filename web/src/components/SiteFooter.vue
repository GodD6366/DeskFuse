<script setup lang="ts">
import { asset } from './demos'
import { REPOSITORY_URL } from '../config/distribution'
</script>

<template>
  <footer class="footer">
    <div class="footer-inner">
      <div class="footer-brand">
        <img :src="asset('assets/icon.png')" alt="" class="nav-logo" />
        <span>DeskFuse</span>
        <p>macOS 优先的原生桌面协作工具<br />Rust 2024 引擎 · SwiftUI 客户端</p>
      </div>
      <div class="footer-links">
        <div>
          <h4>下载</h4>
          <a href="#install">直接下载 macOS 安装包</a>
          <a :href="REPOSITORY_URL" target="_blank" rel="noopener">GitHub 仓库</a>
          <a :href="`${REPOSITORY_URL}/issues`" target="_blank" rel="noopener">反馈问题</a>
        </div>
        <div>
          <h4>项目</h4>
          <a href="#features">功能</a>
          <a href="#compare">对比</a>
          <a href="#install">安装</a>
        </div>
      </div>
    </div>
    <p class="footer-copy">© 2026 DeskFuse · 预发行阶段，构建通过不代表真实两机验收</p>
  </footer>
</template>

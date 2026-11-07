<script setup lang="ts">
import { asset } from '../../modules/asset'
import { useDownloads } from '../../composables/useDownloads'
import { REPOSITORY_URL } from '../../config/distribution'

const { tag } = useDownloads()
</script>

<template>
  <section class="cta section section--gray">
    <div class="page cta__inner">
      <img class="cta__icon" :src="asset('assets/icon.png')" alt="" width="76" height="76" />
      <h2>复制在这台，<br />粘贴在那台。</h2>
      <p>在每台电脑上安装，配对一次。键鼠、剪贴板和文件就能跟着你走。</p>
      <div class="cta__actions">
        <a class="btn btn--primary" href="#download">下载 DeskFuse</a>
        <a class="text-link" :href="REPOSITORY_URL" target="_blank" rel="noopener">在 GitHub 查看 <span aria-hidden="true">›</span></a>
      </div>
      <p class="cta__meta">{{ tag }} · 局域网直连 · 设备密钥配对</p>
    </div>
  </section>
</template>

<style scoped>
.cta {
  text-align: center;
}

.cta__inner {
  display: flex;
  flex-direction: column;
  align-items: center;
}

.cta__icon {
  margin-bottom: 28px;
  border-radius: 18px;
  box-shadow: 0 12px 30px rgb(0 0 0 / 12%);
}

.cta h2 {
  font-size: var(--t-h2);
}

.cta__inner > p:not(.cta__meta) {
  margin-top: 20px;
  color: var(--body);
  font-size: var(--t-lead);
  line-height: 1.6;
}

.cta__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 12px 26px;
  margin-top: 28px;
}

.cta__meta {
  margin-top: 18px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.6;
}
</style>

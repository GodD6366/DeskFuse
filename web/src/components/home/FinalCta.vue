<script setup lang="ts">
import { useDownloads } from '../../composables/useDownloads'
import { REPOSITORY_URL } from '../../config/distribution'

const { tag, arm64 } = useDownloads()
</script>

<template>
  <section class="cta">
    <div class="page cta__grid">
      <div class="cta__copy">
        <span class="eyebrow">END NOTE / 下一次复制</span>
        <h2>复制在这台，<br />粘贴在那台。</h2>
      </div>
      <div class="cta__aside">
        <p>两台都装好，配对一次。之后键鼠、剪贴板、文件和麦克风都像在同一台电脑上。</p>
        <div class="cta__actions">
          <a class="btn btn--primary" :href="arm64.url">下载 DeskFuse</a>
          <a class="cta__link" :href="REPOSITORY_URL" target="_blank" rel="noopener">
            在 GitHub 查看 <span aria-hidden="true">↗</span>
          </a>
        </div>
        <p class="cta__meta">{{ tag }} · macOS 14 或更新 · Apple 芯片与 Intel</p>
      </div>
    </div>
    <div class="page cta__folio">
      <span>DESKFUSE / 2026</span>
      <span>LOCAL NETWORK · NO CLOUD</span>
    </div>
  </section>
</template>

<style scoped>
.cta {
  padding-block: clamp(72px, 9vw, 120px) 18px;
  border-top: 1px solid var(--rule-strong);
  background: var(--paper-2);
}

.cta__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.25fr) minmax(280px, 0.75fr);
  gap: clamp(32px, 6vw, 88px);
  align-items: end;
}

.cta__copy h2 {
  margin-top: 16px;
  font-size: clamp(2.7rem, 1.7rem + 3.6vw, 4.8rem);
  line-height: 1.15;
}

.cta__aside {
  padding-bottom: 4px;
}

.cta__aside > p:first-child {
  max-width: 32em;
  color: var(--body);
  font-size: 1.0625rem;
  line-height: 1.85;
}

.cta__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 22px;
  margin-top: 24px;
}

.cta__link {
  padding-bottom: 2px;
  border-bottom: 1px solid var(--rule-strong);
  color: var(--ink);
  font-size: 0.875rem;
  font-weight: 600;
  text-decoration: none;
}

.cta__link:hover {
  color: var(--vermilion);
  border-bottom-color: var(--vermilion);
}

.cta__meta {
  margin-top: 18px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.03em;
  line-height: 1.6;
}

.cta__folio {
  display: flex;
  justify-content: space-between;
  gap: 20px;
  margin-top: clamp(48px, 7vw, 88px);
  padding-top: 12px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
  line-height: 1.5;
}

@media (max-width: 900px) {
  .cta__grid {
    grid-template-columns: 1fr;
    gap: 28px;
  }
}

@media (max-width: 640px) {
  .cta__actions {
    align-items: flex-start;
    flex-direction: column;
    gap: 16px;
  }

  .cta__folio {
    flex-direction: column;
    gap: 4px;
  }
}
</style>

<script setup lang="ts">
import { asset } from '../modules/asset'
import { REPOSITORY_URL } from '../config/distribution'

const issuesUrl = `${REPOSITORY_URL}/issues`
const releasesUrl = `${REPOSITORY_URL}/releases`
const architectureUrl = `${REPOSITORY_URL}/blob/main/docs/ARCHITECTURE.md`
const securityUrl = `${REPOSITORY_URL}/blob/main/SECURITY.md`
const homebrewUrl = `${REPOSITORY_URL}/tree/main/Casks`

interface FooterLink {
  label: string
  href: string
  external?: boolean
}

interface FooterGroup {
  title: string
  links: FooterLink[]
}

const groups: FooterGroup[] = [
  {
    title: '页面',
    links: [
      { label: '现场', href: '#demo' },
      { label: '能力', href: '#abilities' },
      { label: '架构', href: '#architecture' },
      { label: '对比', href: '#compare' },
      { label: '安装', href: '#install' },
    ],
  },
  {
    title: '获取',
    links: [
      { label: 'GitHub', href: REPOSITORY_URL, external: true },
      { label: '全部版本', href: releasesUrl, external: true },
      { label: 'Homebrew cask', href: homebrewUrl, external: true },
    ],
  },
  {
    title: '支持',
    links: [
      { label: '反馈问题', href: issuesUrl, external: true },
      { label: '架构文档', href: architectureUrl, external: true },
      { label: '安全说明', href: securityUrl, external: true },
    ],
  },
]
</script>

<template>
  <footer class="footer">
    <div class="container">
      <div class="footer__support" v-reveal>
        <span class="footer__support-mark" aria-hidden="true">?</span>
        <p>
          <b>用的时候卡住了？</b>
          在 GitHub 开一个问题；应用内「帮助 → 反馈问题」会一并导出脱敏诊断，排查会快很多。
        </p>
        <a class="btn btn--ghost btn--small" :href="issuesUrl" target="_blank" rel="noopener">
          反馈问题
        </a>
      </div>

      <a class="footer__wordmark" href="#top" aria-label="回到顶部">
        <span>DeskFuse</span>
      </a>

      <div class="footer__main">
        <div class="footer__brand">
          <img :src="asset('assets/icon.png')" alt="" width="28" height="28" class="footer__icon" />
          <p>
            macOS 优先的原生桌面协作工具。<br />
            Rust 2024 引擎 · SwiftUI 客户端。
          </p>
          <span class="footer__local"><i aria-hidden="true"></i>LOCAL-FIRST / QUIC</span>
        </div>

        <nav class="footer__groups" aria-label="页脚导航">
          <div v-for="group in groups" :key="group.title" class="footer__group">
            <h3>{{ group.title }}</h3>
            <a
              v-for="link in group.links"
              :key="link.label"
              :href="link.href"
              :target="link.external ? '_blank' : undefined"
              :rel="link.external ? 'noopener' : undefined"
            >
              {{ link.label }}
            </a>
          </div>
        </nav>
      </div>

      <div class="footer__bottom">
        <span>© 2026 DeskFuse</span>
        <span>预发行阶段，部分能力仍在持续验收</span>
        <a href="#top">回到顶部</a>
      </div>
    </div>
  </footer>
</template>

<style scoped>
.footer {
  position: relative;
  padding-top: 1px;
  border-top: 1px solid var(--line);
  background: var(--bg-deep);
  overflow: clip;
}

.footer__support {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-top: 0;
  padding: 20px 0;
  border-bottom: 1px solid var(--line);
}

.footer__support-mark {
  display: grid;
  place-items: center;
  flex: none;
  width: 24px;
  height: 24px;
  border: 1px solid rgba(89, 243, 218, 0.35);
  border-radius: 50%;
  color: var(--aqua);
  font-family: var(--mono);
  font-size: 10px;
}

.footer__support p {
  max-width: 68ch;
  color: var(--faint);
  font-size: 11.5px;
  line-height: 1.65;
}

.footer__support b {
  margin-right: 8px;
  color: var(--ink);
  font-weight: 600;
}

.footer__support .btn {
  flex: none;
  margin-left: auto;
}

.footer__wordmark {
  display: block;
  width: 100%;
  margin: clamp(44px, 7vw, 90px) 0 clamp(34px, 5vw, 64px);
  color: transparent;
  font-family: var(--display);
  font-size: clamp(90px, 23vw, 380px);
  line-height: 0.72;
  letter-spacing: -0.03em;
  white-space: nowrap;
  -webkit-text-stroke: 1px rgba(207, 255, 243, 0.22);
  transition: -webkit-text-stroke-color 0.35s ease;
}

.footer__wordmark:hover {
  -webkit-text-stroke-color: rgba(89, 243, 218, 0.7);
}

.footer__wordmark span {
  display: block;
  transform: translateY(-0.06em);
}

.footer__main {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: clamp(32px, 7vw, 120px);
  padding-bottom: clamp(44px, 6vw, 72px);
}

.footer__brand {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
}

.footer__icon {
  width: 28px;
  height: 28px;
  border-radius: 7px;
}

.footer__brand p {
  color: var(--muted);
  font-size: 12.5px;
  line-height: 1.7;
}

.footer__local {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  letter-spacing: 0.06em;
}

.footer__local i {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--aqua);
}

.footer__groups {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 24px;
}

.footer__group {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.footer__group h3 {
  margin-bottom: 5px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  font-weight: 400;
  letter-spacing: 0.08em;
}

.footer__group a {
  width: fit-content;
  color: var(--muted);
  font-size: 12px;
  border-bottom: 1px solid transparent;
  transition:
    color 0.2s ease,
    border-color 0.2s ease;
}

.footer__group a:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

.footer__bottom {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 26px;
  padding: 18px 0 26px;
  border-top: 1px solid var(--line);
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  letter-spacing: 0.03em;
}

.footer__bottom span:nth-child(2) {
  margin-inline: auto;
}

.footer__bottom a {
  border-bottom: 1px solid var(--line);
}

.footer__bottom a:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

@media (max-width: 800px) {
  .footer__support {
    flex-wrap: wrap;
    align-items: flex-start;
  }

  .footer__support p {
    flex: 1;
    min-width: 220px;
  }

  .footer__support .btn {
    margin-left: 40px;
  }

  .footer__main {
    grid-template-columns: minmax(0, 1fr);
    gap: 32px;
  }
}

@media (max-width: 520px) {
  .footer__support .btn {
    margin-left: 0;
  }

  .footer__groups {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 28px 18px;
  }

  .footer__bottom {
    flex-direction: column;
    align-items: flex-start;
    gap: 7px;
  }

  .footer__bottom span:nth-child(2) {
    margin-inline: 0;
  }
}
</style>

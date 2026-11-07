<script setup lang="ts">
import { REPOSITORY_URL } from '../config/distribution'

const newIssueUrl = `${REPOSITORY_URL}/issues/new`
const issuesUrl = `${REPOSITORY_URL}/issues`
const securityUrl = `${REPOSITORY_URL}/blob/main/SECURITY.md`

const questions = [
  { q: '需要登录同一个 Apple 账号吗？', a: '不需要。不同电脑通过设备密钥配对，第一次连接时两台电脑上都要确认。配对过的设备可以随时移除。' },
  { q: '数据会经过云端吗？', a: '不会。设备在同一个局域网里直接连接，键鼠、剪贴板、文件和音频都走加密连接；没有账号，也没有云端中转。' },
  { q: '光标移到屏幕边缘，为什么过不去？', a: '先确认两台都打开了键鼠共享，并在「系统设置 › 隐私与安全性 › 辅助功能」里允许 DeskFuse。再看一眼屏幕排布：光标只能从两块屏幕相接的那段边过去。' },
  { q: '列表里看不到另一台电脑怎么办？', a: '确认两台连在同一个网络，并允许 DeskFuse 访问本地网络。公司或酒店网络常常不让设备互相发现，这时点「+」输入对方的 IP 地址。' },
  { q: '支持 Windows 吗？', a: '支持。Windows x64 当前为 Beta，可与 Mac 共用键鼠、同步剪贴板并传输文件。安装器随 Release 发布；麦克风共享和鼠标自定义等能力以对应功能的平台说明为准。' },
  { q: '怎么反馈问题？', a: '在应用菜单栏点「帮助 › 反馈问题…」，会打开一个填好版本、系统和运行状态的 GitHub Issue，诊断文件已经脱敏。' },
]
</script>

<template>
  <section id="help" class="faq section">
    <div class="page">
      <header class="section-heading section-heading--left">
        <h2>常见问题。</h2>
        <p>安装、连接和权限相关的问题，先看这里。</p>
      </header>

      <div class="faq__grid">
        <div class="faq__list">
          <details v-for="item in questions" :key="item.q">
            <summary>{{ item.q }}</summary>
            <p>{{ item.a }}</p>
          </details>
        </div>

        <aside class="faq__contact">
          <h3>还没找到答案？</h3>
          <p>直接反馈问题。附带的诊断已经脱敏，不会包含剪贴板内容。</p>
          <div class="faq__actions">
            <a class="btn btn--primary btn--small" :href="newIssueUrl" target="_blank" rel="noopener">反馈问题</a>
            <a class="btn btn--quiet btn--small" :href="issuesUrl" target="_blank" rel="noopener">已报告的问题</a>
          </div>
          <p class="faq__security">
            发现安全漏洞请按 <a class="link" :href="securityUrl" target="_blank" rel="noopener">安全说明</a> 私下报告，不要公开提交。
          </p>
        </aside>
      </div>
    </div>
  </section>
</template>

<style scoped>
.faq__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.45fr) minmax(280px, 0.55fr);
  gap: clamp(40px, 7vw, 88px);
  align-items: start;
}

.faq__list {
  border-top: 1px solid var(--rule);
}

details {
  border-bottom: 1px solid var(--rule);
}

summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  min-height: 68px;
  padding: 16px 0;
  color: var(--ink);
  font-size: 1.0625rem;
  font-weight: 500;
  line-height: 1.6;
  list-style: none;
  cursor: pointer;
}

summary::-webkit-details-marker {
  display: none;
}

summary::after {
  flex: none;
  color: var(--link);
  font-size: 1.5rem;
  font-weight: 400;
  content: '+';
}

details[open] summary::after {
  content: '−';
}

details p {
  max-width: 62ch;
  padding: 0 34px 24px 0;
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.8;
}

.faq__contact {
  padding: 28px;
  border-radius: 12px;
  background: var(--paper-2);
}

.faq__contact h3 {
  font-size: 1.5rem;
  line-height: 1.35;
}

.faq__contact > p {
  margin-top: 12px;
  color: var(--body);
  font-size: 0.875rem;
  line-height: 1.75;
}

.faq__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 9px;
  margin-top: 24px;
}

.faq__security {
  margin-top: 24px !important;
  padding-top: 18px;
  border-top: 1px solid var(--rule);
  color: var(--muted) !important;
  font-size: 0.75rem !important;
}

@media (max-width: 900px) {
  .faq__grid {
    grid-template-columns: 1fr;
  }

  .faq__contact {
    max-width: 560px;
  }
}

@media (max-width: 520px) {
  summary {
    font-size: 1rem;
  }

  details p {
    padding-right: 0;
  }

  .faq__contact {
    padding: 24px;
  }
}
</style>

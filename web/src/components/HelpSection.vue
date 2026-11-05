<script setup lang="ts">
import { REPOSITORY_URL } from '../config/distribution'

const newIssueUrl = `${REPOSITORY_URL}/issues/new`
const issuesUrl = `${REPOSITORY_URL}/issues`
const securityUrl = `${REPOSITORY_URL}/blob/main/SECURITY.md`

const questions = [
  {
    q: '两台 Mac 需要登录同一个 Apple 账号吗？',
    a: '不需要。DeskFuse 用设备密钥配对，第一次连接时两台电脑上都要确认。配对过的设备可以随时移除。',
  },
  {
    q: '数据会经过云端吗？',
    a: '不会。设备在同一个局域网里直接连接，键鼠、剪贴板、文件和音频都走加密连接；没有账号，也没有云端中转。',
  },
  {
    q: '光标移到屏幕边缘，为什么过不去？',
    a: '先确认两台都打开了键鼠共享，并在「系统设置 › 隐私与安全性 › 辅助功能」里允许 DeskFuse。再看一眼屏幕排布：光标只能从两块屏幕相接的那段边过去。',
  },
  {
    q: '列表里看不到另一台电脑怎么办？',
    a: '确认两台连在同一个网络，并允许 DeskFuse 访问本地网络。公司或酒店网络常常不让设备互相发现，这时点「+」输入对方的 IP 地址。',
  },
  {
    q: '支持 Windows 吗？',
    a: 'Windows 客户端仍在测试，暂不提供公开下载。macOS 上可以通过远程应用（Beta）打开 Windows 程序，不需要对局域网开放 RDP 端口。',
  },
  {
    q: '怎么反馈问题？',
    a: '在应用菜单栏点「帮助 › 反馈问题…」，会打开一个填好版本、系统和运行状态的 GitHub Issue，诊断文件已经脱敏。',
  },
]
</script>

<template>
  <section id="help" class="faq section">
    <div class="page">
      <header class="faq__head">
        <h2>常见问题</h2>
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
          <span class="faq__label">CONTACT / 联系编辑</span>
          <b>没找到答案？</b>
          <p>反馈问题时附带的诊断已经脱敏，不会包含剪贴板内容。</p>
          <div class="faq__actions">
            <a class="btn btn--primary btn--small" :href="newIssueUrl" target="_blank" rel="noopener">
              反馈问题
            </a>
            <a class="btn btn--quiet btn--small" :href="issuesUrl" target="_blank" rel="noopener">
              已报告的问题
            </a>
          </div>
          <p class="faq__security">
            发现安全漏洞请不要公开提交，按
            <a class="link" :href="securityUrl" target="_blank" rel="noopener">安全说明</a>
            私下报告。
          </p>
        </aside>
      </div>
    </div>
  </section>
</template>

<style scoped>
.faq {
  border-top: 1px solid var(--rule-strong);
}

.faq__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 34px;
  padding-top: 14px;
}

.faq__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.faq__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.faq__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.5fr) minmax(260px, 0.5fr);
  gap: clamp(32px, 5vw, 72px);
  align-items: start;
}

.faq__list {
  border-top: 1px solid var(--rule-strong);
}

details {
  border-bottom: 1px solid var(--rule);
}

summary {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 20px;
  padding: 19px 0;
  color: var(--ink);
  font-size: 1.0625rem;
  font-weight: 500;
  line-height: 1.55;
  list-style: none;
  cursor: pointer;
}

summary::-webkit-details-marker {
  display: none;
}

summary::after {
  flex: none;
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 1.1rem;
  font-weight: 400;
  content: '+';
}

details[open] summary::after {
  content: '−';
}

summary:hover {
  color: var(--vermilion);
}

details p {
  max-width: 62ch;
  padding: 0 30px 22px 0;
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.85;
}

.faq__contact {
  padding: 18px;
  border: 1px solid var(--rule);
  background: var(--paper-2);
}

.faq__label {
  display: block;
  margin-bottom: 18px;
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
}

.faq__contact b {
  font-family: var(--font-display);
  font-size: 1.5rem;
  font-weight: 500;
  letter-spacing: 0;
  line-height: 1.3;
}

.faq__contact > p {
  margin-top: 8px;
  color: var(--body);
  font-size: 0.875rem;
  line-height: 1.75;
}

.faq__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 18px;
}

.faq__actions .btn {
  flex: 1 1 auto;
}

.faq__security {
  padding-top: 14px;
  border-top: 1px solid var(--rule);
  color: var(--muted) !important;
  font-size: 0.75rem !important;
  line-height: 1.7 !important;
}

@media (max-width: 1080px) {
  .faq__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .faq__head h2,
  .faq__head p {
    grid-column: 1;
  }

  .faq__head p {
    max-width: 38em;
  }
}

@media (max-width: 960px) {
  .faq__grid {
    grid-template-columns: 1fr;
  }

  .faq__contact {
    max-width: 520px;
  }
}

@media (max-width: 640px) {
  summary {
    padding: 16px 0;
    font-size: 1rem;
  }

  details p {
    padding-right: 0;
    font-size: 0.9375rem;
  }

  .faq__actions {
    flex-direction: column;
  }
}
</style>

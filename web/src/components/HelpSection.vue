<script setup lang="ts">
import { REPOSITORY_URL } from '../config/distribution'

const newIssueUrl = `${REPOSITORY_URL}/issues/new`
const issuesUrl = `${REPOSITORY_URL}/issues`
const securityUrl = `${REPOSITORY_URL}/blob/main/SECURITY.md`

const questions = [
  {
    q: '「设备」页里看不到另一台电脑',
    a: '先确认两台连在同一个网络里，并且都在系统弹窗里允许了 DeskFuse 访问本地网络。公司或酒店的网络常常不让设备互相发现，这时点「+」直接输入对方的 IP 地址。',
  },
  {
    q: '鼠标移到屏幕边缘过不去',
    a: '两台电脑都要打开键鼠共享，并在「系统设置 › 隐私与安全性 › 辅助功能」里允许 DeskFuse。再看一眼键鼠共享页的屏幕排布：光标只能从两块屏幕相接的那段边过去。',
  },
  {
    q: '侧栏底部显示「需要留意」',
    a: '点开它，每一条都写着出了什么问题，旁边就是修复的按钮。',
  },
]
</script>

<template>
  <section id="help" class="section help">
    <div class="page help__grid">
      <div>
        <div class="section-head">
          <h2>遇到问题</h2>
          <p>
            在应用里点菜单栏的「帮助 › 反馈问题…」（<kbd>⌘</kbd> <kbd>?</kbd>），会打开一个填好版本、系统和运行状态的 GitHub Issue，附带的诊断已经脱敏。
          </p>
        </div>
        <div class="help__actions">
          <a class="btn btn--primary" :href="newIssueUrl" target="_blank" rel="noopener">反馈问题</a>
          <a class="btn btn--quiet" :href="issuesUrl" target="_blank" rel="noopener">查看已报告的问题</a>
        </div>
        <p class="help__security">
          发现安全漏洞请不要公开提交，按
          <a class="link" :href="securityUrl" target="_blank" rel="noopener">安全说明</a>
          私下报告。
        </p>
      </div>

      <div class="faq">
        <details v-for="item in questions" :key="item.q">
          <summary>{{ item.q }}</summary>
          <p>{{ item.a }}</p>
        </details>
      </div>
    </div>
  </section>
</template>

<style scoped>
.help__grid {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
  gap: clamp(40px, 6vw, 96px);
}

.help .section-head {
  margin-bottom: 32px;
}

.help__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

.help__security {
  margin-top: 20px;
  color: var(--c-muted);
  font-size: var(--t-small);
}

.faq {
  align-self: start;
  border-top: 1px solid var(--c-line);
}

details {
  border-bottom: 1px solid var(--c-line);
}

summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 20px 0;
  font-weight: 600;
  cursor: pointer;
  list-style: none;
}

summary::-webkit-details-marker {
  display: none;
}

summary::after {
  content: '';
  width: 10px;
  height: 10px;
  margin-right: 4px;
  border-right: 1.5px solid var(--c-muted);
  border-bottom: 1.5px solid var(--c-muted);
  transform: translateY(-3px) rotate(45deg);
  transition: transform 0.2s var(--ease);
  flex: none;
}

details[open] summary::after {
  transform: translateY(2px) rotate(-135deg);
}

details p {
  max-width: 34em;
  padding-bottom: 22px;
  color: var(--c-muted);
}

@media (max-width: 860px) {
  .help__grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>

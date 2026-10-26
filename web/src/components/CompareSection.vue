<script setup lang="ts">
type Mark = 'yes' | 'partial' | 'no'

interface Row {
  feature: string
  apple: [Mark, string?]
  deskflow: [Mark, string?]
  deskfuse: [Mark, string?]
}

const rows: Row[] = [
  {
    feature: '键鼠跨屏',
    apple: ['yes', '需要同一个 Apple 账号'],
    deskflow: ['partial', '屏幕位置不能自由摆放'],
    deskfuse: ['yes', '按真实位置摆放'],
  },
  {
    feature: '远程应用窗口',
    apple: ['partial', '屏幕共享只能看整块屏幕'],
    deskflow: ['no'],
    deskfuse: ['yes', '单个应用开成本机窗口，含 Windows'],
  },
  {
    feature: '共享麦克风',
    apple: ['no'],
    deskflow: ['no'],
    deskfuse: ['yes', '另一台多出一个虚拟麦克风'],
  },
  {
    feature: '剪贴板同步',
    apple: ['yes', '依赖 iCloud，时好时坏'],
    deskflow: ['partial', '只有文字和小图片'],
    deskfuse: ['yes', '保留原生格式'],
  },
  {
    feature: '剪贴板历史',
    apple: ['no'],
    deskflow: ['no'],
    deskfuse: ['yes', '跨设备同步，可搜图片里的文字'],
  },
  {
    feature: '文件传输',
    apple: ['yes', '隔空投送'],
    deskflow: ['no'],
    deskfuse: ['yes', '支持整个文件夹'],
  },
  {
    feature: '文件暂存抽屉',
    apple: ['no'],
    deskflow: ['no'],
    deskfuse: ['yes'],
  },
  {
    feature: '加密传输',
    apple: ['yes'],
    deskflow: ['yes', 'TLS'],
    deskfuse: ['yes', 'QUIC'],
  },
]

const label: Record<Mark, string> = { yes: '支持', partial: '部分支持', no: '不支持' }
</script>

<template>
  <section id="compare" class="section compare">
    <div class="page">
      <div class="section-head">
        <h2>和系统自带的、和 Deskflow 比</h2>
        <p>如果你的电脑都登录同一个 Apple 账号，只需要键鼠跨屏，系统自带的通用控制就够用。</p>
      </div>

      <div class="compare__scroll" tabindex="0" role="region" aria-label="功能对比表">
        <table>
          <thead>
            <tr>
              <th scope="col"><span class="sr-only">功能</span></th>
              <th scope="col">macOS 连续互通</th>
              <th scope="col">Deskflow</th>
              <th scope="col" class="is-us">DeskFuse</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.feature">
              <th scope="row">{{ row.feature }}</th>
              <td v-for="(cell, i) in [row.apple, row.deskflow, row.deskfuse]" :key="i" :class="{ 'is-us': i === 2 }">
                <span class="mark" :class="`mark--${cell[0]}`">{{ label[cell[0]] }}</span>
                <span v-if="cell[1]" class="note">{{ cell[1] }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <p class="compare__source">
        依据 2026 年 9 月的公开资料：
        <a class="link" href="https://support.apple.com/en-us/HT212757" target="_blank" rel="noopener">通用控制</a>、
        <a class="link" href="https://support.apple.com/en-us/102430" target="_blank" rel="noopener">通用剪贴板</a>、
        <a class="link" href="https://support.apple.com/guide/mac-help/mh14066/mac" target="_blank" rel="noopener">屏幕共享</a>、
        <a class="link" href="https://github.com/deskflow/deskflow" target="_blank" rel="noopener">Deskflow</a>。
      </p>
    </div>
  </section>
</template>

<style scoped>
.compare__scroll {
  overflow-x: auto;
  border-radius: var(--r-card);
  background: var(--c-paper);
  box-shadow: inset 0 0 0 1px var(--c-line);
}

table {
  width: 100%;
  min-width: 640px;
  border-collapse: collapse;
  font-size: 0.9375rem;
}

th,
td {
  padding: 18px 20px;
  text-align: left;
  vertical-align: top;
}

thead th {
  font-weight: 600;
  color: var(--c-muted);
  font-size: var(--t-small);
}

thead th.is-us {
  color: var(--c-ink);
}

tbody tr {
  border-top: 1px solid var(--c-line);
}

tbody th {
  font-weight: 600;
  white-space: nowrap;
}

td.is-us {
  background: rgba(240, 90, 36, 0.05);
}

.mark {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-weight: 500;
}

.mark::before {
  content: '';
  width: 12px;
  height: 8px;
  border-radius: 2px;
  flex: none;
}

.mark--yes::before {
  background: var(--c-graphite);
}

.is-us .mark--yes::before {
  background: var(--c-signal);
}

.mark--partial::before {
  background: linear-gradient(90deg, var(--c-graphite) 50%, var(--c-line) 50%);
}

.mark--no {
  color: var(--c-muted);
}

.mark--no::before {
  box-shadow: inset 0 0 0 1.5px var(--c-line);
}

.note {
  display: block;
  margin-top: 4px;
  padding-left: 20px;
  color: var(--c-muted);
  font-size: var(--t-small);
  line-height: 1.5;
}

.compare__source {
  margin-top: 16px;
  color: var(--c-muted);
  font-size: var(--t-small);
}
</style>

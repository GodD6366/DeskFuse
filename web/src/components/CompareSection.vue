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
    deskfuse: ['yes', '单个应用开成本机窗口，默认虚拟屏，含 Windows'],
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
    deskfuse: ['yes'],
  },
]

const label: Record<Mark, string> = { yes: '支持', partial: '部分支持', no: '不支持' }

const columns = (row: Row) => [
  { id: 'deskfuse', name: 'DeskFuse', cell: row.deskfuse, us: true },
  { id: 'apple', name: 'macOS 连续互通', cell: row.apple, us: false },
  { id: 'deskflow', name: 'Deskflow', cell: row.deskflow, us: false },
]
</script>

<template>
  <section id="compare" class="section compare">
    <div class="page">
      <header class="compare__head">
        <h2>和系统自带的、<br />和 Deskflow 比</h2>
        <p>如果你的电脑都登录同一个 Apple 账号，只需要键鼠跨屏，系统自带的通用控制就够用。</p>
      </header>

      <div class="compare__scroll" tabindex="0" role="region" aria-label="功能对比表">
        <table>
          <thead>
            <tr>
              <th scope="col"><span class="compare__row-label">功能 / FEATURE</span></th>
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

      <div class="compare__cards">
        <article v-for="row in rows" :key="row.feature" class="cc">
          <h3>{{ row.feature }}</h3>
          <dl class="cc__list">
            <div v-for="col in columns(row)" :key="col.id" class="cc__row" :class="{ 'is-us': col.us }">
              <dt>{{ col.name }}</dt>
              <dd>
                <span class="mark" :class="`mark--${col.cell[0]}`">{{ label[col.cell[0]] }}</span>
                <span v-if="col.cell[1]" class="note">{{ col.cell[1] }}</span>
              </dd>
            </div>
          </dl>
        </article>
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
.compare {
  border-top: 1px solid var(--rule-strong);
}

.compare__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 40px;
  padding-top: 14px;
}

.compare__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.compare__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.compare__cards {
  display: none;
}

.compare__scroll {
  overflow-x: auto;
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule-strong);
  background: var(--paper);
}

table {
  width: 100%;
  min-width: 680px;
  border-collapse: collapse;
  font-size: 0.875rem;
}

th,
td {
  padding: 13px 16px;
  text-align: left;
  vertical-align: top;
}

thead th {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  font-weight: 500;
  letter-spacing: 0.04em;
  line-height: 1.5;
}

.compare__row-label {
  display: inline-block;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.06em;
}

thead th.is-us {
  color: var(--vermilion);
}

tbody tr {
  border-top: 1px solid var(--rule);
}

tbody tr:hover {
  background: rgb(242 237 226 / 45%);
}

tbody th {
  color: var(--ink);
  font-size: 0.875rem;
  font-weight: 600;
  white-space: nowrap;
}

td.is-us {
  background: var(--paper-2);
}

.mark {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-weight: 500;
  line-height: 1.5;
}

.mark::before {
  width: 7px;
  height: 7px;
  flex: none;
  background: var(--ink);
  content: '';
}

.is-us .mark::before {
  background: var(--vermilion);
}

.mark--partial::before {
  background: linear-gradient(90deg, var(--ink) 50%, transparent 50%);
  box-shadow: inset 0 0 0 1px var(--ink);
}

.is-us .mark--partial::before {
  background: linear-gradient(90deg, var(--vermilion) 50%, transparent 50%);
  box-shadow: inset 0 0 0 1px var(--vermilion);
}

.mark--no {
  color: var(--muted);
}

.mark--no::before {
  background: transparent;
  box-shadow: inset 0 0 0 1px var(--rule);
}

.note {
  display: block;
  margin-top: 3px;
  padding-left: 14px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.6;
}

.compare__source {
  margin-top: 14px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.8;
}

@media (max-width: 1080px) {
  .compare__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .compare__head h2,
  .compare__head p {
    grid-column: 1;
  }

  .compare__head p {
    max-width: 38em;
  }
}

@media (max-width: 760px) {
  .compare__scroll {
    display: none;
  }

  .compare__cards {
    display: grid;
    border-top: 1px solid var(--rule-strong);
  }

  .cc {
    padding: 16px 0;
    border-bottom: 1px solid var(--rule);
  }

  .cc h3 {
    font-family: var(--font-sans);
    font-size: 0.9375rem;
    font-weight: 600;
    letter-spacing: 0;
  }

  .cc__list {
    display: grid;
    gap: 0;
    margin: 10px 0 0;
  }

  .cc__row {
    display: grid;
    grid-template-columns: 100px minmax(0, 1fr);
    gap: 12px;
    padding: 7px 0;
    border-top: 1px solid var(--rule);
  }

  .cc__row.is-us {
    background: var(--paper-2);
  }

  .cc dt {
    color: var(--muted);
    font-size: 0.75rem;
    line-height: 1.7;
  }

  .cc__row.is-us dt {
    color: var(--ink);
    font-weight: 600;
  }

  .cc dd {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.6;
  }

  .cc .note {
    margin-top: 1px;
    padding-left: 14px;
    font-size: 0.75rem;
  }
}
</style>

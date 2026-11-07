<script setup lang="ts">
type Mark = 'yes' | 'partial' | 'no'
interface Row {
  feature: string
  apple: [Mark, string?]
  deskflow: [Mark, string?]
  deskfuse: [Mark, string?]
}

const rows: Row[] = [
  { feature: '键鼠跨屏', apple: ['yes', '需要同一个 Apple 账号'], deskflow: ['partial', '屏幕位置不能自由摆放'], deskfuse: ['yes', '按真实位置摆放'] },
  { feature: '远程应用窗口', apple: ['partial', '屏幕共享只能看整块屏幕'], deskflow: ['no'], deskfuse: ['yes', '单个应用开成本机窗口，默认虚拟屏，含 Windows'] },
  { feature: '共享麦克风', apple: ['no'], deskflow: ['no'], deskfuse: ['yes', '另一台多出一个虚拟麦克风'] },
  { feature: '剪贴板同步', apple: ['yes', '依赖 iCloud，时好时坏'], deskflow: ['partial', '只有文字和小图片'], deskfuse: ['yes', '保留原生格式'] },
  { feature: '剪贴板历史', apple: ['no'], deskflow: ['no'], deskfuse: ['yes', '跨设备同步，可搜图片里的文字'] },
  { feature: '文件传输', apple: ['yes', '隔空投送'], deskflow: ['no'], deskfuse: ['yes', '支持整个文件夹'] },
  { feature: '文件暂存抽屉', apple: ['no'], deskflow: ['no'], deskfuse: ['yes'] },
  { feature: '加密传输', apple: ['yes'], deskflow: ['yes', 'TLS'], deskfuse: ['yes'] },
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
      <header class="section-heading section-heading--left">
        <h2>选择适合你的连接方式。</h2>
        <p>只需要键鼠跨屏，且电脑都登录同一个 Apple 账号，系统自带的通用控制就够用。需要更多能力时，再看看 DeskFuse。</p>
      </header>

      <div class="compare__scroll" tabindex="0" role="region" aria-label="功能对比表">
        <table>
          <thead>
            <tr>
              <th scope="col"><span class="sr-only">功能</span></th>
              <th scope="col" class="is-us">DeskFuse</th>
              <th scope="col">macOS 连续互通</th>
              <th scope="col">Deskflow</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.feature">
              <th scope="row">{{ row.feature }}</th>
              <td v-for="(cell, i) in [row.deskfuse, row.apple, row.deskflow]" :key="i" :class="{ 'is-us': i === 0 }">
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
.compare__scroll {
  overflow-x: auto;
  border-radius: 12px;
  background: var(--paper-2);
}

table {
  width: 100%;
  min-width: 760px;
  border-collapse: collapse;
  font-size: 0.875rem;
}

th,
td {
  padding: 20px;
  border-bottom: 1px solid var(--rule);
  text-align: left;
  vertical-align: top;
}

thead th {
  color: var(--body);
  font-size: 0.8125rem;
  font-weight: 600;
  line-height: 1.5;
}

thead th.is-us {
  color: var(--ink);
  font-size: 0.9375rem;
}

tbody th {
  width: 18%;
  color: var(--ink);
  font-weight: 500;
  white-space: nowrap;
}

td.is-us {
  background: #fff;
}

tbody tr:last-child th,
tbody tr:last-child td {
  border-bottom: 0;
}

.mark {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--body);
  font-weight: 500;
  line-height: 1.5;
}

.mark::before {
  color: var(--muted);
  content: '✓';
}

.mark--partial::before {
  content: '–';
}

.mark--no {
  color: var(--muted);
}

.mark--no::before {
  content: '—';
}

.is-us .mark,
.is-us .mark::before {
  color: var(--link);
}

.note {
  display: block;
  margin-top: 5px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.65;
}

.compare__source {
  margin-top: 18px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.8;
}

.compare__cards {
  display: none;
}

@media (max-width: 760px) {
  .compare__scroll {
    display: none;
  }

  .compare__cards {
    display: grid;
    border-top: 1px solid var(--rule);
  }

  .cc {
    padding: 22px 0;
    border-bottom: 1px solid var(--rule);
  }

  .cc h3 {
    font-size: 1.0625rem;
    line-height: 1.5;
  }

  .cc__list {
    display: grid;
    margin: 14px 0 0;
  }

  .cc__row {
    display: grid;
    grid-template-columns: 112px minmax(0, 1fr);
    gap: 12px;
    padding: 10px 0;
    border-top: 1px solid #e8e8ed;
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
}

@media (max-width: 380px) {
  .cc__row {
    grid-template-columns: 92px minmax(0, 1fr);
    gap: 8px;
  }
}
</style>

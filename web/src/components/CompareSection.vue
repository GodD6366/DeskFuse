<script setup lang="ts">
const rows = [
  {
    feature: '键鼠跨屏',
    apple: '仅限同一 Apple 账号',
    deskflow: '屏幕位置无法自由摆放',
    fuse: '真实布局，自由摆放',
  },
  {
    feature: '剪贴板同步',
    apple: '依赖 iCloud 与同一账号',
    deskflow: '基础文字与小图片',
    fuse: '文本、图片、原生格式',
  },
  {
    feature: '文件传输',
    apple: 'AirDrop',
    deskflow: '不提供',
    fuse: '支持文件夹，保留结构',
  },
  {
    feature: '文件暂存抽屉',
    apple: '不提供',
    deskflow: '不提供',
    fuse: '240 × 240，整批拖出',
  },
  {
    feature: '剪贴板历史',
    apple: '不提供',
    deskflow: '不提供',
    fuse: '跨设备，可搜图片内容',
  },
  {
    feature: '加密传输',
    apple: '系统级加密',
    deskflow: 'TLS',
    fuse: 'QUIC，设备间加密',
  },
]

const sources = [
  {
    label: 'Apple 通用控制',
    href: 'https://support.apple.com/en-us/HT212757',
  },
  {
    label: 'Apple 通用剪贴板',
    href: 'https://support.apple.com/en-us/102430',
  },
  {
    label: 'Deskflow',
    href: 'https://github.com/deskflow/deskflow',
  },
]
</script>

<template>
  <section id="compare" class="section compare" aria-labelledby="compare-title">
    <div class="container">
      <header class="section-head">
        <div class="section-head__main">
          <p class="section-index">04 / 对比</p>
          <h2 id="compare-title" v-reveal.title>
            你的设备，<br />
            不必共用一个账号。
          </h2>
        </div>
        <div class="section-head__aside" v-reveal="90">
          <p class="lead">
            连续互通适合轻度场景；DeskFuse 在同一局域网里，把剪贴板、文件和键鼠一起接上。
          </p>
        </div>
      </header>

      <div class="compare__wrap" v-reveal>
        <table class="compare__table">
          <thead>
            <tr>
              <th scope="col" class="compare__feature-head">能力</th>
              <th scope="col">macOS 连续互通</th>
              <th scope="col">Deskflow</th>
              <th scope="col" class="compare__us">
                <span>DeskFuse</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.feature">
              <th scope="row">{{ row.feature }}</th>
              <td data-label="macOS 连续互通">{{ row.apple }}</td>
              <td data-label="Deskflow">{{ row.deskflow }}</td>
              <td data-label="DeskFuse" class="compare__us-cell">
                <span class="compare__mark" aria-hidden="true"></span>
                {{ row.fuse }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="compare__cards" v-reveal>
        <article v-for="row in rows" :key="row.feature" class="compare-card">
          <h3>{{ row.feature }}</h3>
          <div class="compare-card__rows">
            <div class="compare-card__row">
              <span>macOS 连续互通</span>
              <b>{{ row.apple }}</b>
            </div>
            <div class="compare-card__row">
              <span>Deskflow</span>
              <b>{{ row.deskflow }}</b>
            </div>
            <div class="compare-card__row is-us">
              <span>DeskFuse</span>
              <b>{{ row.fuse }}</b>
            </div>
          </div>
        </article>
      </div>

      <div class="compare__foot" v-reveal="80">
        <p>
          对比依据 2026-09
          <template v-for="(source, index) in sources" :key="source.href">
            <span v-if="index > 0" aria-hidden="true"> · </span>
            <a :href="source.href" target="_blank" rel="noopener">{{ source.label }}</a>
          </template>
          。功能以各产品实际版本为准。
        </p>
        <p class="compare__pitch">
          DeskFuse 不要求你把设备交给同一个账号，也不要求先经过云端。
        </p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.compare {
  background:
    radial-gradient(ellipse 40% 35% at 85% 20%, rgba(255, 106, 60, 0.045), transparent 70%),
    var(--bg);
}

.compare__wrap {
  border-top: 1px solid var(--line-strong);
  border-bottom: 1px solid var(--line);
  overflow-x: auto;
  scrollbar-width: thin;
}

.compare__cards {
  display: none;
}

.compare__table {
  width: 100%;
  min-width: 760px;
  border-collapse: collapse;
  table-layout: fixed;
}

.compare__table th,
.compare__table td {
  padding: 21px 22px;
  border-bottom: 1px solid var(--line);
  text-align: left;
  font-size: 13px;
  font-weight: 400;
  line-height: 1.5;
}

.compare__table thead th {
  padding-block: 16px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.06em;
  white-space: nowrap;
}

.compare__table thead th:first-child,
.compare__table tbody th {
  width: 21%;
}

.compare__table thead th:not(:first-child) {
  width: 26.3%;
}

.compare__table tbody th {
  color: var(--ink);
  font-size: 14px;
  font-weight: 550;
}

.compare__table tbody td {
  color: var(--muted);
}

.compare__table tbody tr:last-child th,
.compare__table tbody tr:last-child td {
  border-bottom: 0;
}

.compare__table tbody tr {
  transition: background-color 0.2s ease;
}

.compare__table tbody tr:hover {
  background: rgba(207, 255, 243, 0.025);
}

.compare__feature-head {
  color: var(--faint);
}

.compare__us {
  color: var(--aqua);
  background: rgba(89, 243, 218, 0.045);
}

.compare__us span {
  font-family: var(--display);
  font-size: 18px;
  letter-spacing: 0;
}

.compare__us-cell {
  position: relative;
  color: var(--ink) !important;
  font-weight: 500 !important;
  background: rgba(89, 243, 218, 0.045);
}

.compare__mark {
  display: inline-block;
  width: 5px;
  height: 5px;
  margin-right: 9px;
  border-radius: 50%;
  background: var(--aqua);
  box-shadow: 0 0 8px rgba(89, 243, 218, 0.65);
  vertical-align: 1px;
}

.compare__foot {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 24px;
  margin-top: 20px;
}

.compare__foot p {
  max-width: 70ch;
  color: var(--faint);
  font-size: 10.5px;
  line-height: 1.7;
}

.compare__foot a {
  color: var(--muted);
  border-bottom: 1px solid var(--line);
}

.compare__foot a:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

.compare__pitch {
  max-width: 34ch !important;
  color: var(--muted) !important;
  text-align: right;
}

@media (max-width: 900px) {
  .compare__wrap {
    display: none;
  }

  .compare__cards {
    display: grid;
    gap: 14px;
  }

  .compare-card {
    border: 1px solid var(--line);
    background: var(--surface);
  }

  .compare-card h3 {
    padding: 13px 16px;
    border-bottom: 1px solid var(--line);
    background: rgba(207, 255, 243, 0.02);
    color: var(--ink);
    font-family: var(--display);
    font-size: clamp(22px, 6vw, 30px);
    font-weight: 400;
    line-height: 1.1;
  }

  .compare-card__row {
    display: grid;
    grid-template-columns: 120px minmax(0, 1fr);
    gap: 14px;
    align-items: baseline;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }

  .compare-card__row:last-child {
    border-bottom: 0;
  }

  .compare-card__row span {
    color: var(--faint);
    font-family: var(--mono);
    font-size: 8.5px;
    letter-spacing: 0.03em;
  }

  .compare-card__row b {
    color: var(--muted);
    font-size: 12.5px;
    font-weight: 400;
    line-height: 1.55;
  }

  .compare-card__row.is-us {
    background: rgba(89, 243, 218, 0.055);
  }

  .compare-card__row.is-us span {
    color: var(--aqua);
  }

  .compare-card__row.is-us b {
    color: var(--ink);
    font-weight: 550;
  }

  .compare__foot {
    flex-direction: column;
    gap: 10px;
  }

  .compare__pitch {
    max-width: 48ch !important;
    text-align: left;
  }
}

@media (max-width: 520px) {
  .compare-card__row {
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 10px;
    padding: 11px 13px;
  }
}
</style>

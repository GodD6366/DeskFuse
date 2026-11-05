<script setup lang="ts">
import { computed, ref } from 'vue'

interface Feature {
  id: string
  title: string
  body: string
  category: string
  badge?: string
}

const features: Feature[] = [
  {
    id: 'input',
    title: '键鼠共享',
    body: '光标跨过相接的屏幕边，键盘跟着过去。',
    category: '输入',
  },
  {
    id: 'layout',
    title: '屏幕排布',
    body: '按桌上的真实位置摆放，拖动后自动吸附。',
    category: '输入',
  },
  {
    id: 'clipboard',
    title: '剪贴板同步',
    body: '文字、图片和原生格式，复制一次两边都能粘。',
    category: '剪贴板',
  },
  {
    id: 'history',
    title: '剪贴板历史',
    body: '按类型分组保留，图片里的文字也能搜。',
    category: '剪贴板',
  },
  {
    id: 'mic',
    title: '麦克风共享',
    body: '另一台 Mac 的麦克风变成这台的虚拟输入。',
    category: '音频',
  },
  {
    id: 'mouse',
    title: '鼠标按键同步',
    body: '侧键、滚轮键和自定义快捷键跟着当前 Mac 走。',
    category: '鼠标',
    badge: 'Beta',
  },
  {
    id: 'remote',
    title: '远程应用',
    body: '高负载应用跑在另一台 Mac，默认虚拟屏。',
    category: '远程',
    badge: 'Beta',
  },
  {
    id: 'windows',
    title: 'Windows 程序',
    body: '通过 RemoteApp 打开，不开放 RDP 端口。',
    category: '远程',
    badge: 'Beta',
  },
  {
    id: 'files',
    title: '文件与文件夹',
    body: '整个目录原样传，名称和结构都不变。',
    category: '文件',
  },
  {
    id: 'shelf',
    title: '暂存抽屉',
    body: '先接住文件，切完窗口再整批拖出去。',
    category: '文件',
  },
  {
    id: 'phone',
    title: '手机传文件',
    body: '手机浏览器扫码上传，不用装 App。',
    category: '文件',
  },
  {
    id: 'pair',
    title: '设备配对',
    body: '每台设备一把密钥，首次连接两边确认。',
    category: '安全',
  },
  {
    id: 'encryption',
    title: '加密传输',
    body: '键鼠、剪贴板、文件都走加密连接，并验证设备签名。',
    category: '安全',
  },
]

const categories = ['全部', '输入', '剪贴板', '音频', '鼠标', '远程', '文件', '安全']
const filter = ref('全部')
const shown = computed(() =>
  filter.value === '全部' ? features : features.filter((feature) => feature.category === filter.value),
)
const count = (category: string) =>
  category === '全部' ? features.length : features.filter((feature) => feature.category === category).length
const indexOf = (id: string) => String(features.findIndex((feature) => feature.id === id) + 1).padStart(2, '0')
</script>

<template>
  <section id="features" class="features section">
    <div class="page">
      <header class="features__head">
        <h2>桌上想要的，<br class="features__break" />它都有</h2>
        <p>从一套键鼠到一块虚拟麦克风，常用能力都在一个应用里。</p>
      </header>

      <div class="features__filter" role="group" aria-label="按类别筛选能力">
        <button
          v-for="category in categories"
          :key="category"
          type="button"
          :class="{ 'is-active': filter === category }"
          :aria-pressed="filter === category"
          @click="filter = category"
        >
          {{ category }}
          <span>{{ count(category) }}</span>
        </button>
      </div>

      <div class="features__list" aria-live="polite">
        <article v-for="feature in shown" :key="feature.id" class="feature">
          <span class="feature__index">{{ indexOf(feature.id) }}</span>
          <div class="feature__copy">
            <div class="feature__title">
              <h3>{{ feature.title }}</h3>
              <span v-if="feature.badge" class="feature__beta">{{ feature.badge }}</span>
              <span class="feature__category">{{ feature.category }}</span>
            </div>
            <p>{{ feature.body }}</p>
          </div>
        </article>
      </div>

      <p class="features__note">
        <span>NOTE</span>
        所有能力都在你自己的电脑之间直连；只有检查更新会访问 GitHub。
      </p>
    </div>
  </section>
</template>

<style scoped>
.features {
  border-top: 1px solid var(--rule-strong);
  background: var(--paper);
}

.features__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 36px;
  padding-top: 14px;
}

.features__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.features__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.features__filter {
  display: flex;
  flex-wrap: wrap;
  gap: 0 24px;
  margin-bottom: 26px;
  border-top: 1px solid var(--rule);
  border-bottom: 1px solid var(--rule);
}

.features__filter button {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  padding: 12px 0 10px;
  border: 0;
  border-bottom: 2px solid transparent;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
}

.features__filter button:hover {
  color: var(--ink);
}

.features__filter button.is-active {
  border-bottom-color: var(--vermilion);
  color: var(--ink);
}

.features__filter span {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.02em;
}

.features__filter button.is-active span {
  color: var(--vermilion);
}

.features__list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  column-gap: clamp(32px, 5vw, 72px);
  border-top: 1px solid var(--rule);
}

.feature {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  gap: 12px;
  padding: 18px 0;
  border-bottom: 1px solid var(--rule);
}

.feature:last-child:nth-child(odd) {
  grid-column: 1 / -1;
}

.feature__index {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.03em;
  line-height: 1.9;
}

.feature__title {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 7px;
}

.feature__title h3 {
  font-family: var(--font-sans);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: 0;
  line-height: 1.5;
}

.feature__beta {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.feature__category {
  margin-left: auto;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.04em;
}

.feature__copy p {
  margin-top: 2px;
  color: var(--body);
  font-size: 0.875rem;
  line-height: 1.7;
}

.features__note {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 10px;
  margin-top: 24px;
  padding-top: 12px;
  border-top: 1px solid var(--rule-strong);
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.7;
}

.features__note span {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
}

@media (max-width: 1080px) {
  .features__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .features__head h2,
  .features__head p {
    grid-column: 1;
  }

  .features__head p {
    max-width: 38em;
  }
}

@media (max-width: 700px) {
  .features__list {
    grid-template-columns: 1fr;
  }

  .feature__category {
    margin-left: 0;
  }
}

@media (max-width: 640px) {
  .features__filter {
    gap: 0 16px;
  }
}

.features__break {
  display: none;
}

@media (max-width: 640px) {
  .features__break {
    display: inline;
  }
}
</style>

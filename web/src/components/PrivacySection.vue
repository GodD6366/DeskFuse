<script setup lang="ts">
const points = [
  {
    title: '局域网里直连',
    body: '电脑之间在同一个网络里互相发现、直接连接。不用注册账号，内容也不经过任何云端中转。',
  },
  {
    title: '先配对，才能连',
    body: '每台电脑有自己的设备密钥，第一次配对要你确认。没配对过的电脑即使在同一个 Wi-Fi 里也连不进来，配对过的随时可以移除。',
  },
  {
    title: '全程加密',
    body: '剪贴板、文件和键鼠操作都走 QUIC 加密连接，每次连接都会验证对方的设备签名。',
  },
  {
    title: '诊断先脱敏',
    body: '反馈问题时导出的诊断文件，电脑名、IP、MAC 地址和用户目录都会先换成代号，剪贴板内容不会写进去。',
  },
]
</script>

<template>
  <section id="privacy" class="section privacy">
    <div class="page privacy__grid">
      <div class="section-head">
        <h2>你的数据，只在你的电脑之间传</h2>
      </div>
      <dl class="privacy__list">
        <div v-for="p in points" :key="p.title">
          <dt>{{ p.title }}</dt>
          <dd>{{ p.body }}</dd>
        </div>
      </dl>
    </div>
  </section>
</template>

<style scoped>
.privacy {
  background: var(--c-graphite);
  color: #f2f1ee;
}

.privacy__grid {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: clamp(32px, 6vw, 96px);
}

.privacy .section-head {
  margin: 0;
}

.privacy__list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 40px 48px;
  margin: 0;
}

dt {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 1.125rem;
  font-weight: 600;
}

dt::before {
  content: '';
  width: 10px;
  height: 7px;
  border-radius: 2px;
  background: var(--c-signal);
  flex: none;
}

dd {
  margin: 10px 0 0;
  color: #b4b5b9;
}

@media (max-width: 860px) {
  .privacy__grid,
  .privacy__list {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>

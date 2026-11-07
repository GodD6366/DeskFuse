<script setup lang="ts">
import GlyphIcon from '../GlyphIcon.vue'
</script>

<template>
  <section class="adv section">
    <div class="page">
      <header class="section-heading">
        <h2>你的数据，<br />只在你的设备之间。</h2>
        <p>DeskFuse 没有账号体系，也没有云端中转。连接发生在你自己的局域网里。</p>
      </header>

      <div class="adv__statement">
        <GlyphIcon name="shield" :size="54" />
        <p>不必信任一台服务器。<br />只需确认你面前的电脑。</p>
      </div>

      <div class="adv__facts">
        <article>
          <GlyphIcon name="lock" :size="26" />
          <h3>设备密钥配对</h3>
          <p>每台设备一把密钥。第一次连接时，两台电脑上都要明确确认。</p>
        </article>
        <article>
          <GlyphIcon name="pulse" :size="26" />
          <h3>局域网加密直连</h3>
          <p>键鼠、剪贴板、文件和音频都走同一条加密连接，不经过云端。</p>
        </article>
        <article>
          <GlyphIcon name="check" :size="26" />
          <h3>控制权在你手里</h3>
          <p>每项能力独立开关，配对设备可以随时移除，不使用时不必常驻开启。</p>
        </article>
      </div>
    </div>
  </section>
</template>

<style scoped>
.adv__statement {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 28px;
  padding: clamp(40px, 6vw, 72px) 24px;
  border-radius: 12px;
  background: var(--paper-2);
  text-align: center;
}

.adv__statement p {
  font-size: clamp(2rem, 1.55rem + 1.8vw, 3rem);
  font-weight: 600;
  line-height: 1.3;
}

.adv__facts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: clamp(28px, 5vw, 64px);
  margin-top: 56px;
}

.adv__facts article {
  padding-top: 24px;
  border-top: 1px solid var(--rule);
}

.adv__facts h3 {
  margin-top: 18px;
  font-size: 1.25rem;
  line-height: 1.4;
}

.adv__facts p {
  margin-top: 10px;
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.75;
}

@media (max-width: 700px) {
  .adv__facts {
    grid-template-columns: 1fr;
    gap: 28px;
    margin-top: 40px;
  }

  .adv__facts article {
    display: grid;
    grid-template-columns: 30px minmax(0, 1fr);
    gap: 4px 14px;
  }

  .adv__facts h3,
  .adv__facts p {
    grid-column: 2;
    margin-top: 0;
  }

  .adv__facts p {
    margin-top: 6px;
  }
}
</style>

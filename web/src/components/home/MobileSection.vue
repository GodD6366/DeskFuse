<script setup lang="ts">
import GlyphIcon from '../GlyphIcon.vue'
import mobileShot from '../../assets/shots/mobile.webp'
</script>

<template>
  <section class="mobile section section--gray">
    <div class="page mobile__grid">
      <div class="mobile__copy">
        <p class="eyebrow">手机也能加入</p>
        <h2>照片在手机上。<br />下一秒，在 Mac 上。</h2>
        <p class="mobile__lead">
          不用装 App。手机浏览器打开电脑上的地址，选好照片直接发送；文件和文件夹也能这样传。
        </p>
        <ul class="mobile__list">
          <li><GlyphIcon name="check" :size="19" />手机扫码或打开地址，即用即走</li>
          <li><GlyphIcon name="check" :size="19" />同一局域网内直传，不经过云端</li>
          <li><GlyphIcon name="check" :size="19" />收到的文件默认放进「下载」，位置可改</li>
        </ul>
        <a class="text-link" href="#download">查看安装步骤 <span aria-hidden="true">›</span></a>
      </div>

      <figure class="mobile__figure">
        <div class="mobile__phone">
          <div class="mobile__speaker" aria-hidden="true"></div>
          <img
            :src="mobileShot"
            width="780"
            height="1688"
            loading="lazy"
            alt="手机浏览器里的 DeskFuse 局域网传输页面，显示文字输入与照片上传入口"
          />
        </div>
        <figcaption>手机浏览器中的真实传输页面，无需安装 App。</figcaption>
      </figure>
    </div>
  </section>
</template>

<style scoped>
.mobile__grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(300px, 0.78fr);
  align-items: center;
  gap: clamp(48px, 9vw, 120px);
}

.mobile__copy h2 {
  margin-top: 14px;
  font-size: clamp(2.5rem, 2rem + 2vw, 3.5rem);
  line-height: 1.15;
}

.mobile__lead {
  max-width: 34em;
  margin-top: 24px;
  color: var(--body);
  font-size: 1.125rem;
  line-height: 1.75;
}

.mobile__list {
  display: grid;
  gap: 14px;
  margin: 28px 0 20px;
  padding: 0;
  list-style: none;
}

.mobile__list li {
  display: grid;
  grid-template-columns: 20px minmax(0, 1fr);
  gap: 11px;
  color: var(--ink);
  font-size: 0.9375rem;
  line-height: 1.65;
}

.mobile__list svg {
  margin-top: 3px;
  color: var(--vermilion);
}

.mobile__figure {
  justify-self: center;
  width: min(340px, 100%);
}

.mobile__phone {
  position: relative;
  padding: 10px;
  border: 2px solid #3a3a3c;
  border-radius: 44px;
  background: #1d1d1f;
  box-shadow: 0 26px 54px rgb(0 0 0 / 18%);
}

.mobile__phone img {
  width: 100%;
  height: auto;
  border-radius: 32px;
  background: #fff;
}

.mobile__speaker {
  position: absolute;
  top: 20px;
  left: 50%;
  z-index: 1;
  width: 72px;
  height: 20px;
  border-radius: 980px;
  background: #1d1d1f;
  transform: translateX(-50%);
}

.mobile__figure figcaption {
  margin-top: 24px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.65;
  text-align: center;
}

@media (max-width: 820px) {
  .mobile__grid {
    grid-template-columns: 1fr;
  }

  .mobile__copy {
    max-width: 620px;
  }
}

@media (max-width: 420px) {
  .mobile__figure {
    width: min(300px, 100%);
  }
}
</style>

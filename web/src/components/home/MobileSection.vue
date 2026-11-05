<script setup lang="ts">
import { useDownloads } from '../../composables/useDownloads'
import mobileShot from '../../assets/shots/mobile.webp'

const { arm64 } = useDownloads()
</script>

<template>
  <section class="mobile section">
    <div class="page">
      <div class="mobile__grid">
        <div class="mobile__copy">
          <h2>手机上的照片，<br />扫一下就过来</h2>
          <p>
            不用装 App。手机浏览器打开电脑上的地址，选好照片直接发；文件和文件夹也能这样传。
          </p>
          <ol class="mobile__list">
            <li><span>01</span>手机不用装 App，扫码即用</li>
            <li><span>02</span>同一局域网内直传，不经过云端</li>
            <li><span>03</span>收到的文件默认放进「下载」，接收位置可改</li>
          </ol>
          <div class="mobile__actions">
            <a class="btn btn--primary" :href="arm64.url">下载 DeskFuse</a>
            <a class="mobile__link" href="#download">查看安装步骤 <span aria-hidden="true">→</span></a>
          </div>
        </div>

        <figure class="mobile__plate">
          <div class="mobile__screen">
            <img
              :src="mobileShot"
              width="780"
              height="1688"
              loading="lazy"
              alt="手机浏览器里的 DeskFuse 局域网传输页面，显示照片已上传到 Mac"
            />
          </div>
          <figcaption>
            <b>Fig. 03</b>
            手机浏览器，不用装 App。照片和文字走同一条局域网连接。
            <span>SCAN / SEND / SAVED</span>
          </figcaption>
        </figure>
      </div>
    </div>
  </section>
</template>

<style scoped>
.mobile {
  padding-block: clamp(64px, 8vw, 104px);
  border-top: 1px solid var(--rule-strong);
  background: var(--paper-2);
}

.mobile__grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(300px, 0.72fr);
  align-items: center;
  gap: clamp(36px, 6vw, 88px);
}

.mobile__copy h2 {
  margin-top: 0;
  font-size: clamp(2.5rem, 1.8rem + 2.6vw, 3.9rem);
  line-height: 1.16;
}

.mobile__copy > p {
  max-width: 34em;
  margin-top: 20px;
  color: var(--body);
  font-size: 1.0625rem;
  line-height: 1.85;
}

.mobile__list {
  display: grid;
  gap: 0;
  margin: 26px 0 0;
  padding: 0;
  border-top: 1px solid var(--rule-strong);
  list-style: none;
}

.mobile__list li {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr);
  gap: 10px;
  padding: 10px 0;
  border-bottom: 1px solid var(--rule);
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.7;
}

.mobile__list span {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  line-height: 1.9;
}

.mobile__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 22px;
  margin-top: 26px;
}

.mobile__link {
  padding-bottom: 2px;
  border-bottom: 1px solid var(--vermilion);
  color: var(--vermilion);
  font-size: 0.875rem;
  font-weight: 600;
  text-decoration: none;
}

.mobile__plate {
  justify-self: center;
  width: min(340px, 100%);
  padding: 6px;
  border: 1px solid var(--rule-strong);
  background: var(--paper);
}

.mobile__screen {
  border: 1px solid var(--rule);
  overflow: hidden;
  background: #f3f5f7;
}

.mobile__screen img {
  display: block;
  width: 100%;
  height: auto;
}

.mobile__plate figcaption {
  display: grid;
  gap: 3px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.65;
}

.mobile__plate figcaption b {
  color: var(--vermilion);
  font-family: var(--font-serif-en);
  font-size: 0.875rem;
  font-weight: 500;
}

.mobile__plate figcaption span {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.04em;
}

@media (max-width: 860px) {
  .mobile__grid {
    grid-template-columns: 1fr;
  }

  .mobile__plate {
    justify-self: start;
  }
}

@media (max-width: 640px) {
  .mobile__plate {
    width: min(300px, 100%);
  }
}
</style>

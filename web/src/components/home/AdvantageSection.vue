<script setup lang="ts">
import inputShot from '../../assets/shots/input-wide.webp'
</script>

<template>
  <section class="adv section">
    <div class="page">
      <header class="adv__head">
        <h2>不是把功能<wbr />堆在一起</h2>
        <p>让几台电脑真的像一台：输入跟着人走，内容随手就过去，数据始终留在自己的网络里。</p>
      </header>

      <div class="adv__spread">
        <figure class="adv__plate">
          <img
            :src="inputShot"
            width="1900"
            height="700"
            loading="lazy"
            alt="DeskFuse 键鼠共享界面里的两台 Mac 屏幕排布，光标从相接的边缘穿过去"
          />
          <figcaption>
            <b>Fig. 02</b>
            两块屏幕，按桌上的真实位置排开。光标只从相接的边过去。
            <span>2 SCREENS / 1.0 MS</span>
          </figcaption>
        </figure>

        <div class="adv__essay">
          <p class="adv__pull">屏幕按真实位置摆好，<br />光标从相接的边穿过去。</p>

          <section class="adv__note">
            <h3>数据不过云</h3>
            <p>设备之间在局域网里直连，没有账号，也没有云端中转。键鼠、剪贴板、文件和音频都走同一条加密连接。</p>
            <ul>
              <li>局域网直连</li>
              <li>设备密钥</li>
              <li>加密连接</li>
            </ul>
          </section>

          <section class="adv__note">
            <h3>装好就能用</h3>
            <p>不用注册账号。附近设备点一下，两边确认就配对。配对过的设备可以随时移除。</p>
          </section>
        </div>
      </div>

      <div class="adv__notes">
        <article>
          <span>01</span>
          <h3>远程应用 <em>Beta</em></h3>
          <p>高负载应用留在另一台 Mac，默认开虚拟屏；不是整块屏幕投屏。</p>
        </article>
        <article>
          <span>02</span>
          <h3>剪贴板与文件</h3>
          <p>复制即粘贴，文件夹原样过去；暂存抽屉先接住，再整批拖出去。</p>
        </article>
        <article>
          <span>03</span>
          <h3>配对与密钥</h3>
          <p>每台设备一把密钥，首次连接两边确认；不在同一个网络时，也可以输入 IP 直连。</p>
        </article>
      </div>
    </div>
  </section>
</template>

<style scoped>
.adv {
  border-top: 1px solid var(--rule-strong);
}

.adv__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 44px;
  padding-top: 14px;
}

.adv__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.adv__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.adv__spread {
  display: grid;
  grid-template-columns: minmax(0, 1.2fr) minmax(260px, 0.8fr);
  gap: clamp(32px, 5vw, 72px);
  align-items: start;
}

.adv__plate {
  padding: 6px;
  border: 1px solid var(--rule-strong);
  background: var(--paper);
}

.adv__plate img {
  width: 100%;
  height: auto;
  border: 1px solid var(--rule);
}

.adv__plate figcaption {
  display: grid;
  gap: 3px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.65;
}

.adv__plate figcaption b {
  color: var(--vermilion);
  font-family: var(--font-serif-en);
  font-size: 0.875rem;
  font-weight: 500;
}

.adv__plate figcaption span {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.03em;
}

.adv__essay {
  min-width: 0;
}

.adv__pull {
  padding: 6px 0 22px;
  border-bottom: 1px solid var(--rule-strong);
  font-family: var(--font-display);
  font-size: clamp(1.65rem, 1.2rem + 1.8vw, 2.4rem);
  font-weight: 500;
  letter-spacing: 0;
  line-height: 1.45;
}

.adv__note {
  padding: 20px 0;
  border-bottom: 1px solid var(--rule);
}

.adv__note h3 {
  font-family: var(--font-sans);
  font-size: 0.8125rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.adv__note p {
  margin-top: 8px;
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.8;
}

.adv__note ul {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 18px;
  margin: 14px 0 0;
  padding: 0;
  list-style: none;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.03em;
}

.adv__note li::before {
  margin-right: 6px;
  color: var(--vermilion);
  content: '·';
}

.adv__notes {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0;
  margin-top: 44px;
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule);
}

.adv__notes article {
  padding: 16px 20px 18px 0;
  border-right: 1px solid var(--rule);
}

.adv__notes article + article {
  padding-left: 20px;
}

.adv__notes article:last-child {
  border-right: 0;
}

.adv__notes span {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.05em;
}

.adv__notes h3 {
  margin-top: 6px;
  font-family: var(--font-sans);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: 0;
  line-height: 1.5;
}

.adv__notes em {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  font-style: normal;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.adv__notes p {
  margin-top: 6px;
  color: var(--muted);
  font-size: 0.875rem;
  line-height: 1.7;
}

@media (max-width: 1080px) {
  .adv__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .adv__head h2,
  .adv__head p {
    grid-column: 1;
  }

  .adv__head p {
    max-width: 38em;
  }
}

@media (max-width: 960px) {
  .adv__spread {
    grid-template-columns: 1fr;
  }

  .adv__plate {
    max-width: 720px;
  }
}

@media (max-width: 700px) {
  .adv__notes {
    grid-template-columns: 1fr;
  }

  .adv__notes article,
  .adv__notes article + article {
    padding: 14px 0;
    border-right: 0;
    border-bottom: 1px solid var(--rule);
  }

  .adv__notes article:last-child {
    border-bottom: 0;
  }
}
</style>

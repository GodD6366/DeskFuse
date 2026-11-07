<script setup lang="ts">
import { useDownloads } from '../../composables/useDownloads'
import clipboardShot from '../../assets/shots/clipboard.webp'
import inputShot from '../../assets/shots/input-wide.webp'

const { tag, prerelease } = useDownloads()
</script>

<template>
  <section id="top" class="hero">
    <div class="page hero__intro">
      <h1>DeskFuse</h1>
      <p class="hero__tagline">一套键鼠，连接每台电脑。</p>
      <p class="hero__description">
        光标穿过屏幕，剪贴板和文件跟着走。<br class="hero__break" />
        局域网直连，不必上云。
      </p>
      <div class="hero__actions">
        <a class="btn btn--primary" href="#download">下载 DeskFuse</a>
        <a class="text-link" href="#capabilities">了解 DeskFuse <span aria-hidden="true">›</span></a>
      </div>
      <p class="hero__meta">
        {{ tag }}{{ prerelease ? ' 预发行' : '' }} · 局域网直连 · 设备密钥配对
      </p>
    </div>

    <figure class="hero__stage">
      <div class="hero__canvas">
        <div class="hero__rear">
          <div class="hero__rear-screen">
            <img
              :src="inputShot"
              width="1900"
              height="700"
              alt="DeskFuse 中按真实桌面位置排列的两台电脑屏幕"
            />
          </div>
          <span>键鼠共享</span>
        </div>

        <div class="hero__front">
          <img
            :src="clipboardShot"
            width="2200"
            height="1394"
            fetchpriority="high"
            alt="DeskFuse 剪贴板实际界面，展示文字、图片、历史记录与来源设备"
          />
        </div>

        <div class="hero__local" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6">
            <path d="M8 7h11m-3-3 3 3-3 3M16 17H5m3 3-3-3 3-3" />
          </svg>
          局域网直连
        </div>
      </div>
      <figcaption>真实产品界面。让桌上的电脑，自然协作。</figcaption>
    </figure>
  </section>
</template>

<style scoped>
.hero {
  padding-top: clamp(64px, 7vw, 96px);
  background: var(--paper);
}

.hero__intro {
  position: relative;
  z-index: 2;
  text-align: center;
}

.hero h1 {
  font-size: var(--t-h1);
  line-height: 1.02;
}

.hero__tagline {
  margin-top: 14px;
  font-size: clamp(2rem, 1.45rem + 2.3vw, 3.5rem);
  font-weight: 600;
  line-height: 1.18;
}

.hero__description {
  max-width: 42em;
  margin: 24px auto 0;
  color: var(--body);
  font-size: var(--t-lead);
  line-height: 1.65;
}

.hero__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 12px 26px;
  margin-top: 26px;
}

.hero__meta {
  margin-top: 16px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.6;
}

.hero__stage {
  width: min(1240px, 100% - 48px);
  margin: clamp(48px, 6vw, 76px) auto 0;
  padding: clamp(28px, 4vw, 56px) clamp(22px, 4vw, 52px) 0;
  overflow: hidden;
  border-radius: 12px 12px 0 0;
  background: var(--paper-2);
}

.hero__canvas {
  position: relative;
  height: clamp(420px, 48vw, 600px);
}

.hero__rear {
  position: absolute;
  top: 0;
  left: 0;
  width: 48%;
  padding: 8px;
  border-radius: 16px;
  background: #2c2c2e;
  box-shadow: 0 24px 48px rgb(0 0 0 / 16%);
}

.hero__rear-screen {
  overflow: hidden;
  border-radius: 9px;
  background: #fff;
}

.hero__rear img {
  width: 100%;
  height: auto;
}

.hero__rear > span {
  display: block;
  padding: 10px 8px 5px;
  color: rgb(255 255 255 / 85%);
  font-size: 0.75rem;
  line-height: 1.5;
  text-align: center;
}

.hero__front {
  position: absolute;
  top: 16%;
  right: 0;
  width: 82%;
  padding: 8px;
  border: 1px solid rgb(0 0 0 / 7%);
  border-radius: 14px;
  background: #fff;
  box-shadow: 0 28px 64px rgb(0 0 0 / 15%);
}

.hero__front img {
  width: 100%;
  height: auto;
  border-radius: 7px;
}

.hero__local {
  position: absolute;
  left: 5%;
  bottom: 15%;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 11px 16px;
  border: 1px solid rgb(0 0 0 / 6%);
  border-radius: 980px;
  background: rgb(255 255 255 / 94%);
  box-shadow: 0 10px 28px rgb(0 0 0 / 12%);
  color: var(--ink);
  font-size: 0.8125rem;
  font-weight: 500;
}

.hero__local svg {
  color: var(--vermilion);
}

.hero__stage figcaption {
  padding: 22px 0 24px;
  color: var(--muted);
  font-size: 0.875rem;
  line-height: 1.6;
  text-align: center;
}

@media (prefers-reduced-motion: no-preference) {
  .hero__front {
    animation: product-in 600ms var(--ease) both;
  }

  .hero__rear {
    animation: product-in 600ms 80ms var(--ease) both;
  }

  @keyframes product-in {
    from {
      opacity: 0;
      transform: translateY(18px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
}

@media (max-width: 760px) {
  .hero__stage {
    width: calc(100% - 24px);
    padding: 24px 16px 0;
  }

  .hero__canvas {
    height: clamp(330px, 72vw, 480px);
  }

  .hero__front {
    top: 20%;
    width: 90%;
  }

  .hero__rear {
    width: 56%;
  }

  .hero__local {
    left: 2%;
    bottom: 12%;
  }
}

@media (max-width: 520px) {
  .hero {
    padding-top: 52px;
  }

  .hero h1 {
    font-size: 3.75rem;
  }

  .hero__tagline {
    margin-top: 12px;
    font-size: 2rem;
  }

  .hero__description {
    margin-top: 18px;
    font-size: 1.125rem;
  }

  .hero__break {
    display: none;
  }

  .hero__stage {
    margin-top: 38px;
    padding: 20px 12px 0;
  }

  .hero__canvas {
    height: 310px;
  }

  .hero__rear {
    width: 68%;
    padding: 5px;
    border-radius: 10px;
  }

  .hero__rear > span {
    padding: 6px 4px 3px;
    font-size: 0.625rem;
  }

  .hero__front {
    top: 24%;
    width: 96%;
    padding: 5px;
    border-radius: 10px;
  }

  .hero__local {
    bottom: 11%;
    padding: 8px 11px;
    font-size: 0.6875rem;
  }

  .hero__stage figcaption {
    padding: 16px 8px 18px;
    font-size: 0.75rem;
  }
}

@media (max-width: 350px) {
  .hero h1 {
    font-size: 3.25rem;
  }

  .hero__tagline {
    font-size: 1.75rem;
  }

  .hero__canvas {
    height: 270px;
  }
}
</style>

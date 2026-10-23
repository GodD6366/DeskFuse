<template>
  <div class="clip" aria-hidden="true">
    <aside class="clip__side">
      <div class="clip__lights"><i></i><i></i><i></i></div>
      <div class="clip__brand">DeskFuse</div>
      <div class="clip__side-label">剪贴板</div>
      <div class="clip__nav">
        <span class="is-active"><i class="nav-dot"></i>全部 <em>326</em></span>
        <span><i class="nav-dot"></i>图片 <em>48</em></span>
        <span><i class="nav-dot"></i>文件 <em>24</em></span>
        <span><i class="nav-dot"></i>我的列表 <em>6</em></span>
      </div>
      <div class="clip__side-foot"><i></i>这台 Mac · 运行中</div>
    </aside>

    <div class="clip__main">
      <div class="clip__head">
        <div>
          <b>剪贴板</b>
          <span>所有设备之间，自由流转</span>
        </div>
        <span class="clip__sync"><i></i>已同步 2 台</span>
      </div>

      <div class="clip__search">
        <span>搜索剪贴板</span>
        <kbd>⌘ F</kbd>
      </div>

      <div class="clip__group">
        <span>所有设备</span><em>326 条</em>
      </div>

      <div class="clip__grid">
        <div class="clip-card is-active">
          <p>https://deskfuse.godd.cc</p>
          <span>13:24</span>
          <i class="card-link"></i>
        </div>
        <div class="clip-card">
          <p>设计稿评审改到 15:00</p>
          <span>13:23</span>
          <i class="card-line"></i>
        </div>
        <div class="clip-card">
          <p>IMG_2887.JPG</p>
          <div class="clip-thumb"></div>
          <span>13:21</span>
        </div>
        <div class="clip-card">
          <p>release / v0.7.2</p>
          <span>13:20</span>
          <i class="card-line"></i>
        </div>
        <div class="clip-card">
          <p># 产品设计<br /># 用户反馈</p>
          <span>13:17</span>
          <i class="card-hash"></i>
        </div>
        <div class="clip-card">
          <p>会议纪要：<br />确定下一版范围</p>
          <span>13:16</span>
          <i class="card-line"></i>
        </div>
      </div>
    </div>

    <aside class="clip__preview">
      <div class="clip__preview-head">
        <span>链接</span>
        <i>可复制</i>
      </div>
      <div class="clip__preview-body">
        <p class="clip__url">https://<br />deskfuse.godd.cc</p>
        <div class="clip__meta">
          <span>来源</span><b>这台 Mac</b>
          <span>大小</span><b>25 bytes</b>
          <span>添加</span><b>13:24</b>
        </div>
      </div>
      <div class="clip__preview-foot">
        <button type="button" tabindex="-1">复制</button>
        <span>···</span>
      </div>
    </aside>
  </div>
</template>

<style scoped>
.clip {
  display: grid;
  grid-template-columns: 19% 47% 34%;
  width: 100%;
  height: 100%;
  color: #e8f1ed;
  font-family: var(--sans);
  background: #0a1513;
}

.clip__side {
  display: flex;
  flex-direction: column;
  padding: 2.3cqw 1.5cqw;
  border-right: 1px solid rgba(207, 255, 243, 0.09);
  background: rgba(207, 255, 243, 0.018);
}

.clip__lights {
  display: flex;
  gap: 0.5cqw;
  margin-bottom: 2cqw;
}

.clip__lights i {
  width: 0.75cqw;
  height: 0.75cqw;
  border-radius: 50%;
  background: rgba(255, 106, 60, 0.85);
}

.clip__lights i:nth-child(2) {
  background: rgba(216, 183, 94, 0.85);
}

.clip__lights i:nth-child(3) {
  background: rgba(89, 243, 218, 0.85);
}

.clip__brand {
  font-family: var(--display);
  font-size: clamp(9px, 1.8cqw, 17px);
  line-height: 1;
}

.clip__side-label {
  margin-top: 0.8cqw;
  color: rgba(148, 170, 163, 0.6);
  font-size: clamp(6px, 0.95cqw, 9px);
}

.clip__nav {
  display: flex;
  flex-direction: column;
  gap: 0.35cqw;
  margin-top: 2.4cqw;
}

.clip__nav span {
  display: flex;
  align-items: center;
  gap: 0.7cqw;
  padding: 0.7cqw 0.8cqw;
  border-radius: 0.55cqw;
  color: rgba(148, 170, 163, 0.9);
  font-size: clamp(6px, 1.05cqw, 10px);
}

.clip__nav span.is-active {
  background: rgba(89, 243, 218, 0.1);
  color: #e8f1ed;
}

.clip__nav em {
  margin-left: auto;
  color: rgba(148, 170, 163, 0.5);
  font-family: var(--mono);
  font-size: 0.85em;
  font-style: normal;
}

.nav-dot {
  width: 0.55cqw;
  height: 0.55cqw;
  border: 1px solid currentColor;
  border-radius: 50%;
  opacity: 0.7;
}

.clip__nav span.is-active .nav-dot {
  border-color: #59f3da;
  background: #59f3da;
  opacity: 1;
}

.clip__side-foot {
  display: flex;
  align-items: center;
  gap: 0.6cqw;
  margin-top: auto;
  color: rgba(148, 170, 163, 0.65);
  font-size: clamp(5.5px, 0.9cqw, 8.5px);
}

.clip__side-foot i {
  width: 0.55cqw;
  height: 0.55cqw;
  border-radius: 50%;
  background: #59f3da;
}

.clip__main {
  display: flex;
  flex-direction: column;
  min-width: 0;
  padding: 2.2cqw 2cqw 1.6cqw;
}

.clip__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1cqw;
}

.clip__head > div {
  display: flex;
  flex-direction: column;
  gap: 0.2cqw;
}

.clip__head b {
  font-size: clamp(9px, 1.7cqw, 16px);
  font-weight: 600;
}

.clip__head span {
  color: rgba(148, 170, 163, 0.65);
  font-size: clamp(5.5px, 0.9cqw, 8.5px);
}

.clip__sync {
  display: inline-flex;
  align-items: center;
  gap: 0.5cqw;
  padding: 0.5cqw 0.8cqw;
  border: 1px solid rgba(89, 243, 218, 0.18);
  border-radius: 999px;
  color: #59f3da !important;
  font-size: clamp(5.5px, 0.85cqw, 8px) !important;
  white-space: nowrap;
}

.clip__sync i {
  width: 0.55cqw;
  height: 0.55cqw;
  border-radius: 50%;
  background: #59f3da;
  box-shadow: 0 0 0.7cqw rgba(89, 243, 218, 0.5);
  animation: clip-pulse 2.4s ease-in-out infinite;
}

.clip__search {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 1.5cqw;
  padding: 0.9cqw 1cqw;
  border: 1px solid rgba(207, 255, 243, 0.08);
  border-radius: 0.6cqw;
  color: rgba(148, 170, 163, 0.65);
  font-size: clamp(5.5px, 0.9cqw, 8.5px);
  background: rgba(207, 255, 243, 0.025);
}

.clip__search kbd {
  font-size: 0.9em;
  opacity: 0.7;
}

.clip__group {
  display: flex;
  align-items: center;
  gap: 0.8cqw;
  margin: 1.4cqw 0 0.8cqw;
  color: rgba(233, 243, 238, 0.8);
  font-size: clamp(5.5px, 0.9cqw, 8.5px);
}

.clip__group em {
  margin-left: auto;
  color: rgba(148, 170, 163, 0.55);
  font-family: var(--mono);
  font-size: 0.85em;
  font-style: normal;
}

.clip__grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75cqw;
  min-height: 0;
}

.clip-card {
  position: relative;
  min-height: 0;
  padding: 1.1cqw 1cqw;
  overflow: hidden;
  border: 1px solid rgba(207, 255, 243, 0.07);
  border-radius: 0.7cqw;
  background: rgba(207, 255, 243, 0.035);
}

.clip-card.is-active {
  border-color: rgba(89, 243, 218, 0.45);
  background: rgba(89, 243, 218, 0.065);
}

.clip-card p {
  display: -webkit-box;
  overflow: hidden;
  color: rgba(233, 243, 238, 0.92);
  font-size: clamp(6px, 1cqw, 9.5px);
  line-height: 1.45;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.clip-card > span {
  position: absolute;
  right: 0.9cqw;
  bottom: 0.7cqw;
  color: rgba(148, 170, 163, 0.5);
  font-family: var(--mono);
  font-size: clamp(4.5px, 0.72cqw, 7px);
}

.clip-thumb {
  width: 3.5cqw;
  height: 2.3cqw;
  margin-top: 0.8cqw;
  border-radius: 0.4cqw;
  background: linear-gradient(135deg, #59f3da, #526dff 55%, #ff6a3c);
  opacity: 0.8;
}

.card-link,
.card-line {
  position: absolute;
  right: 0.9cqw;
  bottom: 0.8cqw;
  width: 1.2cqw;
  height: 1.2cqw;
  border: 1px solid rgba(89, 243, 218, 0.35);
  border-radius: 50%;
  opacity: 0.6;
}

.card-line {
  border-radius: 0;
  border-width: 0 0 1px 1px;
}

.card-hash {
  position: absolute;
  right: 0.9cqw;
  bottom: 0.8cqw;
  width: 1.1cqw;
  height: 1.1cqw;
  opacity: 0.45;
  background:
    linear-gradient(90deg, transparent 42%, #59f3da 42% 58%, transparent 58%),
    linear-gradient(0deg, transparent 42%, #59f3da 42% 58%, transparent 58%);
}

.clip__preview {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border-left: 1px solid rgba(207, 255, 243, 0.09);
  background: rgba(3, 10, 8, 0.25);
}

.clip__preview-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1.4cqw 1.2cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.08);
  color: rgba(148, 170, 163, 0.7);
  font-size: clamp(5.5px, 0.82cqw, 8px);
}

.clip__preview-head i {
  padding: 0.25cqw 0.5cqw;
  border-radius: 999px;
  background: rgba(89, 243, 218, 0.1);
  color: #59f3da;
  font-style: normal;
}

.clip__preview-body {
  display: flex;
  flex: 1;
  flex-direction: column;
  padding: 2.4cqw 1.4cqw;
}

.clip__url {
  color: #e8f1ed;
  font-family: var(--mono);
  font-size: clamp(7px, 1.35cqw, 13px);
  line-height: 1.4;
  word-break: break-all;
}

.clip__meta {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 0.5cqw 1cqw;
  margin-top: auto;
  padding-top: 1.2cqw;
  color: rgba(148, 170, 163, 0.55);
  font-size: clamp(5px, 0.78cqw, 7.5px);
}

.clip__meta b {
  color: rgba(233, 243, 238, 0.75);
  font-weight: 500;
}

.clip__preview-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1.2cqw;
  border-top: 1px solid rgba(207, 255, 243, 0.08);
}

.clip__preview-foot button {
  padding: 0.55cqw 1cqw;
  border-radius: 0.5cqw;
  background: #59f3da;
  color: #06100e;
  font-size: clamp(5.5px, 0.82cqw, 8px);
  font-weight: 600;
}

.clip__preview-foot span {
  color: rgba(148, 170, 163, 0.5);
  font-size: 0.9cqw;
}

@keyframes clip-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.25;
  }
}

@media (max-width: 720px) {
  .clip {
    grid-template-columns: 23% 77%;
  }

  .clip__preview {
    display: none;
  }

  .clip__preview-head,
  .clip__nav span:nth-child(4),
  .clip__side-label {
    display: none;
  }

  .clip__meta {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .clip__sync i {
    animation: none;
  }
}
</style>

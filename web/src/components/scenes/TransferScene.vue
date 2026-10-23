<template>
  <div class="transfer" aria-hidden="true">
    <div class="transfer__top">
      <div class="transfer__title">
        <b>分享与传输</b>
        <span><i></i>2 台设备在线</span>
      </div>
      <div class="transfer__tabs">
        <span class="is-active">发送</span>
        <span>队列</span>
        <span>手机</span>
      </div>
      <span class="transfer__action">选择文件</span>
    </div>

    <div class="transfer__body">
      <div class="transfer__main">
        <div class="transfer__drop">
          <span class="transfer__drop-plus">+</span>
          <b>拖入文件或文件夹</b>
          <span>按需传输 · 保留名称与结构</span>
        </div>

        <div class="transfer__queue-head">
          <span>传输队列</span>
          <em>2 项</em>
        </div>

        <div class="transfer__row">
          <span class="file-icon is-orange">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <path d="M14 2v6h6" />
            </svg>
          </span>
          <div class="transfer__row-main">
            <b>项目备份.zip</b>
            <span>文件夹结构保留 · 正在传输</span>
            <div class="transfer__progress"><i></i></div>
          </div>
          <span class="transfer__state">62%</span>
        </div>

        <div class="transfer__row is-done">
          <span class="file-icon is-aqua">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7">
              <rect x="3" y="3" width="18" height="18" rx="2.5" />
              <circle cx="9" cy="9" r="1.8" />
              <path d="m21 15-4-4L6 20" />
            </svg>
          </span>
          <div class="transfer__row-main">
            <b>设计稿 · 首页.png</b>
            <span>2.4 MB · 已写入对端剪贴板</span>
          </div>
          <span class="transfer__state">完成</span>
        </div>

        <div class="transfer__foot">
          <span>缓存有上限 · 清理有回执</span>
          <span class="transfer__foot-route">LAN / 对端直写</span>
        </div>
      </div>

      <div class="transfer__shelf-wrap">
        <span class="transfer__route" aria-hidden="true"><i></i></span>
        <div class="shelf">
          <div class="shelf__head">
            <span><i></i>暂存抽屉</span>
            <em>3 项</em>
          </div>
          <div class="shelf__grid">
            <div class="shelf__file f-orange">
              <span>ZIP</span>
              <i>项目备份</i>
            </div>
            <div class="shelf__file f-aqua">
              <span>PNG</span>
              <i>首页</i>
            </div>
            <div class="shelf__file f-blue">
              <span>FIG</span>
              <i>设计源文件</i>
            </div>
          </div>
          <div class="shelf__foot">
            <span>240 × 240</span>
            <b>整批拖出</b>
          </div>
        </div>
        <span class="transfer__shelf-caption">悬浮于桌面 <!-- -->·<!-- --> 拖入即暂存</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.transfer {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  color: #e8f1ed;
  font-family: var(--sans);
  background: #0a1513;
}

.transfer__top {
  display: flex;
  align-items: center;
  gap: 2cqw;
  padding: 1.7cqw 2cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.09);
}

.transfer__title {
  display: flex;
  flex-direction: column;
  gap: 0.3cqw;
}

.transfer__title b {
  font-size: clamp(8px, 1.45cqw, 14px);
  font-weight: 600;
}

.transfer__title span {
  display: inline-flex;
  align-items: center;
  gap: 0.45cqw;
  color: rgba(148, 170, 163, 0.6);
  font-size: clamp(5px, 0.78cqw, 7.5px);
}

.transfer__title i {
  width: 0.5cqw;
  height: 0.5cqw;
  border-radius: 50%;
  background: #59f3da;
}

.transfer__tabs {
  display: flex;
  gap: 0.2cqw;
  margin-left: auto;
  padding: 0.25cqw;
  border: 1px solid rgba(207, 255, 243, 0.08);
  border-radius: 999px;
}

.transfer__tabs span {
  padding: 0.45cqw 0.9cqw;
  border-radius: 999px;
  color: rgba(148, 170, 163, 0.65);
  font-size: clamp(5.5px, 0.82cqw, 8px);
}

.transfer__tabs .is-active {
  background: rgba(207, 255, 243, 0.08);
  color: #e8f1ed;
}

.transfer__action {
  padding: 0.55cqw 1cqw;
  border: 1px solid rgba(89, 243, 218, 0.3);
  color: #59f3da;
  font-size: clamp(5.5px, 0.82cqw, 8px);
}

.transfer__body {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 39%;
  flex: 1;
  min-height: 0;
}

.transfer__main {
  display: flex;
  flex-direction: column;
  min-width: 0;
  padding: 1.8cqw 2cqw 1.4cqw;
}

.transfer__drop {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.3cqw;
  min-height: 7.2cqw;
  border: 1px dashed rgba(89, 243, 218, 0.2);
  border-radius: 0.8cqw;
  color: rgba(148, 170, 163, 0.6);
  background: rgba(89, 243, 218, 0.018);
}

.transfer__drop b {
  color: rgba(233, 243, 238, 0.8);
  font-size: clamp(6px, 0.95cqw, 9px);
  font-weight: 500;
}

.transfer__drop span:last-child {
  font-size: clamp(5px, 0.76cqw, 7.5px);
}

.transfer__drop-plus {
  display: grid;
  place-items: center;
  width: 1.6cqw;
  height: 1.6cqw;
  color: #59f3da;
  font-family: var(--mono);
  font-size: clamp(7px, 1.2cqw, 12px);
}

.transfer__queue-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin: 1.7cqw 0 0.8cqw;
  color: rgba(233, 243, 238, 0.8);
  font-size: clamp(5.5px, 0.86cqw, 8.5px);
}

.transfer__queue-head em {
  color: rgba(148, 170, 163, 0.5);
  font-family: var(--mono);
  font-style: normal;
  font-size: 0.85em;
}

.transfer__row {
  display: flex;
  align-items: center;
  gap: 1cqw;
  padding: 0.9cqw;
  border: 1px solid rgba(207, 255, 243, 0.09);
  border-radius: 0.65cqw;
  background: rgba(207, 255, 243, 0.03);
}

.transfer__row + .transfer__row {
  margin-top: 0.65cqw;
}

.file-icon {
  display: grid;
  place-items: center;
  flex: none;
  width: 2.4cqw;
  height: 2.4cqw;
  border: 1px solid rgba(255, 106, 60, 0.3);
  color: #ffb59e;
}

.file-icon svg {
  width: 1.3cqw;
  height: 1.3cqw;
}

.file-icon.is-aqua {
  border-color: rgba(89, 243, 218, 0.25);
  color: #59f3da;
}

.transfer__row-main {
  display: flex;
  flex-direction: column;
  gap: 0.25cqw;
  min-width: 0;
}

.transfer__row-main b {
  overflow: hidden;
  color: rgba(233, 243, 238, 0.9);
  font-size: clamp(5.8px, 0.92cqw, 9px);
  font-weight: 500;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.transfer__row-main span {
  color: rgba(148, 170, 163, 0.55);
  font-size: clamp(4.5px, 0.72cqw, 7px);
}

.transfer__progress {
  width: 100%;
  height: 0.22cqw;
  margin-top: 0.25cqw;
  overflow: hidden;
  border-radius: 99px;
  background: rgba(207, 255, 243, 0.08);
}

.transfer__progress i {
  display: block;
  width: 62%;
  height: 100%;
  background: #59f3da;
  box-shadow: 0 0 0.8cqw rgba(89, 243, 218, 0.4);
  animation: transfer-fill 3.2s ease-in-out infinite;
}

.transfer__state {
  margin-left: auto;
  color: #59f3da;
  font-family: var(--mono);
  font-size: clamp(4.8px, 0.76cqw, 7.5px);
  white-space: nowrap;
}

.transfer__row.is-done .transfer__state {
  color: rgba(148, 170, 163, 0.65);
}

.transfer__foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: auto;
  padding-top: 1.1cqw;
  border-top: 1px solid rgba(207, 255, 243, 0.07);
  color: rgba(148, 170, 163, 0.45);
  font-size: clamp(4.5px, 0.7cqw, 7px);
}

.transfer__foot-route {
  font-family: var(--mono);
  letter-spacing: 0.04em;
}

.transfer__shelf-wrap {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1cqw;
  min-width: 0;
  border-left: 1px solid rgba(207, 255, 243, 0.08);
  background:
    radial-gradient(circle at 50% 46%, rgba(89, 243, 218, 0.08), transparent 60%),
    rgba(3, 10, 8, 0.28);
}

.transfer__route {
  position: absolute;
  top: 50%;
  left: -0.2cqw;
  width: 3.2cqw;
  height: 1px;
  background: rgba(89, 243, 218, 0.25);
}

.transfer__route i {
  position: absolute;
  top: -0.18cqw;
  left: 0;
  width: 0.45cqw;
  height: 0.45cqw;
  background: #ff6a3c;
  box-shadow: 0 0 0.7cqw rgba(255, 106, 60, 0.6);
  animation: shelf-packet 3s var(--ease-in-out) infinite;
}

.shelf {
  width: 67%;
  padding: 1.2cqw;
  border: 1px solid rgba(207, 255, 243, 0.17);
  border-radius: 0.9cqw;
  background: rgba(10, 21, 19, 0.92);
  box-shadow: 0 1.5cqw 3cqw rgba(0, 0, 0, 0.35);
  outline: 1px dashed rgba(89, 243, 218, 0.24);
  outline-offset: 0.7cqw;
}

.shelf__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  color: rgba(233, 243, 238, 0.78);
  font-size: clamp(5.5px, 0.82cqw, 8px);
}

.shelf__head span {
  display: inline-flex;
  align-items: center;
  gap: 0.5cqw;
}

.shelf__head i {
  width: 0.5cqw;
  height: 0.5cqw;
  border-radius: 50%;
  border: 1px solid #59f3da;
}

.shelf__head em {
  color: rgba(148, 170, 163, 0.5);
  font-family: var(--mono);
  font-style: normal;
}

.shelf__grid {
  display: flex;
  gap: 0.7cqw;
  margin-top: 1.1cqw;
}

.shelf__file {
  display: flex;
  flex: 1;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.5cqw;
  height: 4.2cqw;
  border-radius: 0.6cqw;
  font-size: clamp(4.5px, 0.65cqw, 6.5px);
}

.shelf__file span {
  color: #06100e;
  font-family: var(--mono);
  font-size: 0.8em;
  font-weight: 700;
}

.shelf__file i {
  color: rgba(6, 16, 14, 0.7);
  font-style: normal;
}

.f-orange {
  background: linear-gradient(135deg, #ff9f80, #ff6a3c);
}

.f-aqua {
  background: linear-gradient(135deg, #a8fff1, #59f3da);
}

.f-blue {
  background: linear-gradient(135deg, #aebaff, #6e84ff);
}

.shelf__foot {
  display: flex;
  justify-content: space-between;
  margin-top: 1.1cqw;
  padding-top: 0.7cqw;
  border-top: 1px solid rgba(207, 255, 243, 0.08);
  color: rgba(148, 170, 163, 0.5);
  font-family: var(--mono);
  font-size: clamp(4.4px, 0.65cqw, 6.5px);
}

.shelf__foot b {
  color: #59f3da;
  font-weight: 500;
}

.transfer__shelf-caption {
  color: rgba(148, 170, 163, 0.45);
  font-size: clamp(4.8px, 0.72cqw, 7px);
}

@keyframes transfer-fill {
  0% {
    width: 54%;
    opacity: 0.7;
  }
  50% {
    width: 68%;
    opacity: 1;
  }
  100% {
    width: 54%;
    opacity: 0.7;
  }
}

@keyframes shelf-packet {
  0% {
    left: 0;
    opacity: 0;
  }
  15% {
    opacity: 1;
  }
  80% {
    opacity: 1;
  }
  100% {
    left: calc(100% - 0.45cqw);
    opacity: 0;
  }
}

@media (max-width: 720px) {
  .transfer__tabs span:not(.is-active),
  .transfer__route,
  .transfer__shelf-caption {
    display: none;
  }

  .transfer__body {
    grid-template-columns: minmax(0, 1fr);
  }

  .transfer__shelf-wrap {
    display: none;
  }

  .transfer__main {
    padding: 1.3cqw;
  }
}

@media (prefers-reduced-motion: reduce) {
  .transfer__progress i,
  .transfer__route i {
    animation: none;
  }
}
</style>

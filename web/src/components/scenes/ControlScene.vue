<template>
  <div class="control" aria-hidden="true">
    <div class="control__top">
      <div class="control__title">
        <b>当前屏幕排布</b>
        <span>本机主控已开启</span>
      </div>
      <div class="control__tools">
        <span class="control__tool">+ 添加屏幕</span>
        <span class="control__toggle"><i></i>共享键鼠</span>
        <span class="control__reclaim">紧急收回</span>
      </div>
    </div>

    <div class="control__canvas">
      <div class="control__grid" aria-hidden="true"></div>
      <span class="control__ordinate control__ordinate--x">X</span>
      <span class="control__ordinate control__ordinate--y">Y</span>

      <div class="monitor monitor--remote">
        <div class="monitor__bar"><i></i><i></i><i></i></div>
        <div class="monitor__label">
          <b>02</b>
          <strong>MacBook Pro 14</strong>
          <span>1512 × 982 pt</span>
        </div>
        <div class="monitor__corner"></div>
      </div>

      <div class="monitor monitor--local">
        <div class="monitor__bar"><i></i><i></i><i></i></div>
        <div class="monitor__label">
          <b>01</b>
          <strong>本机 · DeskFuse</strong>
          <span>2048 × 1152 pt</span>
        </div>
        <div class="monitor__corner"></div>
      </div>

      <svg class="control__cursor" viewBox="0 0 24 24" width="12" height="12">
        <path
          d="M4 2l16 11-7.2 1.4L9 22z"
          fill="#ffffff"
          stroke="#06100e"
          stroke-width="1.4"
          stroke-linejoin="round"
        />
      </svg>

      <span class="control__bridge"><i></i></span>

      <div class="control__status">
        <span><i></i>连接质量：良好</span>
        <span>真实显示器布局 · 松手自动贴合</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.control {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  color: #e8f1ed;
  font-family: var(--sans);
  background: #0a1513;
}

.control__top {
  display: flex;
  align-items: center;
  gap: 2cqw;
  padding: 1.5cqw 2cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.09);
}

.control__title {
  display: flex;
  flex-direction: column;
  gap: 0.25cqw;
}

.control__title b {
  font-size: clamp(8px, 1.45cqw, 14px);
  font-weight: 600;
}

.control__title span {
  color: rgba(148, 170, 163, 0.6);
  font-size: clamp(5px, 0.76cqw, 7.5px);
}

.control__tools {
  display: flex;
  align-items: center;
  gap: 0.6cqw;
  margin-left: auto;
}

.control__tool {
  padding: 0.55cqw 0.8cqw;
  border: 1px solid rgba(89, 243, 218, 0.28);
  color: #59f3da;
  font-size: clamp(5px, 0.76cqw, 7.5px);
  white-space: nowrap;
}

.control__toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.5cqw;
  padding: 0.55cqw 0.8cqw;
  border: 1px solid rgba(207, 255, 243, 0.1);
  color: rgba(233, 243, 238, 0.8);
  font-size: clamp(5px, 0.76cqw, 7.5px);
  white-space: nowrap;
}

.control__toggle i {
  position: relative;
  width: 1.4cqw;
  height: 0.8cqw;
  border-radius: 99px;
  background: #59f3da;
}

.control__toggle i::after {
  content: "";
  position: absolute;
  top: 0.13cqw;
  right: 0.13cqw;
  width: 0.54cqw;
  height: 0.54cqw;
  border-radius: 50%;
  background: #06100e;
}

.control__reclaim {
  padding: 0.55cqw 0.8cqw;
  background: rgba(255, 106, 60, 0.9);
  color: #160906;
  font-size: clamp(5px, 0.76cqw, 7.5px);
  font-weight: 600;
  white-space: nowrap;
}

.control__canvas {
  position: relative;
  flex: 1;
  min-height: 0;
  margin: 1.3cqw 1.6cqw 1.4cqw;
  overflow: hidden;
  border: 1px solid rgba(207, 255, 243, 0.08);
  border-radius: 0.8cqw;
  background: #08120f;
}

.control__grid {
  position: absolute;
  inset: 0;
  background-image: radial-gradient(rgba(207, 255, 243, 0.18) 0.55px, transparent 0.65px);
  background-size: 1.7cqw 1.7cqw;
  opacity: 0.55;
}

.control__ordinate {
  position: absolute;
  color: rgba(148, 170, 163, 0.35);
  font-family: var(--mono);
  font-size: clamp(4px, 0.6cqw, 6px);
}

.control__ordinate--x {
  right: 0.7cqw;
  bottom: 0.4cqw;
}

.control__ordinate--y {
  top: 0.6cqw;
  left: 0.6cqw;
}

.monitor {
  position: absolute;
  overflow: hidden;
  border-radius: 0.55cqw;
  background: rgba(10, 21, 19, 0.68);
}

.monitor--local {
  top: 19%;
  left: 9%;
  width: 43%;
  height: 49%;
  border: 1.5px solid rgba(89, 243, 218, 0.85);
  box-shadow: 0 1.2cqw 3cqw rgba(89, 243, 218, 0.1);
}

.monitor--remote {
  right: 10%;
  bottom: 16%;
  width: 36%;
  height: 43%;
  border: 1.5px solid rgba(255, 106, 60, 0.8);
  box-shadow: 0 1.2cqw 3cqw rgba(255, 106, 60, 0.08);
}

.monitor__bar {
  display: flex;
  align-items: center;
  gap: 0.35cqw;
  height: 1.5cqw;
  padding-inline: 0.6cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.06);
  background: rgba(207, 255, 243, 0.025);
}

.monitor__bar i {
  width: 0.38cqw;
  height: 0.38cqw;
  border-radius: 50%;
  background: rgba(148, 170, 163, 0.3);
}

.monitor__label {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.15cqw;
  height: calc(100% - 1.5cqw);
  text-align: center;
}

.monitor__label b {
  color: #59f3da;
  font-family: var(--mono);
  font-size: clamp(7px, 1.5cqw, 14px);
  font-weight: 500;
}

.monitor--remote .monitor__label b {
  color: #ff8a64;
}

.monitor__label strong {
  color: rgba(233, 243, 238, 0.9);
  font-size: clamp(5.5px, 0.85cqw, 8px);
  font-weight: 500;
  white-space: nowrap;
}

.monitor__label span {
  color: rgba(148, 170, 163, 0.55);
  font-family: var(--mono);
  font-size: clamp(4px, 0.62cqw, 6px);
}

.monitor__corner {
  position: absolute;
  right: 0.45cqw;
  bottom: 0.45cqw;
  width: 0.8cqw;
  height: 0.8cqw;
  border-right: 1px solid currentColor;
  border-bottom: 1px solid currentColor;
  color: rgba(207, 255, 243, 0.4);
}

.control__cursor {
  position: absolute;
  z-index: 2;
  top: 43%;
  left: 22%;
  filter: drop-shadow(0 0.2cqw 0.35cqw rgba(0, 0, 0, 0.45));
  animation: cursor-cross 6s var(--ease-in-out) infinite;
}

.control__bridge {
  position: absolute;
  top: 45%;
  left: 52%;
  width: 10%;
  height: 1px;
  background: rgba(89, 243, 218, 0.2);
}

.control__bridge i {
  position: absolute;
  top: -0.18cqw;
  left: 0;
  width: 0.45cqw;
  height: 0.45cqw;
  border-radius: 50%;
  background: #ff6a3c;
  box-shadow: 0 0 0.7cqw rgba(255, 106, 60, 0.6);
  animation: bridge-packet 6s var(--ease-in-out) infinite;
}

.control__status {
  position: absolute;
  right: 0.8cqw;
  bottom: 0.7cqw;
  left: 0.8cqw;
  display: flex;
  align-items: center;
  justify-content: space-between;
  color: rgba(148, 170, 163, 0.5);
  font-size: clamp(4.4px, 0.66cqw, 6.5px);
}

.control__status span:first-child {
  display: inline-flex;
  align-items: center;
  gap: 0.45cqw;
  color: rgba(89, 243, 218, 0.75);
}

.control__status i {
  width: 0.45cqw;
  height: 0.45cqw;
  border-radius: 50%;
  background: #59f3da;
}

@keyframes cursor-cross {
  0%,
  12% {
    top: 43%;
    left: 22%;
  }
  45%,
  55% {
    top: 48%;
    left: 56%;
  }
  88%,
  100% {
    top: 43%;
    left: 22%;
  }
}

@keyframes bridge-packet {
  0%,
  20% {
    left: 0;
    opacity: 0;
  }
  30% {
    opacity: 1;
  }
  55% {
    left: calc(100% - 0.45cqw);
    opacity: 1;
  }
  66%,
  100% {
    left: calc(100% - 0.45cqw);
    opacity: 0;
  }
}

@media (max-width: 720px) {
  .control__tool,
  .control__toggle {
    display: none;
  }

  .control__title span {
    display: none;
  }

  .control__status span:last-child {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .control__cursor,
  .control__bridge i {
    animation: none;
  }
}
</style>

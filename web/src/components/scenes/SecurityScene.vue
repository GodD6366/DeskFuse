<template>
  <div class="security" aria-hidden="true">
    <div class="security__top">
      <div class="security__title">
        <b>设备与安全</b>
        <span>本地链路 · 已认证</span>
      </div>
      <span class="security__badge">QUIC</span>
      <span class="security__status"><i></i>仅可信设备</span>
    </div>

    <div class="security__body">
      <div class="topology">
        <div class="topology__device">
          <span class="topology__icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
              <rect x="3" y="4" width="18" height="12" rx="2" />
              <path d="M8 20h8M12 16v4" />
            </svg>
          </span>
          <b>这台 Mac</b>
          <span>身份已确认</span>
          <i>01</i>
        </div>

        <div class="topology__link">
          <span class="topology__wire"></span>
          <span class="topology__packet p1"></span>
          <span class="topology__packet p2"></span>
          <span class="topology__lock">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <rect x="4" y="10" width="16" height="11" rx="2" />
              <path d="M8 10V7a4 4 0 0 1 8 0v3" />
            </svg>
          </span>
          <span class="topology__link-label">配对确认 · 设备间加密</span>
        </div>

        <div class="topology__device">
          <span class="topology__icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
              <rect x="2" y="4" width="20" height="13" rx="2" />
              <path d="M8 21h8M12 17v4" />
            </svg>
          </span>
          <b>MacBook Pro 14</b>
          <span>身份已确认</span>
          <i>02</i>
        </div>

        <div class="topology__cloud">
          <span class="topology__cloud-node">云端</span>
          <span class="topology__cloud-line"></span>
          <span class="topology__cloud-state">默认关闭</span>
        </div>

        <span class="topology__coordinate x">42424</span>
        <span class="topology__coordinate y">LAN</span>
      </div>

      <div class="log">
        <div class="log__head">
          <span>运行记录</span>
          <em>仅元数据</em>
        </div>
        <div class="log__body">
          <p><i>01</i><span>pair.confirmed</span><b>peer#1</b></p>
          <p><i>02</i><span>quic.handshake</span><b>ok</b></p>
          <p><i>03</i><span>clipboard.sync</span><b>hash 8f3a</b></p>
          <p><i>04</i><span>content.log</span><b class="muted">never</b></p>
        </div>
        <div class="log__foot">
          <span>不写入剪贴板正文</span>
          <i></i>
        </div>
      </div>
    </div>

    <div class="security__foot">
      <span><i></i>对端确认配对</span>
      <span><i></i>公网通道默认关闭</span>
      <span><i></i>日志仅元数据与哈希</span>
    </div>
  </div>
</template>

<style scoped>
.security {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  color: #e8f1ed;
  font-family: var(--sans);
  background: #0a1513;
}

.security__top {
  display: flex;
  align-items: center;
  gap: 1cqw;
  padding: 1.6cqw 2cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.09);
}

.security__title {
  display: flex;
  flex-direction: column;
  gap: 0.25cqw;
}

.security__title b {
  font-size: clamp(8px, 1.45cqw, 14px);
  font-weight: 600;
}

.security__title span {
  color: rgba(148, 170, 163, 0.6);
  font-size: clamp(5px, 0.76cqw, 7.5px);
}

.security__badge {
  margin-left: auto;
  padding: 0.4cqw 0.75cqw;
  border: 1px solid rgba(89, 243, 218, 0.28);
  color: #59f3da;
  font-family: var(--mono);
  font-size: clamp(5px, 0.7cqw, 7px);
}

.security__status {
  display: inline-flex;
  align-items: center;
  gap: 0.45cqw;
  color: rgba(148, 170, 163, 0.65);
  font-size: clamp(5px, 0.76cqw, 7.5px);
  white-space: nowrap;
}

.security__status i {
  width: 0.5cqw;
  height: 0.5cqw;
  border-radius: 50%;
  background: #59f3da;
}

.security__body {
  display: grid;
  grid-template-columns: minmax(0, 1.55fr) minmax(0, 1fr);
  flex: 1;
  min-height: 0;
  gap: 1.4cqw;
  padding: 1.5cqw 1.8cqw 1.2cqw;
}

.topology {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1.2cqw;
  min-width: 0;
  padding: 2.2cqw 1.2cqw 3.2cqw;
  overflow: hidden;
  border: 1px solid rgba(207, 255, 243, 0.08);
  border-radius: 0.8cqw;
  background:
    radial-gradient(circle at 50% 50%, rgba(89, 243, 218, 0.09), transparent 55%),
    #08120f;
}

.topology::before {
  content: "";
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(rgba(207, 255, 243, 0.04) 1px, transparent 1px),
    linear-gradient(90deg, rgba(207, 255, 243, 0.04) 1px, transparent 1px);
  background-size: 2.4cqw 2.4cqw;
  opacity: 0.5;
}

.topology__device {
  position: relative;
  z-index: 2;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.3cqw;
  width: 25%;
  padding: 1.4cqw 0.8cqw;
  border: 1px solid rgba(207, 255, 243, 0.12);
  border-radius: 0.6cqw;
  background: rgba(10, 21, 19, 0.85);
  text-align: center;
}

.topology__icon {
  display: grid;
  place-items: center;
  width: 2.4cqw;
  height: 2.4cqw;
  margin-bottom: 0.35cqw;
  color: #59f3da;
}

.topology__icon svg {
  width: 100%;
  height: 100%;
}

.topology__device b {
  color: rgba(233, 243, 238, 0.9);
  font-size: clamp(5.5px, 0.85cqw, 8.5px);
  font-weight: 500;
  white-space: nowrap;
}

.topology__device span {
  color: rgba(89, 243, 218, 0.65);
  font-size: clamp(4.4px, 0.66cqw, 6.5px);
}

.topology__device i {
  position: absolute;
  top: 0.4cqw;
  right: 0.5cqw;
  color: rgba(148, 170, 163, 0.35);
  font-family: var(--mono);
  font-size: clamp(4px, 0.58cqw, 5.5px);
  font-style: normal;
}

.topology__link {
  position: relative;
  z-index: 2;
  flex: 1;
  min-width: 0;
  text-align: center;
}

.topology__wire {
  position: absolute;
  top: 50%;
  right: 0;
  left: 0;
  height: 1px;
  background-image: linear-gradient(90deg, rgba(89, 243, 218, 0.55) 58%, transparent 42%);
  background-size: 0.7cqw 1px;
}

.topology__packet {
  position: absolute;
  top: calc(50% - 0.18cqw);
  width: 0.45cqw;
  height: 0.45cqw;
  border-radius: 50%;
  background: #ff6a3c;
  box-shadow: 0 0 0.7cqw rgba(255, 106, 60, 0.6);
}

.topology__packet.p1 {
  left: 5%;
  animation: security-packet 3.4s var(--ease-in-out) infinite;
}

.topology__packet.p2 {
  left: 5%;
  animation: security-packet 3.4s var(--ease-in-out) 1.7s infinite;
  background: #59f3da;
  box-shadow: 0 0 0.7cqw rgba(89, 243, 218, 0.65);
}

.topology__lock {
  position: absolute;
  top: 50%;
  left: 50%;
  display: grid;
  place-items: center;
  width: 3.1cqw;
  height: 3.1cqw;
  border: 1px solid rgba(89, 243, 218, 0.4);
  border-radius: 50%;
  background: #08120f;
  color: #59f3da;
  transform: translate(-50%, -50%);
  box-shadow: 0 0 1.4cqw rgba(89, 243, 218, 0.16);
}

.topology__lock svg {
  width: 1.4cqw;
  height: 1.4cqw;
}

.topology__link-label {
  position: absolute;
  top: calc(50% + 2.4cqw);
  left: 50%;
  color: rgba(148, 170, 163, 0.55);
  font-family: var(--mono);
  font-size: clamp(4.2px, 0.62cqw, 6px);
  white-space: nowrap;
  transform: translateX(-50%);
}

.topology__cloud {
  position: absolute;
  bottom: 1.2cqw;
  left: 50%;
  z-index: 1;
  display: flex;
  align-items: center;
  gap: 0.5cqw;
  color: rgba(148, 170, 163, 0.35);
  font-size: clamp(4.4px, 0.65cqw, 6.5px);
  transform: translateX(-50%);
}

.topology__cloud-node {
  padding: 0.3cqw 0.6cqw;
  border: 1px dashed rgba(148, 170, 163, 0.28);
  text-decoration: line-through;
}

.topology__cloud-line {
  width: 2cqw;
  height: 1px;
  background: rgba(148, 170, 163, 0.2);
}

.topology__cloud-state {
  color: rgba(255, 106, 60, 0.65);
}

.topology__coordinate {
  position: absolute;
  color: rgba(148, 170, 163, 0.25);
  font-family: var(--mono);
  font-size: clamp(3.5px, 0.5cqw, 5px);
}

.topology__coordinate.x {
  right: 0.8cqw;
  bottom: 0.5cqw;
}

.topology__coordinate.y {
  top: 0.6cqw;
  left: 0.7cqw;
}

.log {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border: 1px solid rgba(207, 255, 243, 0.08);
  border-radius: 0.8cqw;
  background: rgba(3, 10, 8, 0.45);
}

.log__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1cqw 1.1cqw;
  border-bottom: 1px solid rgba(207, 255, 243, 0.08);
  color: rgba(233, 243, 238, 0.72);
  font-size: clamp(5.5px, 0.8cqw, 8px);
}

.log__head em {
  color: rgba(148, 170, 163, 0.45);
  font-family: var(--mono);
  font-style: normal;
  font-size: 0.9em;
}

.log__body {
  display: flex;
  flex: 1;
  flex-direction: column;
  justify-content: center;
  gap: 0.8cqw;
  padding: 1cqw;
}

.log__body p {
  display: flex;
  align-items: center;
  gap: 0.5cqw;
  color: rgba(148, 170, 163, 0.6);
  font-family: var(--mono);
  font-size: clamp(4.2px, 0.62cqw, 6px);
  white-space: nowrap;
}

.log__body i {
  color: rgba(148, 170, 163, 0.25);
  font-style: normal;
}

.log__body span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.log__body b {
  margin-left: auto;
  color: rgba(89, 243, 218, 0.8);
  font-weight: 500;
}

.log__body b.muted {
  color: rgba(255, 106, 60, 0.7);
}

.log__foot {
  display: flex;
  align-items: center;
  gap: 0.5cqw;
  padding: 0.8cqw 1.1cqw;
  border-top: 1px solid rgba(207, 255, 243, 0.08);
  color: rgba(148, 170, 163, 0.4);
  font-size: clamp(4.2px, 0.6cqw, 6px);
}

.log__foot i {
  width: 0.4cqw;
  height: 0.4cqw;
  border-radius: 50%;
  background: rgba(89, 243, 218, 0.5);
}

.security__foot {
  display: flex;
  flex-wrap: wrap;
  gap: 0.6cqw 1.8cqw;
  padding: 0 1.8cqw 1.3cqw;
}

.security__foot span {
  display: inline-flex;
  align-items: center;
  gap: 0.45cqw;
  color: rgba(148, 170, 163, 0.5);
  font-size: clamp(4.2px, 0.64cqw, 6.5px);
}

.security__foot i {
  width: 0.4cqw;
  height: 0.4cqw;
  border: 1px solid rgba(89, 243, 218, 0.6);
  border-radius: 50%;
}

@keyframes security-packet {
  0% {
    left: 5%;
    opacity: 0;
  }
  12% {
    opacity: 1;
  }
  85% {
    opacity: 1;
  }
  100% {
    left: 88%;
    opacity: 0;
  }
}

@media (max-width: 720px) {
  .security__status,
  .topology__cloud,
  .security__foot span:last-child {
    display: none;
  }

  .security__body {
    grid-template-columns: minmax(0, 1fr);
    padding: 1cqw;
  }

  .log {
    display: none;
  }

  .topology {
    padding: 1.2cqw;
  }

  .topology__device {
    width: 32%;
    padding: 0.8cqw 0.3cqw;
  }

  .topology__link-label {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .topology__packet {
    animation: none;
  }
}
</style>

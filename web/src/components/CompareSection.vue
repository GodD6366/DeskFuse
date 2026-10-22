<script setup lang="ts">
const rows: [string, string, string, string][] = [
  ['键鼠跨屏', '仅限同 Apple 账号', '屏幕位置无法自由摆放', '✅'],
  ['剪贴板同步', '依赖 iCloud，稳定性不佳', '仅基础文字、小图片', '✅'],
  ['文件传输', 'AirDrop', '❌', '✅ 支持文件夹'],
  ['文件暂存抽屉', '❌', '❌', '✅'],
  ['剪贴板历史', '❌', '❌', '✅ 跨设备，图片可搜'],
  ['加密传输', '✅', 'TLS', 'QUIC'],
]
</script>

<template>
  <section class="section section-alt" id="compare">
    <div class="section-inner">
      <div class="section-head" v-reveal>
        <p class="eyebrow">对比</p>
        <h2>为什么不是连续互通<br />或 Deskflow？</h2>
      </div>
      <div class="table-wrap" v-reveal>
        <table class="compare-table">
          <thead>
            <tr><th>功能</th><th>macOS 连续互通</th><th>Deskflow</th><th class="hl">DeskFuse</th></tr>
          </thead>
          <tbody>
            <tr v-for="r in rows" :key="r[0]">
              <td>{{ r[0] }}</td>
              <td>{{ r[1] }}</td>
              <td>{{ r[2] }}</td>
              <td class="hl">{{ r[3] }}</td>
            </tr>
          </tbody>
        </table>
        <p class="table-note">对比依据 2026-09：Apple 通用控制 / 通用剪贴板支持文档、Deskflow 官方仓库。</p>
      </div>
    </div>
  </section>
</template>

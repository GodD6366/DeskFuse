<script setup lang="ts">
import { computed, toRef } from 'vue'
import { Accessibility, CheckCircle2, CircleDashed, FolderCog, RefreshCw, ScreenShare, Wifi } from 'lucide-vue-next'
import { usePermissions } from './usePermissions'
import type { Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()
const perms = usePermissions(toRef(props, 'snapshot'))

// 与原生 SettingsPermissionsView 一致：只展示实际使用的四项权限，文案用它写死的说明。
const ROWS = [
  { id: 'accessibility', title: '辅助功能', purpose: '拦截文件粘贴、检查 Finder 焦点与控制键鼠', icon: Accessibility },
  { id: 'full_disk_access', title: '完全磁盘访问', purpose: '访问受系统保护的文件与目录', icon: FolderCog },
  { id: 'screen_recording', title: '屏幕录制', purpose: '远程应用访问本机窗口时捕获屏幕内容', icon: ScreenShare },
  { id: 'wifi', title: 'Wi-Fi 名称（定位）', purpose: '按 Wi-Fi 自动切换主控电脑时读取网络名称；不会获取位置。', icon: Wifi },
]
const rows = computed(() => ROWS.flatMap((row) => {
  const item = perms.find(row.id)
  return item ? [{ ...row, granted: item.granted }] : []
}))
</script>

<template>
  <div class="page">
    <div class="head">
      <h3>系统权限</h3>
      <button type="button" class="refresh" @click="perms.refresh"><RefreshCw :size="12" />刷新</button>
    </div>
    <div class="card list">
      <div v-for="item in rows" :key="item.id" class="row">
        <component :is="item.icon" :size="20" class="icon" />
        <div class="text">
          <div class="title">{{ item.title }}</div>
          <div class="purpose">{{ item.purpose }}</div>
        </div>
        <span class="status" :class="{ ok: item.granted }">
          <CheckCircle2 v-if="item.granted" :size="12" />
          <CircleDashed v-else :size="12" />
          {{ item.granted ? '已授权' : '未授权或需要操作' }}
        </span>
        <button type="button" class="button act" :disabled="perms.busy.value === item.id" @click="perms.act(item.id)">
          {{ item.granted ? '查看设置' : '前往授权' }}
        </button>
      </div>
      <p v-if="!rows.length" class="empty">{{ snapshot ? '正在读取系统权限…' : '后端未连接，暂时无法读取系统权限。' }}</p>
    </div>
    <p v-if="perms.error.value" class="err" role="alert">{{ perms.error.value }}</p>
    <p class="foot">状态来自 macOS。仅展示 DeskFuse 实际使用的系统权限。</p>
  </div>
</template>

<style scoped>
.page { display: grid; align-content: start; gap: 8px; }
.head { display: flex; align-items: center; gap: 12px; }
h3 { margin: 0; padding-left: 4px; font-size: 13px; font-weight: 600; color: var(--secondary); }
.refresh {
  margin-left: auto; display: inline-flex; align-items: center; gap: 4px;
  border: 0; padding: 0; background: transparent; color: var(--accent); font-size: 12px; cursor: pointer;
}
.list { padding: 6px 14px; border: 0; box-shadow: inset 0 0 0 0.5px var(--line); }
.row { display: flex; align-items: center; gap: 16px; padding: 8px 0; }
.row + .row { border-top: 0.5px solid var(--line); }
.icon { flex: none; width: 28px; color: var(--secondary); }
.text { display: grid; flex: 1; gap: 5px; min-width: 0; }
.title { font-size: 13px; font-weight: 500; line-height: 16px; }
.purpose { font-size: 11px; line-height: 14px; color: var(--secondary); }
.status { display: inline-flex; align-items: center; gap: 4px; width: 120px; flex-shrink: 0; font-size: 11px; line-height: 14px; color: var(--secondary); }
.status.ok { color: var(--success); }
.act { width: 88px; flex-shrink: 0; }
.empty { margin: 8px 0; font-size: 13px; color: var(--secondary); }
.foot { margin: 0; padding: 0 4px; font-size: 11px; line-height: 1.5; color: var(--secondary); }
.err { margin: 0; font-size: 11px; color: var(--danger); }
</style>

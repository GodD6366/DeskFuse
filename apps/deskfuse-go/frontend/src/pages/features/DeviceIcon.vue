<script setup lang="ts">
import { computed } from 'vue'

// 对应 DeviceProductIconView：宿主把原生同一份产品图（CoreTypes 照片，
// 找不到时是 SF Symbol）栅格化成 blob，`src` 存在时直接使用；只有拿不到
// 的平台（Windows 服务没有图片数据）才按名称绘制同形轮廓。
const props = withDefaults(defineProps<{ name: string; size?: number; src?: string }>(), { size: 42 })

type ProductKind = 'windows' | 'macmini' | 'macbook' | 'phone'

const kind = computed<ProductKind>(() => {
  const value = props.name
    .toLowerCase()
    .replace(/[-_.]/g, ' ')
  if (/iphone|ipad|手机|phone/.test(value)) return 'phone'
  if (/windows|desktop|pc/.test(value)) return 'windows'
  if (/mac\s*mini|mini/.test(value)) return 'macmini'
  if (/macbook|mac book|mbp|book/.test(value)) return 'macbook'
  return 'macbook'
})
</script>

<template>
  <span class="device-art" :style="{ width: `${size}px`, height: `${size}px` }" aria-hidden="true">
    <img v-if="src" class="product-art" :src="src" alt="" />
    <svg v-else viewBox="0 0 48 48" width="100%" height="100%" focusable="false">
      <template v-if="kind === 'windows'">
        <rect x="5" y="6.5" width="38" height="27" rx="2.4" fill="#3f4650" />
        <rect x="7.5" y="9" width="33" height="20.5" rx="1.2" fill="#e9ebef" />
        <path d="M7.5 9h13.5v9.5H7.5z" fill="#f7f8fa" />
        <path d="M21.5 33.5h5v4.2h-5z" fill="#9aa1aa" />
        <path d="M16.2 38.5h15.6c1 0 1.7.8 1.7 1.7v.3H14.5v-.3c0-.9.7-1.7 1.7-1.7z" fill="#b9bfc7" />
      </template>
      <template v-else-if="kind === 'macmini'">
        <ellipse cx="24" cy="37.6" rx="15.5" ry="2.2" fill="#000" opacity="0.08" />
        <rect x="4.5" y="19.5" width="39" height="16.5" rx="3.4" fill="#c7cbd2" />
        <rect x="4.5" y="19.5" width="39" height="5" rx="2.6" fill="#e4e7eb" />
        <rect x="11.5" y="29.6" width="19" height="1.5" rx="0.75" fill="#9ba1aa" opacity="0.7" />
        <circle cx="38.4" cy="25.6" r="1" fill="#8b919a" />
      </template>
      <template v-else-if="kind === 'phone'">
        <rect x="14" y="4.5" width="20" height="39" rx="3.4" fill="#3f4650" />
        <rect x="16" y="7.5" width="16" height="30" rx="1.2" fill="#9fb8dd" />
        <circle cx="24" cy="40.4" r="1.3" fill="#9aa1aa" />
      </template>
      <template v-else>
        <rect x="9" y="8" width="30" height="21.5" rx="2.2" fill="#bfc4cc" />
        <rect x="10.7" y="9.8" width="26.6" height="17.9" rx="0.9" fill="#8fb4e6" />
        <path d="M10.7 9.8h26.6v5.4H10.7z" fill="#aacbf4" opacity="0.85" />
        <path d="M6.2 31.2h35.6l1.7 3.4c.3.7-.1 1.4-.9 1.4H5.4c-.8 0-1.2-.7-.9-1.4z" fill="#d6d9df" />
        <path d="M20.4 33.2h7.2l-.7 1.3h-5.8z" fill="#b6bcc4" />
      </template>
    </svg>
  </span>
</template>

<style scoped>
.device-art {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
}
.product-art {
  width: 100%;
  height: 100%;
  object-fit: contain;
}
</style>

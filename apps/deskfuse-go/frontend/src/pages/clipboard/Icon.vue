<script setup lang="ts">
// 剪贴板页用到的 SF Symbols 的描边近似，统一 24 视口。
const props = withDefaults(defineProps<{ name: string; size?: number; strokeWidth?: number }>(), { size: 14, strokeWidth: 1.8 })

const paths: Record<string, string> = {
  magnifyingglass: 'M10.5 18a7.5 7.5 0 1 1 0-15 7.5 7.5 0 0 1 0 15ZM16 16l5 5',
  ellipsis: 'M5 12h.01M12 12h.01M19 12h.01',
  laptop: 'M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5V6.5ZM3 18h18',
  list: 'M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01',
  grid: 'M4 4h6.5v6.5H4V4ZM13.5 4H20v6.5h-6.5V4ZM4 13.5h6.5V20H4v-6.5ZM13.5 13.5H20V20h-6.5v-6.5Z',
  copy: 'M9 9h10v11H9V9ZM15 9V5H5v11h4',
  trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  check: 'M5 12.5l4.5 4.5L19 7.5',
  checkCircle: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM8 12.5l2.7 2.7 5.3-5.4',
  xmarkCircle: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM9 9l6 6M15 9l-6 6',
  viewfinder: 'M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M8 9h8M12 9v7',
  download: 'M12 4v10m0 0-4-4m4 4 4-4M5 19h14',
  downloadCircle: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM12 8v6m0 0-3-3m3 3 3-3',
  photo: 'M4 5h16v14H4V5ZM4 16l5-5 4 4 3-3 4 4M15.5 9.5h.01',
  doc: 'M7 3h7l5 5v13H7V3ZM14 3v5h5',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  code: 'M9 8l-4 4 4 4M15 8l4 4-4 4M13.5 6l-3 12',
  clipboard: 'M9 4h6v3H9V4ZM7 5.5H5.5V20h13V5.5H17',
  warning: 'M12 4 2.8 20h18.4L12 4ZM12 10v4.5M12 17.5h.01',
  stack: 'M5 6h14M7 3h10M4 9h16v11H4V9Z',
  folder: 'M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18V6.5Z',
  text: 'M4 6h16M4 10h10M4 14h16M4 18h10',
  smart: 'M4 7h16M7 12h10M10 17h4',
  plusCircle: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM12 8v8M8 12h8',
  minusCircle: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM8 12h8',
  tray: 'M4 14l2-8h12l2 8M4 14v5h16v-5M4 14h4.5a3.5 3.5 0 0 0 7 0H20',
  info: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM12 11v5.5M12 7.5h.01',
  chevronRight: 'M9 5l7 7-7 7',
  chevronDown: 'M5 9l7 7 7-7',
  xmark: 'M6 6l12 12M18 6 6 18',
  docMissing: 'M7 3h7l5 5v13H7V3ZM14 3v5h5M12 11v5M12 14l-2.5 2.5M12 14l2.5 2.5',
  photoMissing: 'M4 5h16v14H4V5ZM4 16l5-5 4 4M16 9v4M16 15.5h.01',
  pencil: 'M4 20l1-4L16.5 4.5a2 2 0 0 1 3 3L8 19l-4 1Z',
  eye: 'M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12ZM12 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5Z',
  paste: 'M10 4h4v3h-4V4ZM8 5.5H6V20h12V5.5h-2M10 13h4',
  paperplane: 'M21 3 3 10.5l7 2.5 2.5 7L21 3ZM10 13l11-10',
  expand: 'M9 4H4v5M15 4h5v5M20 15v5h-5M4 15v5h5M9 9 4 4M15 9l5-5M15 15l5 5M9 15l-5 5',
}
</script>

<template>
  <svg
    v-if="props.name === 'xmarkFill'"
    :width="size" :height="size" viewBox="0 0 24 24" aria-hidden="true" class="df-icon"
  >
    <circle cx="12" cy="12" r="10" fill="currentColor" />
    <path d="M8.5 8.5l7 7M15.5 8.5l-7 7" stroke="var(--icon-knockout, #fff)" stroke-width="2" stroke-linecap="round" fill="none" />
  </svg>
  <svg
    v-else
    :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor"
    :stroke-width="props.strokeWidth" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" class="df-icon"
  >
    <path :d="paths[props.name] ?? ''" />
  </svg>
</template>

<style scoped>
.df-icon { flex: none; display: block; }
</style>

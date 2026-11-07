<script setup lang="ts">
defineProps<{ name: string; size?: number }>()
</script>

<template>
  <svg
    :width="size ?? 24"
    :height="size ?? 24"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="name === 'keyboard'">
      <rect x="2.5" y="6" width="19" height="12" rx="2" />
      <path
        d="M6 10h.01M9 10h.01M12 10h.01M15 10h.01M18 10h.01M6 13h.01M9 13h.01M12 13h.01M15 13h.01M18 13h.01M8 16h8"
      />
    </template>

    <template v-else-if="name === 'layout'">
      <rect x="3" y="4" width="8" height="7" rx="1.5" />
      <rect x="13" y="4" width="8" height="7" rx="1.5" />
      <rect x="6" y="14" width="12" height="6" rx="1.5" />
    </template>

    <template v-else-if="name === 'clipboard'">
      <rect x="5" y="4" width="14" height="17" rx="2" />
      <path d="M9 4V3.2A1.2 1.2 0 0 1 10.2 2h3.6A1.2 1.2 0 0 1 15 3.2V4M8 9h8M8 13h8M8 17h5" />
    </template>

    <template v-else-if="name === 'history'">
      <path d="M3 12a9 9 0 1 0 2.6-6.4L3 8" />
      <path d="M3 3v5h5M12 7v5l3 2" />
    </template>

    <template v-else-if="name === 'folder'">
      <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h5l2 2h8A1.5 1.5 0 0 1 21 9.5v8A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z" />
    </template>

    <template v-else-if="name === 'phone'">
      <rect x="7" y="2.5" width="10" height="19" rx="2.5" />
      <path d="M11 5h2M12 18.5h.01" />
    </template>

    <template v-else-if="name === 'window'">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 8h18M6 6h.01M9 6h.01" />
    </template>

    <template v-else-if="name === 'windows'">
      <path d="M3 5.5 10.5 4v7H3zM13.5 3.5 21 2v9h-7.5zM3 13h7.5v7L3 18.5zM13.5 13H21v9l-7.5-1.5z" />
    </template>

    <template v-else-if="name === 'mouse'">
      <rect x="6" y="2" width="12" height="20" rx="6" />
      <path d="M12 2v7M6 10h6M18 12h3M18 16h3" />
    </template>

    <template v-else-if="name === 'mic'">
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M9 21h6" />
    </template>

    <template v-else-if="name === 'shield'">
      <path d="M12 3 20 6v5.5c0 4.8-3.4 8-8 9.5-4.6-1.5-8-4.7-8-9.5V6z" />
      <path d="m8.5 12 2.5 2.5 4.5-5" />
    </template>

    <template v-else-if="name === 'lock'">
      <rect x="4" y="10" width="16" height="11" rx="2" />
      <path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3" />
    </template>

    <template v-else-if="name === 'pulse'">
      <path d="M3 12h4l2.2-6 4.2 12L15 12h6" />
    </template>

    <template v-else-if="name === 'download'">
      <path d="M12 3v12M7.5 10.5 12 15l4.5-4.5M4 20h16" />
    </template>

    <template v-else-if="name === 'copy'">
      <rect x="8" y="8" width="12" height="12" rx="2" />
      <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
    </template>

    <template v-else-if="name === 'arrow'">
      <path d="M4 12h16M14 6l6 6-6 6" />
    </template>

    <template v-else-if="name === 'check'">
      <path d="m4 12 5 5L20 6" />
    </template>

    <template v-else-if="name === 'plus'">
      <path d="M12 5v14M5 12h14" />
    </template>

    <template v-else-if="name === 'pause'">
      <path d="M9 5v14M15 5v14" stroke-width="2" />
    </template>

    <template v-else-if="name === 'play'">
      <path d="m8 5 11 7-11 7z" />
    </template>

    <template v-else-if="name === 'github'">
      <path
        fill="currentColor"
        stroke="none"
        d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02a9.5 9.5 0 0 1 5 0c1.91-1.29 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85V21c0 .27.18.58.69.48A10 10 0 0 0 12 2z"
      />
    </template>

    <template v-else>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l2.5 1.5" />
    </template>
  </svg>
</template>

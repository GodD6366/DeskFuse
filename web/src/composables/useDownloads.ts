import { computed } from 'vue'
import {
  ARM64_DISK_IMAGE,
  FALLBACK_RELEASE_TAG,
  INTEL_DISK_IMAGE,
  REPOSITORY_URL,
  macAppDownload,
} from '../config/distribution'
import { useLatestRelease } from './useLatestRelease'

function formatSize(bytes?: number): string {
  if (!bytes) return ''
  return `${Math.round(bytes / 1024 / 1024)} MB`
}

/** 页面各处的下载按钮共用一份版本与地址；读不到发布列表时回退到已知版本。 */
export function useDownloads() {
  const { release } = useLatestRelease()

  const tag = computed(() => release.value?.tag || FALLBACK_RELEASE_TAG)
  const prerelease = computed(() => release.value?.prerelease ?? true)
  const releaseUrl = computed(
    () => release.value?.url || `${REPOSITORY_URL}/releases/tag/${tag.value}`,
  )
  const arm64 = computed(() =>
    macAppDownload(release.value?.assets, tag.value, ARM64_DISK_IMAGE, 'arm64'),
  )
  const intel = computed(() =>
    macAppDownload(release.value?.assets, tag.value, INTEL_DISK_IMAGE, 'x86_64'),
  )
  const arm64Size = computed(() => formatSize(arm64.value.size))
  const intelSize = computed(() => formatSize(intel.value.size))
  const date = computed(() => {
    const raw = release.value?.publishedAt
    if (!raw) return ''
    const value = new Date(raw)
    if (Number.isNaN(value.getTime())) return ''
    const pad = (n: number) => String(n).padStart(2, '0')
    return `${value.getFullYear()}-${pad(value.getMonth() + 1)}-${pad(value.getDate())}`
  })

  return { tag, prerelease, releaseUrl, arm64, intel, arm64Size, intelSize, date }
}

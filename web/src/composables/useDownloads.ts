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
  const arm64 = computed(() => macAppDownload(release.value?.assets, ARM64_DISK_IMAGE))
  const intel = computed(() => macAppDownload(release.value?.assets, INTEL_DISK_IMAGE))
  const arm64Size = computed(() => formatSize(arm64.value.size))

  return { tag, prerelease, releaseUrl, arm64, intel, arm64Size }
}

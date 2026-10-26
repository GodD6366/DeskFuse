import config from './distribution.json'

/** 源码与发行版所在的公开仓库。 */
export const RELEASE_REPOSITORY = config.release_repository
export const REPOSITORY_URL = `https://github.com/${config.release_repository}`
/** 过渡期仍需合并的旧发行仓库；空字符串表示过渡已结束。 */
export const LEGACY_RELEASE_REPOSITORY = config.legacy_release_repository
export const HOMEBREW_TAP = config.homebrew_tap
export const CASK_TOKEN = config.cask_token
export const FALLBACK_RELEASE_TAG = config.fallback_release_tag

/** 拿不到最新版本信息时的静态下载地址：指向确实存有回退版本的仓库。 */
export function fallbackAssetUrl(name: string): string {
  return `https://github.com/${config.fallback_release_repository}/releases/download/${FALLBACK_RELEASE_TAG}/${name}`
}

export interface DownloadAsset {
  name: string
  url: string
  size?: number
}

export interface MacDownload {
  url: string
  /** 应用包（zip）或磁盘映像（dmg），决定安装第一步怎么写。 */
  kind: 'zip' | 'dmg'
  size?: number
}

export const ARM64_DISK_IMAGE = config.assets.arm64
export const INTEL_DISK_IMAGE = config.assets.x86_64

/** 与打包脚本一致：`DeskFuse-arm64.dmg` → `DeskFuse-arm64-update.zip`。 */
export function appPackageName(diskImage: string): string {
  return diskImage.endsWith('.dmg') ? `${diskImage.slice(0, -4)}-update.zip` : diskImage
}

/**
 * 官网按钮优先指向应用包。当前已发布的版本还没有这个 zip 时，
 * 用同一发布里的磁盘映像，避免按钮在过渡期指向不存在的文件。
 * 发布列表也读不到时，回退到已知存在的磁盘映像。
 */
export function macAppDownload(
  assets: readonly DownloadAsset[] | null | undefined,
  diskImage: string,
): MacDownload {
  const appZip = appPackageName(diskImage)
  const zip = assets?.find((asset) => asset.name === appZip && asset.url)
  if (zip) return { url: zip.url, kind: 'zip', size: zip.size }
  const image = assets?.find((asset) => asset.name === diskImage && asset.url)
  if (image) return { url: image.url, kind: 'dmg', size: image.size }
  return { url: fallbackAssetUrl(diskImage), kind: 'dmg' }
}

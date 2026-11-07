import config from './distribution.json'

/** 源码与发行版所在的公开仓库。 */
export const RELEASE_REPOSITORY = config.release_repository
export const REPOSITORY_URL = `https://github.com/${config.release_repository}`
/** 过渡期仍需合并的旧发行仓库；空字符串表示过渡已结束。 */
export const LEGACY_RELEASE_REPOSITORY = config.legacy_release_repository
export const HOMEBREW_TAP = config.homebrew_tap
export const CASK_TOKEN = config.cask_token
export const FALLBACK_RELEASE_TAG = config.fallback_release_tag
export const FALLBACK_APP_PACKAGE = config.fallback_app_package

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
export const WINDOWS_X64_INSTALLER = config.assets.windows_x64

/** 与打包脚本一致：`DeskFuse-<version>-<arch>.zip`。 */
export function appPackageName(tag: string, arch: 'arm64' | 'x86_64'): string {
  return `DeskFuse-${tag.replace(/^v/, '')}-${arch}.zip`
}

/** v0.8.0 及更早的 zip 文件名，仍需可读。 */
export function legacyAppPackageName(arch: 'arm64' | 'x86_64'): string {
  return `DeskFuse-${arch}-update.zip`
}

/**
 * 官网按钮优先指向版本化应用包。v0.8.0 仍使用旧 zip 文件名；
 * 只有发布里确实没有 zip 时才退回磁盘映像。
 */
export function macAppDownload(
  assets: readonly DownloadAsset[] | null | undefined,
  tag: string,
  diskImage: string,
  arch: 'arm64' | 'x86_64',
): MacDownload {
  const appZip = appPackageName(tag, arch)
  const legacyZip = legacyAppPackageName(arch)
  const zip = assets?.find((asset) => asset.name === appZip && asset.url)
  if (zip) return { url: zip.url, kind: 'zip', size: zip.size }
  const legacy = assets?.find((asset) => asset.name === legacyZip && asset.url)
  if (legacy) return { url: legacy.url, kind: 'zip', size: legacy.size }
  const image = assets?.find((asset) => asset.name === diskImage && asset.url)
  if (image) return { url: image.url, kind: 'dmg', size: image.size }
  // The static fallback must not send new installs back to the DMG. The
  // release list is empty on first paint and when GitHub/feed requests fail;
  // current releases always publish the app zip alongside the disk image.
  if (FALLBACK_APP_PACKAGE) {
    return { url: fallbackAssetUrl(appZip), kind: 'zip' }
  }
  return { url: fallbackAssetUrl(legacyZip), kind: 'zip' }
}

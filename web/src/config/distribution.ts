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

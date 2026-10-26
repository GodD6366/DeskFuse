import { onMounted, ref, type Ref } from 'vue'
import { RELEASE_REPOSITORY } from '../config/distribution'
export { FALLBACK_RELEASE_TAG } from '../config/distribution'

export interface ReleaseAsset {
  name: string
  url: string
  size?: number
}

export interface LatestRelease {
  tag: string
  prerelease: boolean
  url: string
  assets: ReleaseAsset[]
}

// 与 GitHub Releases API 同形的站点清单，部署时已合并过渡期的旧发行仓库；
// 同源读取更快也不受 API 限流影响，读不到时再直接查 GitHub。
const FEED = new URL('api/releases.json', document.baseURI).toString()
const API = `https://api.github.com/repos/${RELEASE_REPOSITORY}/releases?per_page=5`

let cached: Promise<LatestRelease | null> | null = null

async function fetchReleases(url: string): Promise<any[] | null> {
  try {
    const response = await fetch(url, { headers: { Accept: 'application/vnd.github+json' } })
    const payload = response.ok ? await response.json() : null
    return Array.isArray(payload) && payload.length > 0 ? payload : null
  } catch {
    return null
  }
}

function semver(tag: unknown): [number, number, number] | null {
  const match = typeof tag === 'string' ? /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag) : null
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null
}

function compareSemver(a: [number, number, number], b: [number, number, number]): number {
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2]
}

function fetchLatest(): Promise<LatestRelease | null> {
  return (async () => {
    const releases = (await fetchReleases(FEED)) ?? (await fetchReleases(API))
    if (!releases) return null
    // 全部为 prerelease 时 /releases/latest 会 404；合并后的列表不保证按时间排序，按版本号取最高的非草稿
    const latest = releases
      .filter((r) => !r?.draft && semver(r?.tag_name))
      .sort((a, b) => compareSemver(semver(b.tag_name)!, semver(a.tag_name)!))[0]
    if (!latest) return null
    return {
      tag: latest.tag_name || '',
      prerelease: Boolean(latest.prerelease),
      url: latest.html_url || '',
      assets: (latest.assets || []).map((a: any) => ({
        name: a.name,
        url: a.browser_download_url,
        size: typeof a.size === 'number' ? a.size : undefined,
      })),
    }
  })()
}

/** 共享最新 release：页面内多处使用也只请求一次，失败时保持静态回退 */
export function useLatestRelease() {
  const release: Ref<LatestRelease | null> = ref(null)
  onMounted(async () => {
    cached ??= fetchLatest()
    release.value = await cached
  })
  return { release }
}

/** 取某个资产的下载地址；拿不到时回退到给定链接 */
export function assetUrl(release: LatestRelease | null, name: string, fallback: string): string {
  const hit = release?.assets.find((a) => a.name === name)
  return hit ? hit.url : fallback
}

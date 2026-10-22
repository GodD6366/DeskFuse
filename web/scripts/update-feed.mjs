/**
 * Refresh the release endpoints the DeskFuse updater reads.
 *
 * Update checks used to depend entirely on GitHub: the release list came from
 * api.github.com and the small per-release metadata files came from the release
 * asset CDN. Both hosts are unreachable, rate limited, or intermittently 504
 * from some networks, so the site republishes them on its own domain:
 *
 *   https://deskfuse.godd.cc/api/releases.json
 *   https://deskfuse.godd.cc/api/releases/<tag>/SHA256SUMS
 *   https://deskfuse.godd.cc/api/releases/<tag>/release.json
 *   https://deskfuse.godd.cc/api/releases/<tag>/build-info.json
 *
 * The list keeps the GitHub Releases API shape so a client can swap the URL
 * without touching its parser, and the metadata files stay byte for byte
 * identical so the client's SHA-256 checks and code-signing checks still hold.
 *
 * Vite copies the committed baselines from web/public first; this script
 * overwrites a file only after a successful fetch, so a rate limit or a GitHub
 * outage can never take an endpoint offline, empty, or corrupt.
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const feedOutput = resolve(scriptDirectory, '../../site/api/releases.json')
const mirrorOutput = resolve(scriptDirectory, '../../site/api/releases')
// 仓库来自 scripts/distribution-config.py 生成的站点配置，与客户端编译进去的地址同源。
const distribution = JSON.parse(
  await readFile(resolve(scriptDirectory, '../src/config/distribution.json'), 'utf8'),
)
const repository = distribution.release_repository
// 过渡期：旧客户端仍从旧发行仓库升级，站点清单合并两边，同一 tag 以新仓库为准。
const legacyRepository = distribution.legacy_release_repository || null
const releaseLimit = Number.parseInt(process.env.DESKFUSE_FEED_LIMIT ?? '30', 10)
// Overridable so the feed can be pointed at a mirror, and so the script can be
// exercised against a stub without touching the public API.
const apiBase = (process.env.DESKFUSE_FEED_API ?? 'https://api.github.com').replace(/\/+$/, '')
const sumsName = 'SHA256SUMS'
/** 更新检查会在 GitHub 资产 CDN 抖动时改读站点镜像，这三个文件都要有 */
const metadataNames = [sumsName, 'release.json', 'build-info.json']
const metadataLimit = 256 * 1024
const tagPattern = /^v[0-9A-Za-z][0-9A-Za-z.\-]*$/

function headers(extra = {}) {
  const values = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'deskfuse-site',
    'X-GitHub-Api-Version': '2022-11-28',
    ...extra,
  }
  if (process.env.GITHUB_TOKEN) {
    values.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  }
  return values
}

async function fetchText(url) {
  const response = await fetch(url, { headers: headers() })
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`)
  }
  return response.text()
}

async function fetchBytes(url) {
  const response = await fetch(url, {
    headers: headers({ Accept: 'application/octet-stream' }),
    redirect: 'follow',
  })
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`)
  }
  const bytes = Buffer.from(await response.arrayBuffer())
  if (bytes.length === 0 || bytes.length > metadataLimit) {
    throw new Error(`unexpected size ${bytes.length} for ${url}`)
  }
  return bytes
}

async function isReadable(path) {
  try {
    await readFile(path)
    return true
  } catch {
    return false
  }
}

/** `sha256  file` lines, as written by scripts/release.py. */
function parseSums(text) {
  const sums = new Map()
  for (const line of text.split('\n')) {
    const match = /^([a-f0-9]{64})\s+\*?(.+?)\s*$/.exec(line)
    if (match) {
      sums.set(match[2], match[1])
    }
  }
  return sums
}

function normalize(release, sums) {
  return {
    tag_name: release.tag_name,
    name: release.name ?? '',
    draft: Boolean(release.draft),
    prerelease: Boolean(release.prerelease),
    published_at: release.published_at ?? null,
    created_at: release.created_at ?? null,
    html_url: release.html_url ?? '',
    assets: (release.assets ?? []).map((asset) => {
      const normalized = {
        name: asset.name,
        size: asset.size ?? 0,
        state: asset.state ?? '',
        browser_download_url: asset.browser_download_url ?? '',
      }
      // GitHub's own payload carries no checksum; the newest release gets one
      // so a client can verify a DMG without a second request.
      if (sums?.has(asset.name)) {
        normalized.sha256 = sums.get(asset.name)
      }
      return normalized
    }),
  }
}

/** The newest release that is public and actually installable. */
function newestInstallable(releases) {
  return releases.find(
    (release) =>
      !release.draft &&
      release.published_at &&
      release.assets?.some((asset) => asset.name === sumsName && asset.state === 'uploaded'),
  )
}

async function fetchReleaseList(name) {
  const releases = JSON.parse(await fetchText(`${apiBase}/repos/${name}/releases?per_page=100`))
  if (!Array.isArray(releases)) {
    throw new Error(`release list of ${name} is malformed`)
  }
  return releases
}

function semver(tag) {
  const match = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag ?? '')
  return match ? match.slice(1).map(Number) : null
}

/** 新版本在前；无法解析版本号的 tag 排在最后，保持原有相对顺序。 */
function byVersionDescending(left, right) {
  const a = semver(left.tag_name)
  const b = semver(right.tag_name)
  if (!a || !b) return (a ? 0 : 1) - (b ? 0 : 1)
  return b[0] - a[0] || b[1] - a[1] || b[2] - a[2]
}

async function mergedReleases() {
  const primary = await fetchReleaseList(repository)
  if (!legacyRepository) {
    return primary
  }
  let legacy = []
  try {
    legacy = await fetchReleaseList(legacyRepository)
  } catch (error) {
    console.warn(`warning: skipping legacy releases of ${legacyRepository}: ${error.message}`)
  }
  const tags = new Set(primary.map((release) => release?.tag_name))
  return [...primary, ...legacy.filter((release) => !tags.has(release?.tag_name))].sort(byVersionDescending)
}

async function buildFeed() {
  const releases = await mergedReleases()
  // An authenticated call also returns drafts; they must never reach the feed.
  const published = releases.filter((release) => !release?.draft)
  if (published.length === 0) {
    throw new Error('release list is empty')
  }
  const page = published.slice(0, releaseLimit)
  const newest = newestInstallable(page)
  let sums = null
  if (newest) {
    const sumsUrl = newest.assets.find((asset) => asset.name === sumsName).browser_download_url
    try {
      sums = parseSums(await fetchText(sumsUrl))
    } catch (error) {
      console.warn(`warning: skipping checksums for ${newest.tag_name}: ${error.message}`)
    }
  }
  return {
    feed: page.map((release) =>
      normalize(release, sums && release.tag_name === newest?.tag_name ? sums : null),
    ),
    newest: newest?.tag_name ?? 'n/a',
  }
}

/**
 * Mirror the metadata an update check reads, byte for byte. A single failure
 * only warns: the committed snapshot under web/public is copied into site/ by
 * vite and keeps serving, so the endpoint never 404s or goes stale silently.
 */
async function mirrorMetadata(releases) {
  let written = 0
  for (const release of releases) {
    const tag = release.tag_name ?? ''
    if (!tagPattern.test(tag)) {
      console.warn(`warning: skipping metadata for unusable tag ${JSON.stringify(tag)}`)
      continue
    }
    for (const name of metadataNames) {
      const asset = (release.assets ?? []).find(
        (entry) => entry.name === name && entry.state === 'uploaded' && entry.size > 0,
      )
      if (!asset?.browser_download_url) {
        continue
      }
      const target = join(mirrorOutput, tag, name)
      try {
        const bytes = await fetchBytes(asset.browser_download_url)
        await mkdir(dirname(target), { recursive: true })
        await writeFile(target, bytes)
        written += 1
      } catch (error) {
        const kept = await isReadable(target)
        console.warn(
          `warning: ${kept ? 'keeping the committed' : 'no committed'} ${tag}/${name}: ${error.message}`,
        )
      }
    }
  }
  return written
}

async function main() {
  let feed
  try {
    feed = await buildFeed()
  } catch (error) {
    let existing = ''
    try {
      existing = await readFile(feedOutput, 'utf8')
    } catch {
      throw new Error(`release feed is unavailable and no baseline is published: ${error.message}`)
    }
    const published = JSON.parse(existing)
    console.warn(
      `warning: keeping the committed release feed (${published.length} releases): ${error.message}`,
    )
    return
  }
  await writeFile(feedOutput, `${JSON.stringify(feed.feed, null, 2)}\n`)
  const mirrored = await mirrorMetadata(feed.feed)
  console.log(
    `release feed: ${feed.feed.length} releases, newest ${feed.newest}, ${mirrored} mirrored metadata files -> ${feedOutput}`,
  )
}

await main()

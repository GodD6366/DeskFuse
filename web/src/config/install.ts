import { CASK_TOKEN, HOMEBREW_TAP, REPOSITORY_URL } from './distribution'

/**
 * Homebrew 安装命令的唯一来源，Hero 命令行与安装卡片共用。
 * tap 直接指向源码仓库根目录的 `Casks/`。
 */
const TAP = HOMEBREW_TAP
const REPO = REPOSITORY_URL

export const HOMEBREW_ONE_LINE = `brew trust --cask ${TAP}/${CASK_TOKEN} && brew tap ${TAP} ${REPO} && brew install --cask ${TAP}/${CASK_TOKEN}`

export const HOMEBREW_SCRIPT = [
  `brew trust --cask ${TAP}/${CASK_TOKEN}`,
  `brew tap ${TAP} \\
  ${REPO}`,
  `brew install --cask ${TAP}/${CASK_TOKEN}`,
].join('\n')

export const HOMEBREW_UPGRADE = `brew update && brew upgrade --cask ${CASK_TOKEN}`

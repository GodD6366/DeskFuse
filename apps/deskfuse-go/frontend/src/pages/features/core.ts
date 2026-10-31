import './features.css'
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { backend } from '../../services/backend'

export type Snapshot = Record<string, any>

export interface ErrorInfo {
  /** 面向用户的中文说明。 */
  message: string
  /** 原始错误，供“复制详情”和“反馈问题”使用。 */
  raw: string
}

/** 与 HealthCenter.friendlyError 一致：英文内部错误换成通用说明，中文原文保留。 */
export function friendlyError(message: string): string {
  const lower = message.toLowerCase()
  const rules: Array<[string[], string]> = [
    [['timed out', 'timeout', '超时'], '连接超时，请确认两台电脑在同一网络后重试。'],
    [['connection refused', 'unreachable', 'no route', 'network is down'], '无法连接到对方电脑，请确认对方已打开 DeskFuse。'],
    [['permission', 'not permitted', 'accessibility', 'denied'], '缺少系统权限，请在「设置 → 权限」中授权后重试。'],
    [['disk full', 'no space'], '磁盘空间不足，请清理后重试。'],
    [['stream stalled', 'transport operation failed'], '网络传输中断，DeskFuse 会自动重试。'],
    [['store operation failed', 'database', 'sqlite'], '本地数据读写失败，请重新打开 DeskFuse；反复出现时，请通过「帮助 → 反馈问题」告诉我们。'],
    [['command channel is closed', 'runtime'], '后台服务未就绪，请稍后重试或重新打开 DeskFuse。'],
  ]
  const hit = rules.find(([keys]) => keys.some((key) => lower.includes(key)))
  if (hit) return hit[1]
  return /[一-鿿]/.test(message)
    ? message
    : '操作未完成，请稍后重试。反复出现时，请通过「帮助 → 反馈问题」告诉我们。'
}

const camelKey = (key: string) => key.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase())

/** 后端快照可能是 snake_case（Rust）或 camelCase；页面统一按 camelCase 读取。 */
export function camelize<T = any>(value: any): T {
  if (Array.isArray(value)) return value.map((item) => camelize(item)) as T
  if (value && typeof value === 'object') {
    const out: Record<string, any> = {}
    for (const [key, item] of Object.entries(value)) out[camelKey(key)] = camelize(item)
    return out as T
  }
  return value as T
}

/** 枚举既可能是 "Completed"，也可能是 {Completed:{}}；统一成小写名称。 */
export function enumName(value: any): string {
  if (typeof value === 'string') return value.toLowerCase().replace(/[-_ ]/g, '')
  if (value && typeof value === 'object') {
    const [key] = Object.keys(value)
    if (key) return key.toLowerCase().replace(/[-_ ]/g, '')
  }
  return ''
}

export function byteCount(bytes: number | null | undefined): string {
  const value = Math.max(0, Number(bytes) || 0)
  if (value < 1000) return `${value} 字节`
  const units = ['KB', 'MB', 'GB', 'TB']
  let size = value / 1000
  let unit = 0
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000
    unit += 1
  }
  return `${size >= 100 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`
}

/** 与 Transfer.phaseTitle 一致，传输页与设备页共用。 */
export function transferPhaseTitle(task: { phase: unknown; outgoing?: boolean }): string {
  switch (enumName(task.phase)) {
    case 'queued': return '准备中'
    case 'sending': return '发送中'
    case 'receiving': return '接收中'
    case 'verifying': return task.outgoing ? '等待校验' : '校验中'
    case 'delivering': return '保存中'
    case 'completed': return '已完成'
    case 'cancelled': return '已取消'
    case 'failed': return '失败'
    default: return ''
  }
}

export function milliseconds(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—'
  return value > 0 && value < 0.1 ? `${value.toFixed(2)} ms` : `${value.toFixed(1)} ms`
}

export type ActionResult<T = any> = { ok: true; value: T } | { ok: false; error: ErrorInfo }

/**
 * 所有原生动作都走这里。没有后端、动作未接通或后端报错时都显式给出错误，
 * 不返回伪造的成功结果。
 */
export async function invokeAction<T = any>(action: string, params?: Record<string, unknown>): Promise<ActionResult<T>> {
  try {
    if (!backend || typeof backend.invoke !== 'function') {
      throw new Error(`没有可用的后端，无法执行 ${action}`)
    }
    const value = await backend.invoke(action, params)
    return { ok: true, value: value as T }
  } catch (cause) {
    const raw = cause instanceof Error ? cause.message : String(cause)
    return { ok: false, error: { message: friendlyError(raw), raw: `${action}: ${raw}` } }
  }
}

/** 页面级动作状态：忙碌标记 + 可关闭、可重试的错误。 */
export function useAction() {
  const error = ref<ErrorInfo | null>(null)
  const busy = reactive<Record<string, boolean>>({})
  let lastCall: (() => Promise<unknown>) | null = null

  async function run<T = any>(
    action: string,
    params?: Record<string, unknown>,
    options: { key?: string; keepError?: boolean } = {},
  ): Promise<ActionResult<T>> {
    const key = options.key ?? action
    const call = () => run<T>(action, params, options)
    busy[key] = true
    if (!options.keepError) error.value = null
    const result = await invokeAction<T>(action, params)
    busy[key] = false
    if (!result.ok) {
      error.value = result.error
      lastCall = call
    }
    return result
  }

  function retry() {
    const call = lastCall
    error.value = null
    if (call) void call()
  }

  return { error, busy, run, retry, dismiss: () => { error.value = null } }
}

/** 串行轮询：上一次未返回不会再发下一次，卸载后停止。 */
export function usePolling(tick: () => Promise<unknown>, intervalMs: number) {
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  const loop = async () => {
    try {
      await tick()
    } finally {
      if (!stopped) timer = setTimeout(loop, intervalMs)
    }
  }
  onMounted(() => {
    stopped = false
    void loop()
  })
  onBeforeUnmount(() => {
    stopped = true
    if (timer) clearTimeout(timer)
  })
  return { refresh: tick }
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}

export function reportProblem(raw: string) {
  return invokeAction('support.reportProblem', { message: raw })
}

export function localStore<T>(key: string, fallback: T): { read: () => T; write: (value: T) => void } {
  return {
    read() {
      try {
        const text = localStorage.getItem(key)
        return text === null ? fallback : (JSON.parse(text) as T)
      } catch {
        return fallback
      }
    },
    write(value) {
      try {
        localStorage.setItem(key, JSON.stringify(value))
      } catch {
        /* 存储不可用时只丢失偏好，不影响功能 */
      }
    },
  }
}

export type PageKey = 'devices' | 'clipboard' | 'input' | 'files' | 'remoteApplications' | 'microphone' | 'mouse' | 'settings'

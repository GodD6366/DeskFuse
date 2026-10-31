import { Backend, Preview } from '../mygo'

function available() {
  if (!('mygo' in window)) throw new Error('原生服务未连接，请在 DeskFuse 桌面窗口中打开。')
}
export const backend = {
  async snapshot(): Promise<any> { available(); return Backend.snapshot() },
  async hello(): Promise<any> { available(); return Backend.hello() },
  async queryLibrary(params: Record<string, unknown>): Promise<any> { available(); return Backend.queryLibrary(params) },
  async mutateLibrary(params: Record<string, unknown>): Promise<any> { available(); return Backend.mutateLibrary(params) },
  async invoke(action: string, params: Record<string, unknown> = {}): Promise<any> {
    available()
    const result = await Backend.invoke(action, params)
    return Object.prototype.hasOwnProperty.call(result, 'value') ? result.value : result
  },
  async content(id: string): Promise<any> { available(); return Backend.content(id) },
  /** 鼠标适配数据的 Markdown 与文件名由 Go 中间层生成，页面只负责表单。 */
  async mouseContribution(params: Record<string, unknown>): Promise<any> { available(); return Backend.mouseContribution(params) },
  async dismissTransfer(id: string): Promise<void> { available(); await Backend.dismissTransfer(id) },
  /** 面板行的只读预览浮窗由 Go 进程持有；页面只给条目与行位置。 */
  async previewShow(params: Record<string, unknown>): Promise<void> { available(); await Preview.show(params) },
  async previewHide(): Promise<void> { available(); await Preview.hide() },
  thumbnailURL(id: string, size = 240): string { return `deskfuse://blob/thumbnail/${encodeURIComponent(id)}?size=${size}` },
}
export function errorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error) }

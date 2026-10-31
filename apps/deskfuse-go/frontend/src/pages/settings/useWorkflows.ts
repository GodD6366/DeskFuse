import { ref } from 'vue'
import { backend } from '../../services/backend'
import { errorText, get, keyOf } from './util'
import type { WorkflowDefinition } from './workflow'

interface Catalog {
  sync_status: string
  peers: { device_id: string; status: string }[]
  conflicts: { id: string; name: string; message: string }[]
  issues: { workflow_id: string; message: string }[]
  error: string | null
}

const emptyCatalog = (): Catalog => ({ sync_status: '', peers: [], conflicts: [], issues: [], error: null })

// 模块级共享：设置里的工作流页与快捷键页读取同一份列表。
const definitions = ref<WorkflowDefinition[]>([])
const catalog = ref<Catalog>(emptyCatalog())
const runs = ref<any[]>([])
const loaded = ref(false)
const loadError = ref<string | null>(null)
const loading = ref(false)

function normalize(raw: any): WorkflowDefinition {
  return { ...raw, shortcut: keyOf(raw.shortcut) } as WorkflowDefinition
}

async function reload() {
  loading.value = true
  loadError.value = null
  try {
    const result = await backend.invoke('workflow.list')
    definitions.value = ((get(result, 'definitions') ?? []) as any[]).map(normalize)
    catalog.value = { ...emptyCatalog(), ...(get(result, 'catalog') ?? {}) }
    runs.value = get(result, 'runs') ?? []
    loaded.value = true
  } catch (e) {
    loadError.value = errorText(e)
  } finally {
    loading.value = false
  }
}

async function save(flow: WorkflowDefinition) {
  await backend.invoke('workflow.save', { workflow: flow })
  await reload()
  if (catalog.value.error) throw new Error(catalog.value.error)
}

async function remove(id: string) {
  await backend.invoke('workflow.delete', { id, confirm: true })
  await reload()
  if (catalog.value.error) throw new Error(catalog.value.error)
}

async function restoreConflict(id: string) {
  await backend.invoke('workflow.restore_conflict', { id })
  await reload()
}

async function cancelRun(id: string) {
  await backend.invoke('workflow.cancel', { run_id: id })
}

export function useWorkflows() {
  return { definitions, catalog, runs, loaded, loading, loadError, reload, save, remove, restoreConflict, cancelRun }
}

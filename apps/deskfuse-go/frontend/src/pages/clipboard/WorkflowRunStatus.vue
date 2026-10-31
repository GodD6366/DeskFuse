<script setup lang="ts">
// 最近的工作流运行，对应原生 WorkflowRunStatusView：
// 运行中转圈；完成后显示输出摘要与「输入字节 → 输出字节」；失败用警示色。
import { computed, watch } from 'vue'
import { backend } from '../../services/backend'
import { byteCount } from './format'
import Icon from './Icon.vue'
import {
  ensureWorkflowNames, outputFormatName, workflowName, type WorkflowRun,
} from './workflowRuns'

const props = withDefaults(defineProps<{
  runs: WorkflowRun[]
  error?: string
  /** 面板为 compact：只显示最近 1 条，message 最多 2 行。 */
  compact?: boolean
}>(), { error: '', compact: false })

const visible = computed(() => props.runs.slice(0, props.compact ? 1 : 3))
// 没有运行时不必请求工作流定义；有运行后再按需取一次名称映射。
watch(() => props.runs.length, (count) => { if (count) ensureWorkflowNames() }, { immediate: true })

function title(run: WorkflowRun): string {
  const name = workflowName(run.workflowId)
  switch (run.phase) {
    case 'running': return run.stepIndex === null ? name : `${name} · 第 ${run.stepIndex + 1} 步`
    case 'completed': return `${name} · 已完成`
    case 'cancelled': return `${name} · 已取消`
    case 'failed': return `${name} · 失败`
    default: return name
  }
}

function phaseIcon(run: WorkflowRun): string {
  switch (run.phase) {
    case 'completed': return 'checkCircle'
    case 'cancelled': return 'xmarkCircle'
    default: return 'warning'
  }
}

function outputSummary(run: WorkflowRun): string {
  if (run.outputKind === 'image' && run.width > 0 && run.height > 0) {
    return `${outputFormatName(run.outputMime)} · ${run.width} × ${run.height} px`
  }
  return run.outputKind === 'text' ? '文字结果' : `${run.outputPaths.length} 个文件或文件夹`
}

async function cancel(run: WorkflowRun) {
  try {
    await backend.invoke('workflow.cancel', { run_id: run.id })
  } catch {
    // 取消失败时保持现状；下一轮快照仍会显示真实状态。
  }
}
</script>

<template>
  <div v-if="visible.length || error" class="runs" :class="{ compact }" aria-label="工作流运行状态">
    <div v-for="run in visible" :key="run.id" class="run" :class="{ failed: run.phase === 'failed' }">
      <span class="phase">
        <span v-if="run.phase === 'running'" class="spinner" role="status" aria-label="正在运行" />
        <Icon v-else :name="phaseIcon(run)" :size="14" />
      </span>
      <div class="body">
        <div class="title" :title="title(run)">{{ title(run) }}</div>
        <p v-if="run.message" class="message">{{ run.message }}</p>
        <div v-if="run.phase === 'completed' && run.outputBytes > 0" class="output">
          <span>{{ outputSummary(run) }}</span>
          <span class="bytes">{{ byteCount(run.inputBytes) }} → {{ byteCount(run.outputBytes) }}</span>
        </div>
      </div>
      <button v-if="run.phase === 'running'" type="button" class="cancel" @click="cancel(run)">取消</button>
    </div>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>

<style scoped>
.runs { display: flex; flex-direction: column; gap: 8px; padding: 12px; }
.runs.compact { padding: 4px; gap: 6px; }
.run { display: flex; align-items: flex-start; gap: 10px; min-width: 0; }
.phase {
  flex: none; width: 16px; min-height: 17px; display: flex; align-items: center; justify-content: center;
  color: var(--secondary, #595959);
}
.run.failed .phase { color: var(--warning, #ed8b23); }
.body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.title {
  font-size: 12px; font-weight: 600; line-height: 1.3; color: var(--text, #222);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.message {
  margin: 0; font-size: 12px; line-height: 1.4; color: var(--secondary, #595959);
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere;
}
.runs.compact .message { -webkit-line-clamp: 2; }
.output { display: flex; flex-direction: column; gap: 2px; font-size: 12px; line-height: 1.4; color: var(--secondary, #595959); }
.output .bytes { font-variant-numeric: tabular-nums; }
.error {
  margin: 0; font-size: 12px; line-height: 1.4; color: var(--danger, #e53935);
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere;
}
.runs.compact .error { -webkit-line-clamp: 2; }
.cancel { flex: none; font-size: 11px; line-height: 1.5; padding: 2px 8px; border-radius: 6px; color: var(--secondary, #595959); }
.cancel:hover { background: var(--control, rgba(0, 0, 0, 0.04)); color: var(--text, #222); }
.spinner {
  width: 12px; height: 12px; border-radius: 50%;
  border: 2px solid color-mix(in srgb, var(--secondary, #595959) 30%, transparent);
  border-top-color: var(--secondary, #595959); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>

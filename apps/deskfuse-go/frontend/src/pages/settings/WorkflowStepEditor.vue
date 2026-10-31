<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, CircleMinus } from 'lucide-vue-next'
import { backend } from '../../services/backend'
import { errorText } from './util'
import { ACTIONS, ACTION_ICONS, CONTENT_TITLES, actionTitle, isTerminal, type WorkflowContent, type WorkflowStep } from './workflow'

const step = defineModel<WorkflowStep>({ required: true })
defineProps<{ index: number; canMoveUp: boolean; canMoveDown: boolean }>()
const emit = defineEmits<{ move: [offset: number]; remove: [] }>()

interface SystemShortcut { id: string; name: string }
const shortcuts = ref<SystemShortcut[]>([])
const loading = ref(false)
const failure = ref<string | null>(null)
const openError = ref<string | null>(null)

async function refresh() {
  loading.value = true
  failure.value = null
  try {
    shortcuts.value = (await backend.invoke('workflow.system_shortcuts')) as SystemShortcut[]
  } catch (e) {
    failure.value = errorText(e)
  } finally {
    loading.value = false
  }
}
onMounted(() => { if (step.value.kind === 'SystemShortcut') refresh() })

const shortcutValue = () => step.value.local_shortcut_id ?? step.value.shortcut_id
function chooseShortcut(id: string) {
  step.value.local_shortcut_id = id
  step.value.shortcut_id = id
  step.value.shortcut_name = shortcuts.value.find((s) => s.id === id)?.name ?? ''
}
const savedMissing = () => {
  const current = shortcutValue()
  return !!current && !shortcuts.value.some((s) => s.id === current)
}
async function editShortcut() {
  openError.value = null
  try {
    await backend.invoke('system.open_url', { url: `shortcuts://open-shortcut?name=${encodeURIComponent(step.value.shortcut_name)}` })
  } catch (e) {
    openError.value = errorText(e)
  }
}

function setCompressMode(target: boolean) { step.value.target_bytes = target ? 512_000 : null }
function setResizeMode(edge: boolean) {
  step.value.max_edge = edge ? 2048 : null
  step.value.scale_percent = edge ? null : 75
}
function setTargetKiB(value: number) { step.value.target_bytes = Math.min(Math.max(0, value || 0), 131_072) * 1024 }
</script>

<template>
  <div class="step">
    <div class="head">
      <span class="no">{{ index + 1 }}</span>
      <component :is="ACTION_ICONS[step.kind]" :size="14" class="kind-icon" />
      <strong>{{ actionTitle(step.kind) }}</strong>
      <span class="spacer" />
      <template v-if="!isTerminal(step.kind)">
        <button type="button" class="icon-button" :disabled="!canMoveUp" title="上移" aria-label="上移" @click="emit('move', -1)"><ArrowUp :size="14" /></button>
        <button type="button" class="icon-button" :disabled="!canMoveDown" title="下移" aria-label="下移" @click="emit('move', 1)"><ArrowDown :size="14" /></button>
        <button type="button" class="icon-button danger-icon" title="移除步骤" aria-label="移除步骤" @click="emit('remove')"><CircleMinus :size="14" /></button>
      </template>
    </div>

    <div class="params">
      <template v-if="step.kind === 'Compress'">
        <label class="line">模式
          <select class="field" :value="step.target_bytes !== null ? 'target' : 'quality'" @change="setCompressMode(($event.target as HTMLSelectElement).value === 'target')">
            <option value="quality">质量</option>
            <option value="target">目标体积</option>
          </select>
        </label>
        <template v-if="step.target_bytes !== null">
          <label class="line">最终输出上限
            <input class="field narrow" type="number" min="0" max="131072" :value="Math.floor((step.target_bytes ?? 512000) / 1024)" @input="setTargetKiB(Number(($event.target as HTMLInputElement).value))" />
            KiB
          </label>
          <p class="note">在后续转换和快捷指令完成后检查；无法达标时不会复制或粘贴。多个上限取最严格值。</p>
        </template>
        <label v-else class="line">质量
          <input v-model.number="step.quality" type="range" min="1" max="100" step="1" /><span class="num">{{ step.quality }}%</span>
        </label>
      </template>

      <template v-else-if="step.kind === 'Resize'">
        <label class="line">方式
          <select class="field" :value="step.max_edge !== null ? 'edge' : 'scale'" @change="setResizeMode(($event.target as HTMLSelectElement).value === 'edge')">
            <option value="edge">最长边</option>
            <option value="scale">比例</option>
          </select>
        </label>
        <label v-if="step.max_edge !== null" class="line">最长边（px）
          <input v-model.number="step.max_edge" class="field narrow" type="number" min="1" />
        </label>
        <label v-else class="line">缩放比例：{{ step.scale_percent ?? 75 }}%
          <input v-model.number="step.scale_percent" type="range" min="1" max="100" step="1" />
        </label>
        <p class="note">保持比例，不放大图片。</p>
      </template>

      <template v-else-if="step.kind === 'Convert'">
        <label class="line">格式
          <select v-model="step.format" class="field">
            <option value="Png">PNG</option><option value="Jpeg">JPEG</option>
            <option value="Webp">WebP</option><option value="Heic">HEIC</option>
          </select>
        </label>
        <label v-if="step.format !== 'Png'" class="line">质量
          <input v-model.number="step.quality" type="range" min="1" max="100" step="1" /><span class="num">{{ step.quality }}%</span>
        </label>
        <p class="note">{{ step.format === 'Jpeg' || step.format === 'Heic' ? '透明区域使用白色背景；保持当前尺寸。' : '保留透明度及当前尺寸。' }}</p>
      </template>

      <template v-else-if="step.kind === 'TextReplace'">
        <input v-model="step.find" class="field" placeholder="查找" />
        <input v-model="step.replacement" class="field" placeholder="替换为" />
        <p class="note">区分大小写，按原文匹配，不使用正则表达式。</p>
      </template>

      <p v-else-if="step.kind === 'TextTrim'" class="note">仅去除整段文字首尾的空格、制表符和换行，保留段落内部格式。</p>

      <template v-else-if="step.kind === 'Rename'">
        <input v-model="step.find" class="field" placeholder="名称中查找（可留空）" />
        <input v-model="step.replacement" class="field" placeholder="替换为" />
        <input v-model="step.prefix" class="field" placeholder="添加前缀" />
        <input v-model="step.suffix" class="field" placeholder="添加后缀" />
        <p class="note">只更改所选顶层文件及文件夹的输出名称；保留扩展名和内部结构。重名自动编号，不修改源文件。</p>
      </template>

      <p v-else-if="step.kind === 'Zip'" class="note">将所选文件与文件夹打成一个 ZIP，保留目录层级，不修改源文件。</p>

      <template v-else-if="step.kind === 'SystemShortcut'">
        <label class="line">快捷指令
          <select class="field" :value="shortcutValue()" @change="chooseShortcut(($event.target as HTMLSelectElement).value)">
            <option value="">请选择…</option>
            <option v-if="savedMissing()" :value="shortcutValue()">{{ step.shortcut_name || '已保存的快捷指令（请刷新核对）' }}</option>
            <option v-for="item in shortcuts" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
        </label>
        <p v-if="step.local_shortcut_id === ''" class="warn">此设备尚未绑定快捷指令，请从本机列表选择。</p>
        <div class="line">
          <button type="button" class="button" :disabled="loading" @click="refresh">{{ loading ? '读取中…' : '刷新列表' }}</button>
          <button type="button" class="button" :disabled="!step.shortcut_name" @click="editShortcut">在快捷指令中编辑</button>
        </div>
        <label class="line">返回内容类型
          <select v-model="step.shortcut_output" class="field">
            <option v-for="(title, value) in CONTENT_TITLES" :key="value" :value="value as WorkflowContent">{{ title }}</option>
          </select>
        </label>
        <p class="note">快捷指令绑定只保存在本机；其他设备需要选择自己的快捷指令。文件以工作副本传入；返回内容按声明类型校验。交互步骤需要你响应。</p>
        <p v-if="failure" class="err" role="alert">{{ failure }}</p>
        <p v-if="openError" class="err" role="alert">{{ openError }}</p>
      </template>

      <template v-else>
        <label class="line">最后执行
          <select v-model="step.kind" class="field">
            <option v-for="a in ACTIONS.filter((x) => isTerminal(x.kind))" :key="a.kind" :value="a.kind">{{ a.title }}</option>
          </select>
        </label>
        <label class="line">图片输出形式
          <select v-model="step.output_shape" class="field">
            <option value="FollowSource">跟随来源</option>
            <option value="Image">图片内容</option>
            <option value="File">图片文件</option>
          </select>
        </label>
        <p class="note">历史记录入口只复制处理结果；共享入口只发送结果，不执行本机复制或粘贴。图片输出形式不影响文字和文件。</p>
      </template>
    </div>
  </div>
</template>

<style scoped>
.step { display: grid; gap: 12px; padding: 8px 0; }
.head { display: flex; align-items: center; gap: 8px; }
.no { color: var(--secondary); font-variant-numeric: tabular-nums; }
.kind-icon { color: var(--secondary); }
.head strong { font-size: 13px; font-weight: 600; }
.spacer { flex: 1; }
.danger-icon { color: var(--danger); }
.params { display: grid; gap: 8px; }
.line { display: flex; align-items: center; gap: 8px; font-size: 13px; white-space: nowrap; }
.line .field { flex: 1; min-width: 0; }
.line select.field { flex: 0 1 240px; min-width: 160px; margin-left: auto; }
.narrow { max-width: 120px; flex: 0 !important; }
.num { min-width: 44px; font-variant-numeric: tabular-nums; }
.note { margin: 0; font-size: 11px; line-height: 1.5; color: var(--secondary); }
.warn { margin: 0; font-size: 11px; line-height: 1.5; color: var(--warning); }
.err { margin: 0; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>

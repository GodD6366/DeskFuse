<script setup lang="ts">
import { computed, ref, toRaw } from 'vue'
import SegmentedControl from './SegmentedControl.vue'
import WorkflowConnector from './WorkflowConnector.vue'
import WorkflowStepCard from './WorkflowStepCard.vue'
import {
  CONTENT_TITLES, contentAfter, contentFlow, isTerminal, newStep, stepAccepts,
  type WorkflowAction, type WorkflowContent, type WorkflowDefinition, type WorkflowStep,
} from './workflow'

const MAX_STEPS = 32
const flow = defineModel<WorkflowDefinition>({ required: true })

const steps = computed(() => flow.value.steps)
const hasEnd = computed(() => steps.value.length > 0 && isTerminal(steps.value[steps.value.length - 1].kind))
/** 终点之前可插入的位置数：位置 p 表示插到原下标 p 的步骤之前。 */
const endIndex = computed(() => (hasEnd.value ? steps.value.length - 1 : steps.value.length))
const end = computed(() => (hasEnd.value ? steps.value[endIndex.value] : null))
const incoming = computed(() => contentFlow(flow.value))
const full = computed(() => steps.value.length >= MAX_STEPS)

function contentAt(position: number): WorkflowContent {
  if (position < steps.value.length) return incoming.value[position]
  const last = steps.value[steps.value.length - 1]
  return last ? contentAfter(last, incoming.value[incoming.value.length - 1]) : flow.value.input_type
}
const rejected = (index: number): WorkflowContent | null =>
  stepAccepts(steps.value[index].kind, incoming.value[index]) ? null : incoming.value[index]

// 步骤本身没有 id：用对象身份做稳定 key，排序和插入后卡片状态不会错位。
const keys = new WeakMap<object, number>()
let nextKey = 0
function stepKey(step: WorkflowStep) {
  const raw = toRaw(step)
  if (!keys.has(raw)) keys.set(raw, nextKey++)
  return keys.get(raw)!
}

// 同一时间只展开一张；新加的步骤默认展开。
const openStep = ref<WorkflowStep | null>(null)
const isOpen = (step: WorkflowStep) => !!openStep.value && toRaw(openStep.value) === toRaw(step)
const toggle = (step: WorkflowStep) => { openStep.value = isOpen(step) ? null : step }

function insert(position: number, kind: WorkflowAction) {
  if (full.value) return
  flow.value.steps.splice(position, 0, newStep(kind))
  openStep.value = flow.value.steps[position]
}

function remove(index: number) {
  if (isOpen(steps.value[index])) openStep.value = null
  flow.value.steps.splice(index, 1)
}

function move(index: number, offset: number) {
  const list = flow.value.steps
  ;[list[index], list[index + offset]] = [list[index + offset], list[index]]
}

// 拖拽排序（HTML5 原生）：只能拖中间步骤，且不能越过终点。
const dragFrom = ref<number | null>(null)
const dropAt = ref<number | null>(null)
const effectiveDrop = computed(() =>
  dragFrom.value === null || dropAt.value === null || dropAt.value === dragFrom.value || dropAt.value === dragFrom.value + 1 ? null : dropAt.value)

function onDragStart(index: number, event: DragEvent) {
  dragFrom.value = index
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', String(index))
  }
}
function onDragEnd() { dragFrom.value = null; dropAt.value = null }

function overPosition(position: number, event: DragEvent) {
  if (dragFrom.value === null) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
  dropAt.value = Math.min(position, endIndex.value)
}
function overCard(index: number, event: DragEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  overPosition(event.clientY < rect.top + rect.height / 2 ? index : index + 1, event)
}
function onDrop(event: DragEvent) {
  if (dragFrom.value === null) return
  event.preventDefault()
  const from = dragFrom.value
  const target = effectiveDrop.value
  onDragEnd()
  if (target === null) return
  const list = flow.value.steps
  const [moved] = list.splice(from, 1)
  list.splice(target > from ? target - 1 : target, 0, moved)
}

const END_OPTIONS: { value: WorkflowAction; label: string }[] = [
  { value: 'Copy', label: '复制到剪贴板' },
  { value: 'Paste', label: '粘贴到当前应用' },
]
</script>

<template>
  <div class="timeline">
    <div class="node start" @dragover="overPosition(0, $event)" @drop="onDrop">
      <div class="rail"><span class="dot ring" /></div>
      <span class="start-text">输入：{{ CONTENT_TITLES[flow.input_type] }}</span>
    </div>

    <template v-for="(step, index) in steps.slice(0, endIndex)" :key="stepKey(step)">
      <WorkflowConnector :content="incoming[index]" :bad="rejected(index) !== null" :full="full" :drop="effectiveDrop === index"
        @pick="insert(index, $event)" @dragover="overPosition(index, $event)" @drop="onDrop" />
      <div class="node" :class="{ bad: rejected(index) !== null }" @dragover="overCard(index, $event)" @drop="onDrop">
        <div class="rail"><span class="num" :class="{ bad: rejected(index) !== null }">{{ index + 1 }}</span></div>
        <WorkflowStepCard v-model="flow.steps[index]" :index="index" :open="isOpen(step)" :rejected="rejected(index)"
          :can-move-up="index > 0" :can-move-down="index + 1 < endIndex" :dragging="dragFrom === index"
          @toggle="toggle(step)" @move="move(index, $event)" @remove="remove(index)" @dragstart="onDragStart(index, $event)" @dragend="onDragEnd" />
      </div>
    </template>

    <WorkflowConnector persistent :content="contentAt(endIndex)" :bad="false" :full="full" :drop="effectiveDrop === endIndex"
      @pick="insert(endIndex, $event)" @dragover="overPosition(endIndex, $event)" @drop="onDrop" />

    <div v-if="end" class="node last" @dragover="overPosition(endIndex, $event)" @drop="onDrop">
      <div class="rail"><span class="dot solid" /></div>
      <div class="end card">
        <div class="end-head">
          <strong>最后</strong>
          <SegmentedControl :model-value="end.kind" :options="END_OPTIONS" label="最后执行" @update:model-value="end.kind = $event" />
        </div>
        <label class="line">图片输出形式
          <select v-model="end.output_shape" class="field">
            <option value="FollowSource">跟随来源</option>
            <option value="Image">图片内容</option>
            <option value="File">图片文件</option>
          </select>
        </label>
        <p class="note">从历史记录运行只复制结果；共享给设备时只发送结果。</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline { display: grid; min-width: 0; }
.node { position: relative; display: grid; grid-template-columns: 28px minmax(0, 1fr); gap: 8px; }
.rail { position: relative; display: flex; justify-content: center; padding-top: 14px; }
.rail::before { content: ''; position: absolute; top: 0; bottom: 0; left: 50%; width: 1px; transform: translateX(-0.5px); background: color-mix(in srgb, var(--text) 18%, transparent); }
.num, .dot { position: relative; flex: none; }
.num {
  display: inline-grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; background: var(--surface);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--text) 22%, transparent); font-size: 11px; font-variant-numeric: tabular-nums; color: var(--secondary);
}
.num.bad { color: var(--danger); box-shadow: inset 0 0 0 1px var(--danger); }

.start { align-items: center; min-height: 24px; }
.start .rail { align-items: center; padding-top: 0; }
.start .rail::before { top: 50%; }
.dot { border-radius: 50%; }
.dot.ring { width: 10px; height: 10px; background: var(--canvas); box-shadow: inset 0 0 0 1.5px var(--secondary); }
.dot.solid { width: 10px; height: 10px; margin-top: 5px; background: var(--accent); }
.start-text { font-size: 12px; font-weight: 500; color: var(--secondary); }

.last .rail::before { bottom: auto; height: 24px; }
/* 不兼容的步骤：圆点上方这一小段与它上方的连线同色。 */
.node.bad .rail::before { background: linear-gradient(var(--danger) 0 24px, color-mix(in srgb, var(--text) 18%, transparent) 24px); }
.end {
  display: grid; gap: 10px; min-width: 0; padding: 10px 16px 14px; border: 0; border-radius: 12px;
  background: color-mix(in srgb, var(--accent) 5%, var(--surface)); box-shadow: inset 0 0 0 0.5px color-mix(in srgb, var(--accent) 35%, var(--line));
}
.end-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
.end-head strong { font-size: 13px; font-weight: 600; }
.line { display: flex; align-items: center; gap: 8px; font-size: 13px; white-space: nowrap; }
.line select.field { flex: 0 1 240px; min-width: 160px; margin-left: auto; }
.note { margin: 0; font-size: 11px; line-height: 1.5; color: var(--secondary); }
</style>

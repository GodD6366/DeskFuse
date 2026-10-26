<script setup lang="ts">
import { computed, reactive, ref } from 'vue'

defineProps<{ active?: boolean }>()

type Kind = 'text' | 'image' | 'file'

interface Clip {
  id: number
  kind: Kind
  text: string
  /** 图片里识别出的文字，搜索时一起匹配 */
  ocr?: string
  from: string
  fresh?: boolean
}

const sources: Omit<Clip, 'id' | 'from'>[] = [
  { kind: 'text', text: '会议改到周四下午三点，地点不变' },
  { kind: 'image', text: '截图 2026-10-24 14.32', ocr: '发票 金额 ¥1,280.00' },
  { kind: 'file', text: '季度报告.pdf' },
]

let nextId = 10
const history = reactive<Clip[]>([
  { id: 2, kind: 'text', text: 'https://deskfuse.godd.cc', from: 'Mac mini' },
  { id: 1, kind: 'image', text: '白板照片', ocr: '上线计划 第二阶段', from: 'iMac' },
])

const filter = ref<'all' | Kind>('all')
const query = ref('')
const copiedIndex = ref<number | null>(null)

const tabs: { id: 'all' | Kind; label: string }[] = [
  { id: 'all', label: '全部' },
  { id: 'text', label: '文字' },
  { id: 'image', label: '图片' },
  { id: 'file', label: '文件' },
]

const shown = computed(() => {
  const q = query.value.trim().toLowerCase()
  return history.filter(
    (c) =>
      (filter.value === 'all' || c.kind === filter.value) &&
      (!q || c.text.toLowerCase().includes(q) || c.ocr?.toLowerCase().includes(q)),
  )
})

function copy(i: number) {
  history.forEach((c) => (c.fresh = false))
  history.unshift({ ...sources[i], id: nextId++, from: 'MacBook Pro', fresh: true })
  if (history.length > 6) history.pop()
  filter.value = 'all'
  query.value = ''
  copiedIndex.value = i
  setTimeout(() => {
    if (copiedIndex.value === i) copiedIndex.value = null
  }, 1400)
}

const kindLabel: Record<Kind, string> = { text: '文字', image: '图片', file: '文件' }
</script>

<template>
  <div class="sc-host">
    <div class="sc">
      <div class="sc__copy">
        <h2>这台复制，那台粘贴</h2>
        <p>
          文字、图片和应用自带的格式原样同步，文件也能直接粘贴过去。复制过的内容按类型留在历史里，按
          <kbd>⌥</kbd> <kbd>⌘</kbd> <kbd>V</kbd> 随时翻出来。
        </p>
        <ul class="sc__list">
          <li>图片里的文字也搜得到，试试在右边搜「发票」</li>
          <li>可以排除密码管理器之类的应用，不同步它们的内容</li>
        </ul>
      </div>

      <div class="sc__demo">
        <div class="clip">
          <div class="clip__source">
            <span class="clip__device">MacBook Pro</span>
            <ul>
              <li v-for="(s, i) in sources" :key="s.text">
                <span class="clip__thumb" :class="`t-${s.kind}`" aria-hidden="true"></span>
                <span class="clip__text">{{ s.text }}</span>
                <button class="btn btn--quiet btn--small" type="button" @click="copy(i)">
                  {{ copiedIndex === i ? '已复制' : '复制' }}
                </button>
              </li>
            </ul>
          </div>

          <div class="clip__panel">
            <span class="clip__device">Mac mini 的剪贴板历史</span>
            <input v-model="query" class="clip__search" type="search" placeholder="搜索文字或图片里的字" aria-label="搜索剪贴板历史" />
            <div class="clip__tabs" role="tablist" aria-label="按类型筛选">
              <button
                v-for="t in tabs"
                :key="t.id"
                type="button"
                role="tab"
                :aria-selected="filter === t.id"
                :class="{ 'is-on': filter === t.id }"
                @click="filter = t.id"
              >
                {{ t.label }}
              </button>
            </div>
            <TransitionGroup tag="ul" name="clip" class="clip__list">
              <li v-for="c in shown" :key="c.id" :class="{ 'is-fresh': c.fresh }">
                <span class="clip__thumb" :class="`t-${c.kind}`" aria-hidden="true"></span>
                <span class="clip__text">
                  {{ c.text }}
                  <small>{{ kindLabel[c.kind] }}，来自 {{ c.from }}<template v-if="query && c.ocr && c.ocr.includes(query.trim())">，图中文字匹配</template></small>
                </span>
              </li>
            </TransitionGroup>
            <p v-if="!shown.length" class="clip__empty">没有找到，换个词试试</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.clip {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr);
  gap: 14px;
  padding: 14px;
  border-radius: 16px;
  background: #e3e1dc;
}

.clip__source,
.clip__panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  padding: 14px;
  border-radius: 12px;
  background: #fff;
}

.clip__device {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--c-muted);
}

.clip ul {
  display: grid;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.clip__source li,
.clip__list li {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  padding: 8px;
  border-radius: 8px;
  background: var(--c-paper);
  font-size: 0.8125rem;
}

.clip__source .btn {
  min-height: 28px;
  padding: 0 10px;
  font-size: 0.75rem;
  flex: none;
}

.clip__thumb {
  width: 30px;
  height: 30px;
  border-radius: 6px;
  flex: none;
  background: var(--c-desk);
}

.t-text {
  background: repeating-linear-gradient(180deg, #c9c6bf 0 2px, transparent 2px 6px) 7px 8px / 16px 14px no-repeat,
    var(--c-desk);
}

.t-image {
  background: linear-gradient(160deg, #f6a77f, #f05a24 55%, #2d2f35);
}

.t-file {
  background: linear-gradient(180deg, #fff 0 0) 9px 6px / 12px 18px no-repeat, var(--c-graphite);
}

.clip__text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.clip__text small {
  display: block;
  color: var(--c-muted);
  font-size: 0.6875rem;
}

.clip__search {
  width: 100%;
  padding: 6px 10px;
  border: 0;
  border-radius: 7px;
  background: var(--c-paper);
  box-shadow: inset 0 0 0 1px var(--c-line);
  font: inherit;
  font-size: 0.8125rem;
}

.clip__tabs {
  display: flex;
  gap: 4px;
}

.clip__tabs button {
  padding: 2px 10px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--c-muted);
  font: inherit;
  font-size: 0.75rem;
  font-weight: 600;
  cursor: pointer;
}

.clip__tabs button.is-on {
  background: var(--c-graphite);
  color: #fff;
}

.clip__list li.is-fresh {
  background: rgba(240, 90, 36, 0.1);
  box-shadow: inset 3px 0 0 var(--c-signal);
}

.clip__empty {
  color: var(--c-muted);
  font-size: 0.8125rem;
}

.clip-enter-active,
.clip-move {
  transition:
    opacity 0.3s var(--ease),
    transform 0.3s var(--ease);
}

.clip-enter-from {
  opacity: 0;
  transform: translateX(-18px);
}

.clip-leave-active {
  display: none;
}

@container (max-width: 640px) {
  .clip {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>

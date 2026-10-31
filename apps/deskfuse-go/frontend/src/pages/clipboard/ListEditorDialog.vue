<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { backend } from '../../services/backend'
import type { LibraryList, LibraryRule } from '../../types/clipboard'
import Dialog from './Dialog.vue'
import Icon from './Icon.vue'
import { newKey } from './format'
import { errorMessage, library, saveList } from './library'

const props = defineProps<{ initial: LibraryList }>()
const emit = defineEmits<{ saved: [id: string]; close: [] }>()

const draft = reactive<LibraryList>({
  ...props.initial,
  rules: props.initial.rules.map((r) => ({ ...r })),
})
const saving = ref(false)
const error = ref<string | null>(null)
const appMenuFor = ref<string | null>(null)

const smart = computed(() => draft.kind === 'smart')

const operators = (field: string): [string, string][] => {
  switch (field) {
    case 'text': return [['contains', '包含'], ['not_contains', '不包含']]
    case 'date': return [['today', '今天'], ['yesterday', '昨天'], ['last_days', '最近天数'], ['range', '日期范围']]
    default: return [['is', '是'], ['is_not', '不是']]
  }
}

const formatDay = (date: Date) =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`

function setField(rule: LibraryRule, field: string) {
  rule.field = field
  rule.op = operators(field)[0][0]
  rule.value = ''
  rule.value2 = ''
}

function setOp(rule: LibraryRule, op: string) {
  rule.op = op
  if (op === 'range') { rule.value = formatDay(new Date()); rule.value2 = formatDay(new Date()) }
  if (op === 'last_days') rule.value = '7'
}

function addRule() {
  draft.rules.push({ key: newKey(), field: 'text', op: 'contains', value: '', value2: '' })
}

const invalidRules = computed(() =>
  smart.value && draft.rules.some((r) =>
    !['today', 'yesterday'].includes(r.op)
    && (r.value === '' || (r.op === 'last_days' && (Number.parseInt(r.value, 10) || 0) < 1) || (r.op === 'range' && r.value2 === ''))))

const canSave = computed(() => !saving.value && draft.name.trim() !== '' && !invalidRules.value)

async function chooseOtherApp(rule: LibraryRule) {
  appMenuFor.value = null
  try {
    // 宿主返回 {bundle_id: string | null}，null 表示用户取消。
    const result = (await backend.invoke('clipboard.chooseApp')) as { bundle_id?: string | null } | null
    const bundle = result?.bundle_id
    if (bundle) rule.value = bundle
  } catch (e) {
    error.value = errorMessage(e)
  }
}

async function save() {
  if (!canSave.value) return
  saving.value = true
  error.value = null
  try {
    draft.name = draft.name.trim()
    await saveList(draft)
    emit('saved', draft.id)
    emit('close')
  } catch (e) {
    error.value = errorMessage(e)
    saving.value = false
  }
}
</script>

<template>
  <Dialog :width="620" :label="smart ? '编辑智能列表' : '编辑手动列表'" :dismissible="!saving" @close="emit('close')">
    <form class="body" @submit.prevent="save">
      <h2>{{ smart ? '编辑智能列表' : '编辑手动列表' }}</h2>
      <input v-model="draft.name" class="text" type="text" placeholder="列表名称" :disabled="saving" autofocus />

      <template v-if="smart">
        <div class="match">
          <span>包含匹配下列条件的内容</span>
          <div class="segmented" role="radiogroup" aria-label="包含匹配下列条件的内容">
            <button type="button" role="radio" :aria-checked="draft.match_mode === 'all'" :class="{ on: draft.match_mode === 'all' }" @click="draft.match_mode = 'all'">所有条件</button>
            <button type="button" role="radio" :aria-checked="draft.match_mode === 'any'" :class="{ on: draft.match_mode === 'any' }" @click="draft.match_mode = 'any'">任一条件</button>
          </div>
        </div>
        <div class="rules">
          <div v-for="rule in draft.rules" :key="rule.key" class="rule">
            <div class="rule-fields">
              <div class="pair">
                <select :value="rule.field" aria-label="条件" style="width: 140px" @change="setField(rule, ($event.target as HTMLSelectElement).value)">
                  <option value="app">来自 App</option>
                  <option value="date">创建日期</option>
                  <option value="kind">内容类型</option>
                  <option value="text">包含文本</option>
                  <option value="device">来源设备</option>
                </select>
                <select :value="rule.op" aria-label="匹配方式" style="width: 130px" @change="setOp(rule, ($event.target as HTMLSelectElement).value)">
                  <option v-for="[value, label] in operators(rule.field)" :key="value" :value="value">{{ label }}</option>
                </select>
              </div>

              <select v-if="rule.field === 'kind'" v-model="rule.value" aria-label="内容类型">
                <option value="">请选择类型</option>
                <option value="text">文字</option>
                <option value="image">图片</option>
                <option value="files">文件</option>
                <option value="other">其他格式</option>
              </select>
              <select v-else-if="rule.field === 'device'" v-model="rule.value" aria-label="来源设备">
                <option value="">请选择设备</option>
                <option v-for="d in library.devices" :key="d.id" :value="d.id">{{ d.name }}</option>
                <option v-if="rule.value && !library.devices.some((d) => d.id === rule.value)" :value="rule.value">不可用设备 · {{ rule.value }}</option>
              </select>
              <div v-else-if="rule.field === 'app'" class="app">
                <input v-model="rule.value" class="text" type="text" placeholder="App bundle ID" />
                <div class="menu-wrap">
                  <button type="button" @click="appMenuFor = appMenuFor === rule.key ? null : rule.key">选择 App</button>
                  <div v-if="appMenuFor === rule.key" class="popup" role="menu">
                    <button v-for="app in library.apps" :key="app.bundle_id" type="button" role="menuitem" @click="rule.value = app.bundle_id; appMenuFor = null">{{ app.name }}</button>
                    <button type="button" role="menuitem" @click="chooseOtherApp(rule)">其他应用…</button>
                  </div>
                </div>
              </div>
              <template v-else-if="rule.field === 'date'">
                <input v-if="rule.op === 'last_days'" v-model="rule.value" class="text" type="text" inputmode="numeric" placeholder="天数" />
                <div v-if="rule.op === 'range'" class="dates">
                  <label>从<input v-model="rule.value" class="text" type="date" /></label>
                  <label>至<input v-model="rule.value2" class="text" type="date" /></label>
                </div>
              </template>
              <input v-else v-model="rule.value" class="text" type="text" placeholder="要匹配的完整文字" />
            </div>
            <button type="button" class="bare" aria-label="删除条件" :disabled="draft.rules.length === 1" @click="draft.rules.splice(draft.rules.indexOf(rule), 1)">
              <Icon name="minusCircle" :size="16" />
            </button>
          </div>
        </div>
        <button type="button" class="add" @click="addRule"><Icon name="plusCircle" :size="14" />增加条件</button>
      </template>
      <p v-else class="note">加入此列表的内容会长期保留，并在已配对设备间同步。删除列表不会删除其他列表的成员。</p>

      <p v-if="error" class="error" role="alert">{{ error }}</p>
      <div class="actions">
        <span class="spacer" />
        <button type="button" :disabled="saving" @click="emit('close')">取消</button>
        <button type="submit" class="primary" :disabled="!canSave">{{ saving ? '正在保存…' : '保存' }}</button>
      </div>
    </form>
  </Dialog>
</template>

<style scoped>
.body { padding: 24px; display: flex; flex-direction: column; gap: 16px; }
h2 { margin: 0; font-size: 20px; font-weight: 700; }
.text, select {
  height: 28px;
  padding: 0 8px;
  border-radius: 6px;
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.18));
  background: var(--surface, #fff);
  color: inherit;
  font: inherit;
  font-size: 13px;
  min-width: 0;
}
.text:focus, select:focus { outline: 2px solid var(--accent, #3276f3); outline-offset: -1px; }
.match { display: flex; align-items: center; gap: 12px; font-size: 13px; }
.segmented { display: inline-flex; padding: 2px; border-radius: 8px; background: var(--control, rgba(0, 0, 0, 0.06)); }
.segmented button { height: 24px; padding: 0 14px; border: 0; border-radius: 6px; background: transparent; color: inherit; font: inherit; font-size: 12px; }
.segmented button.on { background: var(--surface, #fff); box-shadow: 0 0.5px 2px rgba(0, 0, 0, 0.2); }
.rules { display: flex; flex-direction: column; gap: 12px; min-height: 160px; max-height: 300px; overflow: auto; }
.rule { display: flex; gap: 8px; align-items: flex-start; }
.rule-fields { flex: 1; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
.pair { display: flex; gap: 8px; }
.app { display: flex; gap: 8px; }
.app .text { flex: 1; }
.dates { display: flex; gap: 12px; }
.dates label { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.menu-wrap { position: relative; }
.popup {
  position: absolute; right: 0; top: 30px; z-index: 5; min-width: 180px; max-height: 220px; overflow: auto; padding: 5px;
  border-radius: 10px; background: var(--surface, #fff); border: 0.5px solid var(--line, rgba(0, 0, 0, 0.14));
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.18);
}
.popup button { display: block; width: 100%; text-align: left; height: 26px; padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: inherit; font: inherit; font-size: 13px; }
.popup button:hover { background: var(--accent, #3276f3); color: #fff; }
.note { margin: 0; font-size: 12px; color: var(--secondary, #6e6e73); line-height: 1.5; }
.error { margin: 0; font-size: 12px; color: var(--danger, #ff3b30); }
.actions { display: flex; gap: 10px; align-items: center; }
.spacer { flex: 1; }
button {
  height: 28px; padding: 0 14px; border-radius: 7px; border: 0.5px solid var(--line, rgba(0, 0, 0, 0.14));
  background: var(--control, rgba(0, 0, 0, 0.04)); color: var(--text, #1d1d1f); font: inherit; font-size: 13px; cursor: default;
}
button:disabled { opacity: 0.45; }
button.primary { background: var(--accent, #3276f3); border-color: transparent; color: #fff; }
button.bare { height: 28px; width: 28px; padding: 0; border: 0; background: transparent; color: var(--secondary, #6e6e73); display: inline-flex; align-items: center; justify-content: center; }
button.add { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; border: 0; background: transparent; color: var(--accent, #3276f3); padding: 0; }
</style>

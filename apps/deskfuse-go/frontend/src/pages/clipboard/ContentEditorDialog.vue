<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Bold, Italic, Underline, Strikethrough } from 'lucide-vue-next'
import Dialog from './Dialog.vue'
import { readRuns, writeRuns } from './richText'
import type { RichRun } from './richText'
const props=defineProps<{text:string;runs?:RichRun[];rtfAvailable?:boolean}>()
const emit=defineEmits<{save:[text:string,runs:RichRun[]];close:[]}>()
const plainOnly=ref(false),editable=ref<HTMLElement|null>(null)
onMounted(()=>{if(editable.value)writeRuns(editable.value,props.runs??[],props.text)})
function format(command:string){if(plainOnly.value)return;document.execCommand(command,false);editable.value?.focus()}
function save(){const root=editable.value;if(!root)return;const runs=readRuns(root);const text=runs.map(run=>run.text).join('');emit('save',text,plainOnly.value?[]:runs)}
function togglePlain(){plainOnly.value=!plainOnly.value;if(plainOnly.value&&editable.value){const text=readRuns(editable.value).map(run=>run.text).join('');writeRuns(editable.value,[],text)}}
</script>
<template><Dialog :width="620" label="编辑内容" @close="emit('close')"><div class="editor"><div class="toolbar"><button type="button" title="粗体" aria-label="粗体" :disabled="plainOnly" @click="format('bold')"><Bold :size="14"/></button><button type="button" title="斜体" aria-label="斜体" :disabled="plainOnly" @click="format('italic')"><Italic :size="14"/></button><button type="button" title="下划线" aria-label="下划线" :disabled="plainOnly" @click="format('underline')"><Underline :size="14"/></button><button type="button" title="删除线" aria-label="删除线" :disabled="plainOnly" @click="format('strikeThrough')"><Strikethrough :size="14"/></button><span/><button type="button" @click="togglePlain">{{plainOnly?'使用富文本':'使用纯文本'}}</button><button type="button" class="primary" @click="save">保存</button></div><div ref="editable" class="content" contenteditable="true" role="textbox" aria-label="剪贴板内容" aria-multiline="true" :class="{plain:plainOnly}" @keydown.meta.s.prevent="save" @keydown.ctrl.s.prevent="save"></div><p v-if="rtfAvailable">原条目含富文本样式；可编辑和保存粗体、斜体、下划线与删除线。</p></div></Dialog></template>
<style scoped>.editor{display:flex;flex-direction:column;min-height:340px}.toolbar{display:flex;align-items:center;gap:7px;padding:10px 14px;border-bottom:.5px solid var(--line)}.toolbar>button{height:28px;min-width:28px;border:1px solid var(--line);border-radius:6px;background:var(--control);display:grid;place-items:center}.toolbar>span{flex:1}.content{flex:1;min-height:270px;padding:18px;overflow:auto;outline:none;white-space:pre-wrap;overflow-wrap:anywhere;font:15px/1.5 -apple-system,BlinkMacSystemFont,sans-serif}.content:focus-visible{outline:2px solid color-mix(in srgb,var(--accent) 35%,transparent);outline-offset:-2px}.content.plain{font:15px/1.5 -apple-system,BlinkMacSystemFont,sans-serif!important}.editor p{font-size:11px;color:var(--secondary);padding:4px 18px 10px;margin:0}.primary{background:var(--accent)!important;color:#fff}</style>



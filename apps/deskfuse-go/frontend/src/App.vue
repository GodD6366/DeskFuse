<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, shallowRef } from 'vue'
import { Clipboard, Keyboard, ArrowUpDown, PanelsTopLeft, Mic, Mouse, Laptop, Settings, CircleHelp, RefreshCw, ChevronRight, X } from 'lucide-vue-next'
import { backend, errorMessage } from './services/backend'

const modules = import.meta.glob('./pages/*Page.vue')
const pageNames: Record<string,string> = { 'clipboard-panel':'ClipboardPanel','quick-look':'QuickLook', clipboard:'Clipboard',devices:'Devices',files:'Transfers',input:'Input',mouse:'Mouse',microphone:'Microphone',remote_applications:'RemoteApps',settings:'Settings',onboarding:'Onboarding',shelf:'Shelf',support:'Support',pairing:'Pairing','transfer-progress':'TransferProgress' }
const pages = [
 {id:'clipboard',title:'剪贴板',icon:Clipboard,section:'基础'}, {id:'input',title:'键鼠共享',icon:Keyboard,section:'基础'}, {id:'files',title:'局域网传输',icon:ArrowUpDown,section:'基础'},
 {id:'remote_applications',title:'远程应用',icon:PanelsTopLeft,section:'高级',beta:true}, {id:'microphone',title:'麦克风共享',icon:Mic,section:'高级',beta:true}, {id:'mouse',title:'鼠标自定义',icon:Mouse,section:'高级',beta:true},
 {id:'devices',title:'设备',icon:Laptop,section:'配置'}, {id:'settings',title:'设置',icon:Settings,section:'设置'},
]
const categories=[['general','通用'],['clipboard','剪贴板'],['workflows','工作流'],['shortcuts','快捷键'],['files','文件'],['permissions','权限'],['about','关于与更新'],['advanced','高级']]
const panelMode = new URLSearchParams(location.search).get('page') === 'clipboard-panel'
const selected = ref(panelMode ? 'clipboard-panel' : (new URLSearchParams(location.search).get('page') || 'clipboard'))
const category=ref('general'), snapshot=ref<Record<string,any>|null>(null), error=ref(''), loading=ref(true)
const component=shallowRef<any>(null), pageError=ref(''), help=ref(false), health=ref(false)
// 浮动窗口和引导页自己铺满窗口，不渲染侧栏与顶栏。
const fullBleedPages=new Set(['onboarding','pairing','transfer-progress','quick-look'])
const fullBleed=computed(()=>panelMode||fullBleedPages.has(selected.value))
const current=computed(()=>pages.find(p=>p.id===selected.value))
const paired=computed(()=>snapshot.value?.devices?.filter((d:any)=>d.trusted&&!d.blocked)??[])
const online=computed(()=>paired.value.filter((d:any)=>d.connected).length)
const needsFirstDevice=computed(()=>snapshot.value!==null&&paired.value.length===0)
type HealthIssue={id:string;tone:'danger'|'warning'|'accent';short:string;title:string;detail?:string;action?:string;params?:Record<string,unknown>;page?:string;category?:string}
// 与原生 HealthCenter 一致：先按严重程度收集问题，底部只显示结论。
const healthIssues=computed<HealthIssue[]>(()=>{
 const s:any=snapshot.value; if(!s) return []
 const issues:HealthIssue[]=[]
 const granted=(id:string)=>(s.permissions?.items??[]).some((p:any)=>p.id===id&&p.granted===true)
 if(s.app?.error) issues.push({id:'runtime',tone:'danger',short:'运行异常',title:'DeskFuse 后台服务出现问题',detail:String(s.app.error),action:'support.report',params:{error:String(s.app.error)}})
 const devices=(s.devices??[]).filter((d:any)=>d.trusted&&!d.blocked)
 if(!devices.length) issues.push({id:'no-device',tone:'accent',short:'未连接电脑',title:'还没有连接其他电脑',detail:'添加另一台电脑后，就能同步剪贴板、共享键鼠和传输文件。',page:'devices'})
 else if(!devices.some((d:any)=>d.connected)) issues.push({id:'offline',tone:'warning',short:'设备离线',title:'已添加的电脑都不在线',detail:'请确认对方已打开 DeskFuse，并和这台 Mac 在同一网络。',action:'devices.refresh'})
 if(s.input?.settings?.feature_enabled&&!granted('accessibility')) issues.push({id:'input-permission',tone:'warning',short:'需要授权',title:'键鼠共享需要「辅助功能」权限',detail:'授权后键盘和鼠标才能穿过屏幕边缘。',action:'permissions.open',params:{permission:'accessibility'}})
 else if(s.input?.settings?.feature_enabled&&s.input?.error) issues.push({id:'input',tone:'warning',short:'键鼠暂不可用',title:'键鼠共享暂时不可用',detail:String(s.input.error),page:'input'})
 if(s.features?.remote_applications&&!granted('screen_recording')) issues.push({id:'remote-permission',tone:'warning',short:'需要授权',title:'远程应用需要「屏幕录制」权限',detail:'用于在远端屏幕上显示本机内容。',action:'permissions.open',params:{permission:'screen_recording'}})
 if(s.clipboard?.error) issues.push({id:'clipboard',tone:'warning',short:'剪贴板异常',title:'剪贴板同步遇到问题',detail:String(s.clipboard.error),page:'clipboard'})
 if((s.permissions?.pending_relaunch??[]).length) issues.push({id:'relaunch',tone:'accent',short:'需要重新打开',title:'重新打开 DeskFuse 以启用刚授予的权限'})
 if(s.update?.target_version) issues.push({id:'update',tone:'accent',short:'有新版本',title:`可更新到 ${s.update.target_version}`,page:'settings',category:'about'})
 return issues
})
const healthTone=computed(()=>healthIssues.value.some(i=>i.tone==='danger')?'danger':healthIssues.value.some(i=>i.tone==='warning')?'warning':healthIssues.value.length?'accent':'success')
const healthText=computed(()=>healthIssues.value.length===0?'一切正常':healthIssues.value.length===1?healthIssues.value[0].short:`${healthIssues.value.length} 项需要留意`)
async function resolveIssue(issue:HealthIssue){
 try{
  if(issue.action) await backend.invoke(issue.action,issue.params)
  if(issue.page){ if(issue.category) category.value=issue.category; await select(issue.page) }
  health.value=false
 }catch(e){error.value=errorMessage(e)}
}
async function copyEnvironment(){try{await backend.invoke('support.copy_environment')}catch(e){error.value=errorMessage(e)}}
async function openURL(url:string){try{await backend.invoke('system.open_url',{url})}catch(e){error.value=errorMessage(e)}}
let timer:ReturnType<typeof setTimeout>|undefined, stopped=false, refreshing=false, generation=0
async function select(id:string){if(id.startsWith('settings:')){category.value=id.split(':')[1];id='settings'}selected.value=id;pageError.value='';const g=++generation;const load=modules[`./pages/${pageNames[id]}Page.vue`];if(!load){component.value=null;pageError.value='此页面尚未完成迁移';return}try{const mod=await load() as any;if(g===generation)component.value=mod.default}catch(e){if(g===generation)pageError.value=errorMessage(e)}}
async function refresh(){if(fullBleed.value){loading.value=false;return}if(refreshing)return;refreshing=true;try{const value=await backend.snapshot();if(!stopped){snapshot.value=value;error.value=''}}catch(e){if(!stopped)error.value=errorMessage(e)}finally{refreshing=false;loading.value=false;if(!stopped)timer=setTimeout(refresh,1500)}}
async function report(){try{await backend.invoke('support.report',{error:error.value});help.value=false}catch(e){error.value=errorMessage(e)}}
function retry(){if(timer)clearTimeout(timer);refresh()}
function keydown(e:KeyboardEvent){if((e.metaKey||e.ctrlKey)&&e.key===','){e.preventDefault();select('settings')}if(e.key==='Escape'){help.value=false;health.value=false}}
function navigateFromHost(page:string,categoryName?:string){if(categoryName)category.value=categoryName;void select(page)}
onMounted(()=>{
 ;(window as any).__deskfuseNavigate=navigateFromHost
 ;(window as any).__deskfuseSnapshot=()=>snapshot.value
 select(selected.value);refresh();window.addEventListener('keydown',keydown)
})
onUnmounted(()=>{stopped=true;if(timer)clearTimeout(timer);window.removeEventListener('keydown',keydown);delete (window as any).__deskfuseNavigate;delete (window as any).__deskfuseSnapshot})
</script>

<template>
  <component v-if="fullBleed" :is="component" />
  <div v-else class="app-shell">
  <aside class="sidebar">
   <div class="brand title-drag"><span class="brand-icon" role="img" aria-label="DeskFuse"/><strong>DeskFuse</strong></div>
   <nav aria-label="主导航">
    <template v-for="section in (needsFirstDevice?['配置','基础','高级']:['基础','高级','配置'])" :key="section">
     <div class="nav-section">{{section}}</div>
     <button v-for="page in pages.filter(p=>p.section===section)" :key="page.id" class="nav-row" :class="{selected:selected===page.id}" :aria-current="selected===page.id?'page':undefined" @click="select(page.id)">
      <component :is="page.icon" :size="16"/><span>{{page.title}}</span><span v-if="page.beta" class="badge beta">Beta</span><small v-if="page.id==='devices'" class="device-count">{{paired.length?`${online} 台在线`:'未添加'}}</small>
     </button>
    </template>
   </nav>
   <footer class="sidebar-footer">
    <button class="nav-row" :class="{selected:selected==='settings'}" @click="select('settings')"><Settings :size="16"/><span>设置</span></button>
    <div v-if="selected==='settings'" class="settings-nav"><button v-for="[id,title] in categories.filter(c=>c[0]!=='workflows'||snapshot?.app?.workflows_enabled!==false)" :key="id" :class="{active:category===id}" @click="category=id">{{title}}</button></div>
    <div class="health-row"><button class="health-button" @click="health=!health" title="查看运行状态与需要处理的问题"><i :class="healthTone"/><span>{{healthText}}</span><ChevronRight v-if="healthIssues.length" :size="12"/></button><button class="help-button" aria-label="帮助与反馈" title="帮助与反馈" @click="help=!help"><CircleHelp :size="18"/></button></div>
   </footer>
  </aside>
  <main class="workspace">
   <header class="topbar title-drag"><span>{{current?.section||'DeskFuse'}}</span><ChevronRight :size="12"/><strong>{{selected==='settings'?categories.find(c=>c[0]===category)?.[1]:current?.title||'DeskFuse'}}</strong><span v-if="current?.beta" class="badge beta">Beta</span></header>
   <div v-if="error" class="service-notice notice" role="status"><span>{{error}}</span><button class="button" @click="retry"><RefreshCw :size="13"/>重试</button><button class="icon-button" aria-label="反馈问题" @click="help=true"><CircleHelp :size="15"/></button></div>
   <div class="page-host"><component v-if="component" :is="component" :snapshot="snapshot" :category="category" @navigate="select" @refresh="retry"/><div v-else class="empty-state"><h2>{{pageError||'正在加载'}}</h2><button v-if="pageError" class="button" @click="select(selected)">重试</button></div></div>
  </main>
  <div v-if="help" class="help-popover card"><button class="menu-item" @click="report">反馈问题…</button><button class="menu-item" @click="copyEnvironment">复制版本与环境信息</button><div class="menu-divider"/><button class="menu-item" @click="openURL('https://github.com/GodD6366/DeskFuse/issues')">已报告的问题</button><button class="menu-item" @click="openURL('https://github.com/GodD6366/DeskFuse/releases')">发行说明</button><button class="icon-button dismiss" aria-label="关闭帮助" @click="help=false"><X :size="14"/></button></div>
  <div v-if="health" class="health-popover card"><h2>运行状态</h2><p v-if="!healthIssues.length">{{'当前没有需要处理的问题。'}}</p><div v-for="issue in healthIssues" :key="issue.id" class="issue"><div class="issue-text"><strong>{{issue.title}}</strong><p v-if="issue.detail">{{issue.detail}}</p></div><button v-if="issue.action||issue.page" class="button" @click="resolveIssue(issue)">{{issue.action==='devices.refresh'?'刷新':issue.action==='support.report'?'反馈问题':issue.action==='permissions.open'?'前往授权':issue.page==='devices'?'添加电脑':'查看'}}</button></div><button class="icon-button dismiss" aria-label="关闭运行状态" @click="health=false"><X :size="14"/></button></div>
 </div>
</template>

<style scoped>
.app-shell{height:100vh;min-height:560px;display:flex;overflow:hidden}.sidebar{width:var(--sidebar-width);flex:0 0 var(--sidebar-width);display:flex;flex-direction:column;border-right:.5px solid var(--line);background:var(--canvas)}.title-drag{--app-region:drag;-webkit-app-region:drag;user-select:none}.brand{height:52px;flex:0 0 52px;display:flex;align-items:center;gap:8px;padding-left:92px;font-size:15px;margin-bottom:4px}.brand-icon{width:22px;height:22px;flex-shrink:0;background:url(/deskfuse-icon-light.png) center/contain no-repeat;border-radius:5px}@media(prefers-color-scheme:dark){.brand-icon{background-image:url(/deskfuse-icon.png)}}.brand strong{font-weight:600}nav{overflow:auto;flex:1;padding:0 10px 4px}.nav-section{padding:12px 10px 4px;font-size:11px;color:var(--secondary)}.nav-row{width:100%;display:flex;align-items:center;gap:10px;height:32px;padding:0 12px;border-radius:8px;text-align:left;position:relative;margin:2px 0}.nav-row:hover{background:var(--control)}.nav-row.selected{background:var(--surface);box-shadow:0 1px 2px #00000012;font-weight:500}.nav-row.selected:before{content:'';position:absolute;left:-6px;top:5px;bottom:5px;width:3px;background:var(--accent);border-radius:2px}.nav-row.selected>svg{color:var(--accent)}.nav-row .badge{margin-left:auto;padding:1px 5px;font-weight:400}.device-count{margin-left:auto;white-space:nowrap}.sidebar-footer{padding:8px 10px 0;border-top:.5px solid var(--line)}.settings-nav{border-left:1px solid var(--line);margin:0 0 4px 10px;padding-left:10px;display:flex;flex-direction:column}.settings-nav button{text-align:left;height:28px;border-radius:6px;padding:0 12px;color:var(--secondary)}.settings-nav button:hover,.settings-nav button.active{background:var(--control);color:var(--text)}.health-row{display:flex;align-items:center;padding:10px 4px 10px 12px;gap:4px}.health-button{display:flex;align-items:center;gap:8px;flex:1;font-size:12px;text-align:left;padding:0}.health-button i{width:7px;height:7px;border-radius:50%;background:var(--success)}.health-button i.warning{background:var(--warning)}.health-button i.accent{background:var(--accent)}.health-button i.danger{background:var(--danger)}.help-button{width:24px;height:24px;color:var(--secondary);padding:3px}.workspace{display:flex;flex-direction:column;flex:1;min-width:0;background:var(--canvas)}.topbar{height:52px;flex:0 0 52px;display:flex;align-items:center;gap:10px;padding:0 24px;color:var(--secondary);font-size:13px}.topbar strong{color:var(--text);font-weight:600}.page-host{flex:1;min-height:0;overflow:hidden;position:relative;display:flex;flex-direction:column}.service-notice{margin:0 24px 12px;flex-shrink:0;font-size:12px}.service-notice>span{flex:1}.help-popover,.health-popover{position:fixed;bottom:56px;left:16px;width:300px;z-index:50;box-shadow:0 8px 30px #0002;display:flex;flex-direction:column;gap:12px}.health-popover{bottom:56px;left:24px}.dismiss{position:absolute;right:8px;top:8px}.help-popover{gap:2px;padding:8px}.menu-item{display:flex;align-items:center;height:30px;padding:0 10px;border-radius:6px;text-align:left;font-size:13px}.menu-item:hover{background:var(--control)}.menu-divider{height:.5px;background:var(--line);margin:4px 2px}.health-popover{gap:6px;max-height:60vh;overflow:auto}.issue{display:flex;align-items:flex-start;gap:10px;padding:8px 0}.issue+.issue{border-top:.5px solid var(--line)}.issue-text{flex:1;min-width:0}.issue-text strong{font-size:13px;font-weight:600}.issue-text p{font-size:12px;color:var(--secondary);margin-top:2px}.issue .button{flex-shrink:0;min-height:26px;padding:0 10px;font-size:12px}
</style>

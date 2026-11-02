import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
// WebView2 的 UA 带 Windows NT；共享界面用它区分标题栏留白和仅 macOS 可用的设置。
if (/Windows NT/i.test(navigator.userAgent)) document.documentElement.dataset.platform = 'windows'
createApp(App).mount('#app')

import { createApp } from 'vue'
import App from './App.vue'
import { revealDirective } from './directives/reveal'
import './assets/main.css'

const app = createApp(App)

app.directive('reveal', revealDirective)

app.mount('#app')

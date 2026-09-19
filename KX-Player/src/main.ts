import { createApp } from 'vue'
import { createPinia } from 'pinia'
import '@fontsource/inter/400.css'
import '@fontsource/inter/500.css'
import '@fontsource/inter/600.css'
import '@fontsource/noto-sans-sc/400.css'
import '@fontsource/noto-sans-sc/500.css'
import '@fontsource/noto-sans-sc/700.css'
import './styles/tokens.css'
import './styles/theme.css'
import './styles/base.css'
import './styles/motion.css'
import App from './App.vue'

const app = createApp(App)
app.use(createPinia())
app.config.errorHandler = (err) => {
  console.error('[KX-Player]', err, (err as Error)?.stack ?? '')
}
window.addEventListener('unhandledrejection', (e) => {
  console.error('[KX-Player unhandledrejection]', e.reason, (e.reason as Error)?.stack ?? '')
})
app.mount('#app')

import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { getCurrentWindow } from '@tauri-apps/api/window'
import './styles/tokens.css'
import './styles/theme.css'
import './styles/base.css'
import './styles/motion.css'

function installErrorHandlers(app: ReturnType<typeof createApp>): void {
  app.config.errorHandler = (err) => {
    console.error('[KX-Player]', err, (err as Error)?.stack ?? '')
  }
  window.addEventListener('unhandledrejection', (e) => {
    console.error('[KX-Player unhandledrejection]', e.reason, (e.reason as Error)?.stack ?? '')
  })
}

// 独立悬浮窗（pip）是独立系统窗口（label="pip"），只挂载 PipRoot 迷你应用，
// 不初始化主应用的曲库/设置/搜索等逻辑。用窗口 label 而非 URL hash 判断，
// 避免 WebviewUrl::App 拼接时把 "#/pip" 编码成路径的一部分。
let isPip = false
try {
  isPip = getCurrentWindow().label === 'pip'
} catch {
  isPip = false
}

if (isPip) {
  const { default: PipRoot } = await import('@/components/PipRoot.vue')
  const app = createApp(PipRoot)
  installErrorHandlers(app)
  app.mount('#app')
} else {
  const { default: App } = await import('./App.vue')
  const app = createApp(App)
  app.use(createPinia())
  installErrorHandlers(app)
  app.mount('#app')

  // 首帧渲染后移除启动占位（见 index.html 的内联骨架）
  requestAnimationFrame(() => {
    const boot = document.getElementById('boot')
    if (boot) {
      boot.style.transition = 'opacity 160ms ease'
      boot.style.opacity = '0'
      setTimeout(() => boot.remove(), 200)
    }
  })
}

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

/** 移除 index.html 内联的启动骨架（无论 pip/main 都要移除）。
 *  直接同步移除，不用 rAF/定时器：未聚焦或被遮挡的窗口（后台创建的悬浮窗就是这种）
 *  会被 WebView2 暂停 rAF、并把定时器节流到分钟级，骨架就会一直盖在应用上面转圈。 */
function removeBoot(): void {
  document.getElementById('boot')?.remove()
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

try {
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
  }
} finally {
  // 入口模块加载/挂载失败时也必须撤掉骨架，否则窗口永远停在「正在启动」
  removeBoot()
}

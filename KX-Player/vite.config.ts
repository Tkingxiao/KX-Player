import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'node:path'

// Tauri 前端构建配置（后端在 src-tauri/，由 tauri-cli 编排）
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },
  // Vite 开发服务器选项（tauri dev 调用 dev:vite）
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
    // 预热入口与首屏组件：启动时先在后台完成转换，
    // 避免首个页面请求逐个 transform 造成的长时间白屏。
    warmup: {
      clientFiles: [
        './src/main.ts',
        './src/App.vue',
        './src/features/shell/Sidebar.vue',
        './src/features/shell/PlayerBar.vue',
        './src/features/shell/TitleBar.vue',
        './src/features/library/ContentArea.vue',
        './src/features/library/SmartView.vue',
      ],
    },
  },
  // 依赖预打包策略（直接决定 dev 首次加载的白屏时长）：
  // · entries：只扫描真正的入口 index.html。默认会抓取项目内所有 *.html，
  //   把 src-tauri/target/.../tauri-codegen-assets/ 里内嵌的生产包（数百个字体+JS）
  //   也当入口全量扫描，冷启动时首次页面请求被阻塞长达 40~60s（表现为长时间白屏）。
  // · include：首屏必需的基础依赖。显式声明可避免 Vite 在页面加载途中
  //   「发现新依赖 → 重新优化 → 整页 reload」（表现为所有资源同时卡住十几秒）。
  // · exclude：只在搜索时才动态 import 的重库（chinese-conv / pinyin-pro，合计≈1.5MB）。
  //   实测把它们放进预打包会让首次页面请求阻塞 40s+（esbuild 要现打这两个包）；
  //   排除后 Vite 按需直接服务 ESM 源码，首屏不受影响，搜索时再加载。
  optimizeDeps: {
    entries: ['./index.html'],
    // 不跑全量依赖爬取（本机实测爬取阶段要 15~58s，期间首个页面请求被阻塞、窗口白屏）。
    // 本项目所有静态裸导入都已列在 include 中（vue/pinia/@tauri-apps/api/*），
    // chinese-conv / pinyin-pro 仅动态导入且已 exclude，因此无需发现阶段。
    noDiscovery: true,
    include: [
      'vue',
      'pinia',
      '@vueuse/core',
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
    ],
    exclude: [
      'chinese-conv',
      'pinyin-pro',
    ],
  },
  // 内联空 postcss 配置，跳过 postcss-load-config 的磁盘搜索。
  // 本机实测该搜索在首次 CSS 变换时同步阻塞事件循环 40~50s（页面白屏根因）；
  // 项目无 postcss 配置、CSS 均为原生语法，内联空配置行为等价且无副作用。
  // 若日后引入 postcss 插件（tailwind/autoprefixer 等），需改回配置文件形式。
  css: {
    postcss: {},
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    // WebView2 目标支持较新语法，minify 保持 esbuild 默认
    target: 'chrome120',
    minify: 'esbuild',
    sourcemap: false,
    // 拆分体积大的可选功能，避免首屏加载全部界面代码
    rollupOptions: {
      output: {
        manualChunks(id: string) {
          if (id.includes('node_modules/chinese-conv') || id.includes('node_modules/pinyin-pro')) {
            return 'vendor-search'
          }
          if (id.includes('node_modules/vue') || id.includes('node_modules/pinia')) {
            return 'vendor-vue'
          }
          return undefined
        },
      },
    },
  },
})

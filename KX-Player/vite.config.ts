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
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    // WebView2 目标支持较新语法，minify 保持 esbuild 默认
    target: 'chrome120',
    minify: 'esbuild',
    sourcemap: false,
  },
})

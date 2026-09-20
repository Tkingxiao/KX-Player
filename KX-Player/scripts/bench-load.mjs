import { readFileSync } from 'node:fs'
import { globSync } from 'node:fs'

// 1) 读取 70 个不同的 src 文件（模拟 Vite 首次加载整个依赖图）
const files = globSync('src/**/*.{vue,ts,css}').slice(0, 70)
let t = Date.now()
for (const f of files) readFileSync(f)
console.log(`read ${files.length} distinct src files: ${Date.now() - t}ms (${((Date.now() - t) / files.length).toFixed(1)}ms/file)`)

// 2) 读取 70 个不同的 node_modules 文件
const nm = globSync('node_modules/@fontsource/inter/files/*.woff2').slice(0, 70)
t = Date.now()
for (const f of nm) readFileSync(f)
console.log(`read ${nm.length} distinct font files: ${Date.now() - t}ms (${((Date.now() - t) / nm.length).toFixed(1)}ms/file)`)

// 3) fs.watch 开销：打开 70 个文件监视句柄
import { watch } from 'node:fs'
const handles = []
t = Date.now()
for (const f of files.slice(0, 70)) {
  try { handles.push(watch(f, () => {})) } catch { /* windows can't watch some */ }
}
console.log(`fs.watch ${handles.length} files: ${Date.now() - t}ms`)
for (const h of handles) h.close()

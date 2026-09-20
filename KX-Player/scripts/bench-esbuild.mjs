import { buildSync } from 'esbuild'
import { readFileSync } from 'node:fs'

const t0 = Date.now()
buildSync({
  stdin: { contents: 'import * as x from "vue"; console.log(x)', resolveDir: process.cwd() },
  bundle: true,
  write: false,
  logLevel: 'silent',
})
console.log('esbuild bundle vue: ' + (Date.now() - t0) + 'ms')

const t1 = Date.now()
for (let i = 0; i < 50; i++) {
  readFileSync('node_modules/@fontsource/noto-sans-sc/400.css')
}
console.log('50 css reads: ' + (Date.now() - t1) + 'ms')

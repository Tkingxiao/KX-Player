import { realpathSync, existsSync, readFileSync } from 'node:fs'

const font = 'node_modules/@fontsource/noto-sans-sc/files/noto-sans-sc-85-600-normal.woff'

let t = Date.now()
for (let i = 0; i < 200; i++) existsSync(font)
console.log('200 existsSync: ' + (Date.now() - t) + 'ms')

t = Date.now()
for (let i = 0; i < 200; i++) realpathSync(font)
console.log('200 realpathSync: ' + (Date.now() - t) + 'ms')

t = Date.now()
for (let i = 0; i < 200; i++) readFileSync(font)
console.log('200 readFileSync: ' + (Date.now() - t) + 'ms')

// case-sensitivity check: nonexistent path variants (Windows resolver checks multiple candidates)
t = Date.now()
for (let i = 0; i < 200; i++) existsSync('node_modules/@fontsource/noto-sans-sc/files/NOTEXIST.woff')
console.log('200 existsSync missing: ' + (Date.now() - t) + 'ms')

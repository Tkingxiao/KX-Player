import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

const src = 'src'
const bare = new Set()
const walk = (dir) => {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e)
    if (statSync(p).isDirectory()) walk(p)
    else if (/\.(ts|vue)$/.test(e)) {
      const content = readFileSync(p, 'utf-8')
      for (const m of content.matchAll(/from\s+['"]([^'"]+)['"]/g)) {
        const spec = m[1]
        if (!spec.startsWith('.') && !spec.startsWith('@/') && !spec.startsWith('@')) bare.add(spec)
        else if (spec.startsWith('@')) {
          // scoped package or alias: @tauri-apps/... is a package; @/ is alias
          if (spec.startsWith('@/')) continue
          bare.add(spec)
        }
      }
    }
  }
}
walk(src)
console.log('bare imports:', [...bare])

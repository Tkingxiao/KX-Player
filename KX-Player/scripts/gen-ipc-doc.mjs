#!/usr/bin/env node
/**
 * IPC 契约校验 + 文档生成（02 §6.4）。
 *
 * 无参数        → 生成 docs/IPC.md
 * --check       → 只校验不写文件，不一致即 exit 1（`npm run check:ipc`）
 * --print-rust  → 把解析到的 Rust 命令倒成 JSON（人工写契约时的对照表，非 CI 路径）
 *
 * 四个来源对账单：Rust `#[tauri::command]`、`lib.rs` 的 `generate_handler!` 注册表、
 * `src/contracts/{commands,events}.ts`、`src/bridge/ipc.ts` 的实际调用。
 * 每一条都是今天就可能踩且只能靠机器发现的：命令定义了没注册、契约漏一行、
 * 前端参数键名与 Rust 形参的 camelCase 归一化结果不符（Tauri 直接报 invalid args）、
 * 事件名两边不一致（拼错不报错，只是静默失灵）。
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const TAURI = path.join(ROOT, 'src-tauri', 'src')
const FRONT = path.join(ROOT, 'src')
const DOC = path.join(ROOT, 'docs', 'IPC.md')

const MODE = process.argv.slice(2)
const CHECK = MODE.includes('--check')
const PRINT_RUST = MODE.includes('--print-rust')

const read = (f) => readFileSync(f, 'utf8').replace(/\r\n/g, '\n')
const rel = (f) => path.relative(ROOT, f).replace(/\\/g, '/')

/** Tauri v2 默认把 Rust 的 snake_case 形参名归一化成 camelCase 的 JS 键名。 */
const toCamel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase())

function* walk(dir, re) {
  for (const e of readdirSync(dir)) {
    const p = path.join(dir, e)
    if (statSync(p).isDirectory()) yield* walk(p, re)
    else if (re.test(e)) yield p
  }
}

/** 按顶层逗号切列表（`Vec<(String, String)>` 里的逗号不算）。 */
function splitTop(s) {
  const out = []
  let depth = 0
  let cur = ''
  for (const ch of s) {
    if ('<([{'.includes(ch)) depth++
    if ('>)]}'.includes(ch)) depth--
    if (ch === ',' && depth === 0) {
      out.push(cur)
      cur = ''
    } else cur += ch
  }
  if (cur.trim()) out.push(cur)
  return out.map((x) => x.trim()).filter(Boolean)
}

/** 从 openAt（指向 `{`/`(`/`[`）找到配对的闭合括号下标；字符串里的括号不算。 */
function matchClose(src, openAt, open, close) {
  let depth = 0
  for (let i = openAt; i < src.length; i++) {
    const ch = src[i]
    if (ch === '"' || ch === '`') {
      const end = src.indexOf(ch, i + 1)
      if (end < 0) return -1
      i = end
    } else if (ch === open) depth++
    else if (ch === close && --depth === 0) return i
  }
  return -1
}

// ── Rust 侧 ────────────────────────────────────────────────

/** 命令所属模块：`commands/library.rs → library`，`player/mod.rs → player`。 */
function moduleOf(file) {
  const parts = rel(file).replace(/^src-tauri\/src\//, '').replace(/\.rs$/, '').split('/')
  if (parts.join('/') === 'lib') return 'app'
  if (parts[0] === 'commands') return parts.slice(1).join('::') || 'app'
  return parts[parts.length - 1]
}

/** Tauri 自己注入的参数，不是前端要传的。 */
const INJECTED = new Set(['app', 'window', 'webview', 'state', 'manager', 'resolver', 'uri_scheme', 'window_label'])

/**
 * 抓 `#[tauri::command]` 之后那个 fn。签名常跨行，先按括号配平往下吃，
 * 再按**深度**找参数表的右括号——`-> Result<(), String>` 里也有括号，
 * 用 lastIndexOf(')') 会切出 `i64) -> Result<(` 这种一地碎片。
 */
function rustCommands() {
  const out = []
  for (const file of walk(TAURI, /\.rs$/)) {
    const lines = read(file).split('\n')
    for (let i = 0; i < lines.length; i++) {
      if (!/^\s*#\[tauri::command\b/.test(lines[i])) continue
      let j = i + 1
      while (j < lines.length && !/^\s*((pub|async|unsafe|extern|const)\s+)*fn\s/.test(lines[j])) j++
      if (j >= lines.length) continue
      let sig = lines[j]
      let k = j
      while ((sig.match(/\(/g)?.length ?? 0) > (sig.match(/\)/g)?.length ?? 0) && k + 1 < lines.length) sig += '\n' + lines[++k]
      const name = /fn\s+([a-z0-9_]+)/.exec(sig)?.[1]
      const open = sig.indexOf('(')
      if (!name || open < 0) continue
      const close = matchClose(sig, open, '(', ')')
      if (close < 0) continue
      const params = splitTop(sig.slice(open + 1, close))
        .filter((p) => !INJECTED.has(p.split(':')[0].trim()))
        .map((p) => {
          const [rust, type] = p.split(/:(.+)/).map((s) => s.trim())
          return { rust, ts: toCamel(rust), type: type ?? '', optional: /^Option</.test(type ?? '') }
        })
      out.push({
        module: moduleOf(file), file: rel(file), line: j + 1, name, params,
        ret: /->\s*([\s\S]*?)(?:\{\s*$|$)/.exec(sig.slice(close + 1))?.[1]?.trim() ?? '()',
      })
      i = k
    }
  }
  return out
}

/** lib.rs 的 generate_handler 列表 = 真正注册进运行时的命令集合。 */
function registered() {
  const src = read(path.join(TAURI, 'lib.rs'))
  const block = /invoke_handler\(\s*tauri::generate_handler!\[([\s\S]*?)\]\s*\)/.exec(src)?.[1] ?? ''
  return block
    .replace(/\/\/[^\n]*/g, '')
    .split(',')
    .map((s) => s.trim().split('::').pop())
    .filter((s) => /^[a-z0-9_]+$/.test(s))
}

/** Rust 侧 `emit("name", payload)`；`emit_to(label, "name", …)` 一并收。 */
function rustEmits() {
  const map = new Map()
  for (const file of walk(TAURI, /\.rs$/)) {
    const src = read(file)
    for (const m of src.matchAll(/\.emit(?:_to|_all)?\s*(?:::<[^>]*>)?\s*\(\s*(?:[\w.&]+\.label\(\)\s*,\s*)?"([^"]+)"/g)) {
      if (!map.has(m[1])) map.set(m[1], [])
      map.get(m[1]).push(`${rel(file)}:${src.slice(0, m.index).split('\n').length}`)
    }
  }
  return map
}

// ── 契约侧 ─────────────────────────────────────────────────

function commandSpecs() {
  const src = read(path.join(FRONT, 'contracts', 'commands.ts'))
  const body = /export interface CommandSpecs \{([\s\S]*?)\n\}/.exec(src)?.[1] ?? ''
  const specs = new Map()
  for (const m of body.matchAll(/^\s*([a-z0-9_]+): \{ args: \{([^}]*)\}, returns: (.*?) \},?$/gm)) {
    specs.set(m[1], { args: [...m[2].matchAll(/([A-Za-z0-9_]+):/g)].map((k) => k[1]), returns: m[3].trim() })
  }
  if (!specs.size) throw new Error('contracts/commands.ts 里没解析到 CommandSpecs 条目，检查格式')
  return specs
}

function eventSpecs() {
  const src = read(path.join(FRONT, 'contracts', 'events.ts'))
  const body = /export interface EventSpecs \{([\s\S]*?)\n\}/.exec(src)?.[1] ?? ''
  const map = new Map()
  for (const m of body.matchAll(/^\s*'([^']+)': (.*?)$/gm)) map.set(m[1], m[2].replace(/,$/, '').trim())
  if (!map.size) throw new Error('contracts/events.ts 里没解析到 EventSpecs 条目，检查格式')
  return map
}

// ── 前端侧 ─────────────────────────────────────────────────

/** `invoke('name', { … })` → `{ name, args: [顶层键], line }`。泛型里嵌什么都无所谓。 */
function frontendCalls() {
  const out = []
  for (const file of walk(FRONT, /\.(ts|vue)$/)) {
    if (file.includes(`${path.sep}contracts${path.sep}`)) continue
    const src = read(file)
    for (const m of src.matchAll(/\binvoke\s*(?:<[\s\S]{0,200}?>\s*)?\(\s*(['"])([a-z0-9_]+)\1/g)) {
      const after = m.index + m[0].length
      const end = matchClose(src, m.index + m[0].indexOf('('), '(', ')')
      const brace = src.indexOf('{', after)
      const args = []
      // 第二个实参可能没有（无参命令），也可能对象里套对象——只取顶层键
      if (brace > 0 && (end < 0 || brace < end)) {
        const close = matchClose(src, brace, '{', '}')
        if (close > brace) for (const k of splitTop(src.slice(brace + 1, close))) {
          // `{ sec }` 简写与 `{ sec: v }` 都是传 sec
          const key = /^([A-Za-z0-9_]+)(?:\s*:|$)/.exec(k)?.[1]
          if (key) args.push(key)
        }
      }
      out.push({ name: m[2], args, where: `${rel(file)}:${src.slice(0, m.index).split('\n').length}` })
    }
  }
  return out
}

/** `onEvent('name'…)` / `listen('name'…)` / `once('name'…)`：泛型可能很长，只认紧跟其后的第一个字面量。 */
function frontendListeners() {
  const map = new Map()
  for (const file of walk(FRONT, /\.(ts|vue)$/)) {
    const src = read(file)
    for (const m of src.matchAll(/\b(?:listen|once|onEvent)\s*(?:<[\s\S]{0,300}?>)?\s*\(\s*/g)) {
      const q = /(['"])([a-z]+:[A-Za-z][A-Za-z0-9]*)\1/.exec(src.slice(m.index + m[0].length, m.index + m[0].length + 120))
      if (!q) continue
      if (!map.has(q[2])) map.set(q[2], [])
      const at = `${rel(file)}:${src.slice(0, m.index).split('\n').length}`
      if (!map.get(q[2]).includes(at)) map.get(q[2]).push(at)
    }
  }
  return map
}

// ── 校验 ───────────────────────────────────────────────────

const cmds = rustCommands()
const reg = new Set(registered())
const spec = commandSpecs()
const evtSpec = eventSpecs()
const emits = rustEmits()
const calls = frontendCalls()
const listens = frontendListeners()
const byName = new Map(cmds.map((c) => [c.name, c]))

if (PRINT_RUST) {
  console.log(JSON.stringify(cmds, null, 1))
  process.exit(0)
}

const problems = []
for (const c of cmds) {
  if (!reg.has(c.name)) problems.push(`Rust 命令没注册进 generate_handler，前端永远调不到：${c.name}（${c.file}:${c.line}）`)
  if (!spec.has(c.name)) problems.push(`契约缺失：${c.name} 有 Rust 实现但不在 CommandSpecs 里`)
}
for (const n of spec.keys()) {
  if (!byName.has(n)) problems.push(`契约里的幽灵命令：${n} 在 Rust 侧不存在`)
}
const invoked = new Set(calls.map((c) => c.name))
/** 已实现但前端尚未接入的命令：留名必须有理由，新增的一律视为缺陷。 */
const UNCALLED_OK = {
  taxonomy_category_track_ids: '筛选下沉到 SQL（B1）后由分类视图调用',
  app_force_quit: '退出流程在 Rust 侧内联（lib.rs 托盘/关闭路径），命令保留待崩溃恢复入口',
}
for (const n of spec.keys()) {
  if (!invoked.has(n) && !UNCALLED_OK[n]) problems.push(`命令没有调用方：${n} 在 src/ 里没有任何 invoke`)
}

for (const call of calls) {
  const c = byName.get(call.name)
  if (!c) {
    problems.push(`前端调用了不存在的命令：${call.name}（${call.where}）`)
    continue
  }
  const expect = new Set(c.params.map((p) => p.ts))
  for (const k of call.args) {
    if (!expect.has(k)) {
      problems.push(`参数键名不匹配：${call.name}（${call.where}）传了 ${k}，Rust 形参是 ${[...expect].join('/') || '(无)'}`)
    }
  }
  for (const p of c.params) {
    if (!p.optional && !call.args.includes(p.ts)) {
      problems.push(`参数缺失：${call.name}（${call.where}）没传 ${p.ts}，Rust 侧 ${p.rust}: ${p.type} 是必填`)
    }
  }
}

for (const [name, where] of emits) {
  if (!evtSpec.has(name)) problems.push(`事件未登记契约：Rust 发射 ${name}（${where}）但 EventSpecs 里没有`)
}
for (const name of evtSpec.keys()) {
  if (!emits.has(name)) problems.push(`契约里的幽灵事件：${name} 没有任何 Rust emit`)
}
for (const [name, where] of listens) {
  if (!evtSpec.has(name)) problems.push(`前端监听了未登记的事件：${name}（${where}）`)
}

// ── 文档 ───────────────────────────────────────────────────

const TS_PRIM = {
  String: 'string', '&str': 'string', str: 'string', i64: 'number', u64: 'number', i32: 'number',
  u32: 'number', f64: 'number', f32: 'number', bool: 'boolean', '()': 'null', Value: 'unknown',
  'serde_json::Value': 'unknown',
}

/** Rust 类型 → TS 类型，仅用于文档列；契约里的 `returns` 是人工维护的权威版本。 */
function rustToTs(t) {
  t = t.trim()
  if (TS_PRIM[t]) return TS_PRIM[t]
  const r = /^([\w:]+)<([\s\S]*)>$/.exec(t)
  if (r) {
    const inner = splitTop(r[2])
    // 按路径末段分派：`crate::error::IpcResult<T>` 与 `IpcResult<T>` 是同一个类型别名
    switch (r[1].split('::').pop()) {
      case 'Vec': {
        const el = rustToTs(r[2])
        return r[2].includes(',') && inner.length > 1 ? `(${el})[]` : `${el}[]`
      }
      case 'Option':
        return `${rustToTs(r[2])} | null`
      case 'Result':
      case 'IpcResult':
        return rustToTs(inner[0])
      case 'HashMap': {
        const [k, v] = inner
        return `Record<${rustToTs(k)}, ${rustToTs(v)}>`
      }
      default:
        break
    }
  }
  if (/^\(.*\)$/.test(t)) return `[${splitTop(t.slice(1, -1)).map(rustToTs).join(', ')}]`
  return t.replace(/^crate::([a-z_]+)::/, '').replace(/Dto$/, '')
}

/** 表格单元格：GFM 用 `|` 分列，而代码段里没法转义它，联合类型改用 U+2223（渲染同为竖线）。 */
const cell = (s) => '`' + String(s).replace(/\|/g, '\u2223') + '`'

function docBody() {
  const groups = new Map()
  for (const c of cmds) {
    if (!groups.has(c.module)) groups.set(c.module, [])
    groups.get(c.module).push(c)
  }
  const lines = [
    '# IPC 契约（命令与事件）',
    '',
    '> 由 `npm run gen:ipc`（`scripts/gen-ipc-doc.mjs`）生成，请勿手改。',
    '> 一致性由 `npm run check:ipc` 把关：Rust `#[tauri::command]` · `lib.rs` 注册表 ·',
    '> `src/contracts/commands.ts` · `src/bridge/ipc.ts` 的参数键名，四方对账（`02 §6.4`）。',
    '',
    '类型映射：`String→string`、`i64/u64/f64→number`、`bool→boolean`、`Vec<T>→T[]`、',
    '`Option<T>→T | null`（JSON 是 `null`，不是 `undefined`）、`HashMap<K,V>→Record<K,V>`、',
    '`(A,B)→[A, B]`、`serde_json::Value→unknown`。',
    '',
    '> 参数键名：Rust 的 `snake_case` 形参在 JS 侧是 `camelCase`（Tauri v2 默认归一化）。',
    '> 入参若是结构体，字段名以该结构体的 `#[serde]` 属性为准。',
    '',
    '> **返回列是 resolve 值，即 `IpcResult<T>`/`Result<T, _>` 的 `Ok` 分支**。',
    '> 失败经 Tauri 走 reject，载荷是 `IpcError { code, message, detail }`（`02 §6.1`），',
    '> `bridge/ipc.ts` 统一归一化为 `AppError`；`message` 面向用户，`detail` 只进日志。',
    '',
  ]
  for (const [mod, list] of [...groups.entries()].sort()) {
    lines.push(`## ${mod}`, '')
    lines.push('| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |', '|---|---|---|---|')
    for (const c of [...list].sort((a, b) => a.name.localeCompare(b.name))) {
      const args = c.params.map((p) => cell(`${p.ts}: ${p.type}`)).join(', ') || '—'
      // 只记文件不记行号：行号会被任意上方编辑推动，让 check:ipc 在没有契约变化时报错。
      lines.push(`| ${cell(c.name)} | ${args} | ${cell(rustToTs(c.ret))} | ${c.file} |`)
    }
    lines.push('')
  }
  lines.push('## 事件', '')
  lines.push('载荷类型的权威定义在 `src/contracts/events.ts`；下表核对两侧名字与位置。', '')
  lines.push('| 事件 | 载荷 | 发射（Rust） | 监听（前端） |', '|---|---|---|---|')
  for (const name of [...evtSpec.keys()].sort()) {
    const from = [...new Set((emits.get(name) ?? ['—']).map((s) => s.split(':')[0]))].join('<br>')
    // 同命令表：只记文件，行号会在没有契约变化时把 check:ipc 顶红
    const to = [...new Set((listens.get(name) ?? ['—']).map((s) => s.split(':')[0]))].join('<br>')
    lines.push(`| ${cell(name)} | ${cell(evtSpec.get(name))} | ${from} | ${to} |`)
  }
  lines.push('')
  return lines.join('\n')
}

if (CHECK) {
  if (!existsSync(DOC)) problems.push('docs/IPC.md 不存在，先跑 npm run gen:ipc')
  else if (read(DOC) !== docBody()) problems.push('docs/IPC.md 与代码不同步，重跑 npm run gen:ipc')
  const uniq = [...new Set(problems)]
  if (uniq.length) {
    console.error(`check:ipc 失败（${uniq.length} 项）：`)
    for (const p of uniq) console.error(`  - ${p}`)
    process.exit(1)
  }
  console.log(`check:ipc 通过：${cmds.length} 条命令 / ${evtSpec.size} 个事件 / ${calls.length} 处 invoke / ${listens.size} 处监听，docs/IPC.md 已同步`)
  process.exit(0)
}

mkdirSync(path.dirname(DOC), { recursive: true })
writeFileSync(DOC, docBody(), 'utf8')
console.log(`docs/IPC.md 已生成：${cmds.length} 条命令、${cmds.length ? new Set(cmds.map((c) => c.module)).size : 0} 个模块、${evtSpec.size} 个事件`)

#!/usr/bin/env node
/**
 * 列名对账：src/contracts/columns.ts（单一事实源）↔ DDL ↔ Rust SQL ↔ src/contracts/dto.ts
 *
 * 用法：node scripts/check-columns.mjs        （= npm run check:columns，已串进 npm run check）
 *       node scripts/check-columns.mjs --list  （额外逐条打印「已放行 / 已跳过」的明细）
 *
 * 全绿：打印统计并 exit 0；有任何一处不一致：逐条打印并 exit 1。
 *
 * 四项校验（判据定义见 src/contracts/columns.ts 文件头）：
 *   (a) DB_COLUMNS 登记的列名必须真实存在。schema = migrations/*.sql 的 CREATE TABLE
 *       ∪ Rust 生产代码里的 CREATE (TEMP) TABLE ∪ ensure_column / ALTER TABLE ADD COLUMN；
 *   (b) src-tauri/src/db/*.rs 里每个列引用位置都必须在表里（含 t.id 这种带别名形式）；
 *       位置：SELECT 列表 · INSERT 列表 · ON CONFLICT(...) · SET 左值 · ORDER BY / GROUP BY 项 ·
 *       比较与 IN/LIKE/IS 左侧；解析不出来的位置逐条列进「显式跳过」；
 *   (c) dto.ts 里 DTO_TABLES 覆盖的接口，字段必须命中该表的列（camelCase → snake_case 自动
 *       归一化），或在 EXCEPTIONS.DTO_FIELDS 里；dto.ts 的每个接口都必须被分类过；
 *   (d) 反向：登记过的表，DDL 每一列要么在 DB_COLUMNS，要么在 EXCEPTIONS.DB_COLUMNS。
 *
 * 解析原则：正则 + 括号配平，笨但可读。**不静默跳过** —— 解析不了的输入、被例外清单放行的
 * 名字、用不上的例外条目，全都在输出里点名；例外清单里已经不需要的条目同样判失败
 * （免得清单越写越长、把真问题盖过去）。
 *
 * 已知边界（有意为之，不是漏报）：
 *   · 列校验覆盖的位置：SELECT 列表、INSERT 列清单、ON CONFLICT(...)、SET 左值、
 *     ORDER BY / GROUP BY 项、比较与 IN/LIKE/IS 左侧标识符；解析不出来的位置逐条列进「显式跳过」。
 *   · CREATE / ALTER / PRAGMA / DROP 语句没有列清单可查（建表与补列已走 schema 收集）。
 *   · #[cfg(test)] 模块整体排除：测试会造 mock 库与临时表，不是生产列形状。
 *   · schema 的三处来源在输出里分别标注，出现在三处之外才算失败。
 */

import { readdirSync, readFileSync, statSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const COLUMNS_TS = path.join(ROOT, 'src', 'contracts', 'columns.ts')
const DTO_TS = path.join(ROOT, 'src', 'contracts', 'dto.ts')
const MIGRATIONS_DIR = path.join(ROOT, 'src-tauri', 'migrations')
// 扫整个 src-tauri/src 而不只是 db/：taxonomy.rs 自带连接、有 22 条 SQL，
// 只扫 db/*.rs 时它们的列名没有任何校验（本轮扩口径后 taxonomy/ai/scanner 一起进网）。
const DB_DIR = path.join(ROOT, 'src-tauri', 'src')

/** SQLite 内建目录表：不是项目 schema，SELECT 引用它们不算错。 */
const SQLITE_INTERNAL = new Set(['sqlite_master', 'sqlite_schema', 'sqlite_temp_master', 'sqlite_sequence'])

/** FROM 后面紧跟这些词时不是表别名。 */
const SQL_KEYWORDS = new Set([
  'where', 'group', 'order', 'limit', 'offset', 'join', 'inner', 'left', 'right', 'full', 'cross',
  'on', 'using', 'having', 'union', 'all', 'as', 'and', 'or', 'not', 'collate', 'desc', 'asc',
  'values', 'set', 'returning', 'select', 'from', 'window', 'between', 'like', 'glob', 'regexp',
  'is', 'in', 'null',
])

const problems = []
const allowed = []   // 例外清单命中的名字（显式放行）
const skipped = []   // 有意或无法解析而跳过的输入（显式跳过）
const fail = (msg) => problems.push(msg)
const allow = (msg) => allowed.push(msg)
const skip = (msg) => skipped.push(msg)

const read = (f) => readFileSync(f, 'utf8').replace(/\r\n/g, '\n')
const rel = (f) => path.relative(ROOT, f).split(path.sep).join('/')
const lineAt = (src, index) => src.slice(0, index).split('\n').length
const toSnake = (s) => s.replace(/[A-Z]/g, (c) => '_' + c.toLowerCase())
const QUOTES = "'" + '"' + '`'

function* walk(dir, re) {
  for (const e of readdirSync(dir).sort()) {
    const p = path.join(dir, e)
    if (statSync(p).isDirectory()) yield* walk(p, re)
    else if (re.test(e)) yield p
  }
}

/** 从 openAt（指向 open 字符）找配对闭合；quotes 里的字符视为引号，其内部括号不算。 */
function matchClose(src, openAt, open, close, quotes) {
  let depth = 0
  for (let i = openAt; i < src.length; i++) {
    const ch = src[i]
    if (quotes.includes(ch)) {
      let j = i + 1
      while (j < src.length && src[j] !== ch) {
        if (src[j] === '\\') j++
        j++
      }
      if (j >= src.length) return -1
      i = j
      continue
    }
    if (ch === open) depth++
    else if (ch === close && --depth === 0) return i
  }
  return -1
}

/** 按顶层逗号切段（括号内的逗号不算），带回每段在 s 里的偏移。 */
function topSegments(s) {
  const out = []
  let depth = 0
  let start = 0
  let inStr = false
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]
    if (inStr) {
      if (ch === "'") inStr = false
      continue
    }
    if (ch === "'") inStr = true
    else if (ch === '(' || ch === '[') depth++
    else if (ch === ')' || ch === ']') depth--
    else if (ch === ',' && depth === 0) {
      out.push({ text: s.slice(start, i), at: start })
      start = i + 1
    }
  }
  out.push({ text: s.slice(start), at: start })
  return out
}

/** 每个下标处的括号深度，以及它是否落在字符串字面量内。 */
function scanFlags(s) {
  const d = new Array(s.length)
  const str = new Array(s.length)
  let depth = 0
  let inStr = false
  for (let i = 0; i < s.length; i++) {
    d[i] = depth
    str[i] = inStr
    const ch = s[i]
    if (inStr) {
      if (ch === "'") inStr = false
      continue
    }
    if (ch === "'") inStr = true
    else if (ch === '(') depth++
    else if (ch === ')' && depth > 0) depth--
  }
  return { d, str }
}

const isWordChar = (ch) => ch !== undefined && /[A-Za-z0-9_$]/.test(ch)
/** 下标 at 处是否为独立单词 word（大小写不敏感）。 */
function wordAt(s, at, word) {
  if (s.slice(at, at + word.length).toLowerCase() !== word) return false
  if (isWordChar(s[at - 1])) return false
  return !isWordChar(s[at + word.length])
}
/** 出错时贴一小段上下文。 */
function snippet(text, at) {
  const one = text.replace(/\s+/g, ' ').trim()
  const from = Math.max(0, at - 30)
  return (from > 0 ? '…' : '') + one.slice(from, from + 70) + '…'
}

/* ── TS / Rust 源码解析 ───────────────────────────────────────── */

/** 把 TS 注释替换成空格（保留换行，故行号不变）。 */
function maskTsComments(src) {
  const out = src.split('')
  let i = 0
  while (i < src.length) {
    const ch = src[i]
    if (ch === '/' && src[i + 1] === '/') {
      while (i < src.length && src[i] !== '\n') { out[i] = ' '; i++ }
    } else if (ch === '/' && src[i + 1] === '*') {
      out[i] = ' '
      out[i + 1] = ' '
      i += 2
      while (i < src.length && !(src[i] === '*' && src[i + 1] === '/')) {
        if (src[i] !== '\n') out[i] = ' '
        i++
      }
      if (i < src.length) { out[i] = ' '; out[i + 1] = ' '; i += 2 }
    } else if (ch === "'" || ch === '"' || ch === '`') {
      const end = src.indexOf(ch, i + 1)
      if (end < 0) break
      i = end + 1
    } else i++
  }
  return out.join('')
}

/** 去掉 #[cfg(test)] 模块（这些文件里它们都是文件末尾的 mod tests）。 */
function stripRustTests(src) {
  const keep = []
  let skipping = false
  for (const line of src.split('\n')) {
    if (!skipping && /^\s*#\[cfg\(test\)\]/.test(line)) { skipping = true; continue }
    if (skipping) {
      if (/^\}/.test(line)) skipping = false
      continue
    }
    keep.push(line)
  }
  return keep.join('\n')
}

function unescapeRust(next) {
  switch (next) {
    case 'n': return '\n'
    case 'r': return '\r'
    case 't': return '\t'
    case '0': return '\0'
    case '\\': return '\\'
    case '"': return '"'
    case "'": return "'"
    default: return '\\' + next
  }
}

/** Rust 源码里的字符串字面量（含 r#"..."# 原始串），附起始行号。 */
function rustLiterals(src) {
  const out = []
  let i = 0
  const n = src.length
  while (i < n) {
    const ch = src[i]
    if (ch === '/' && src[i + 1] === '/') {
      while (i < n && src[i] !== '\n') i++
      continue
    }
    if (ch === '/' && src[i + 1] === '*') {
      i += 2
      while (i < n && !(src[i] === '*' && src[i + 1] === '/')) i++
      i += 2
      continue
    }
    if (ch === "'") {
      // 字符字面量（'a'、'\\n'）与生命周期（'static）要分开，别把撇号当字符串开头
      const m = /^'(?:\\.|[^'\\\\])'/.exec(src.slice(i, i + 6))
      i += m ? m[0].length : 1
      continue
    }
    if (ch === '"') {
      let j = i + 1
      let text = ''
      while (j < n) {
        const c = src[j]
        if (c === '\\') { text += unescapeRust(src[j + 1]); j += 2; continue }
        if (c === '"') break
        text += c
        j++
      }
      out.push({ text, line: lineAt(src, i) })
      i = j + 1
      continue
    }
    if (ch === 'r' && (src[i + 1] === '"' || src[i + 1] === '#')) {
      let h = 0
      let j = i + 1
      while (src[j] === '#') { h++; j++ }
      if (src[j] === '"') {
        j++
        const end = src.indexOf('"' + '#'.repeat(h), j)
        if (end >= 0) {
          out.push({ text: src.slice(j, end), line: lineAt(src, i) })
          i = end + h + 1
          continue
        }
      }
    }
    i++
  }
  return out
}

/** 取「导出名 = { ... }」里那个花括号的 body。 */
function objectBody(src, marker, where) {
  const at = src.indexOf(marker)
  if (at < 0) { fail('解析失败：' + where + ' 里找不到 ' + marker); return null }
  const open = src.indexOf('{', at)
  const close = open < 0 ? -1 : matchClose(src, open, '{', '}', QUOTES)
  if (close < 0) { fail('解析失败：' + where + ' 的 ' + marker + ' 花括号未配平'); return null }
  return src.slice(open + 1, close)
}

/** 扁平映射 { 'k': 'v' } / { k: 'v' } → Map。 */
function flatStringMap(body, where) {
  const out = new Map()
  const re = /(?:'([^']+)'|([A-Za-z_][A-Za-z0-9_]*))\s*:\s*'/g
  let m
  while ((m = re.exec(body))) {
    let j = body.indexOf("'", m.index + m[0].length - 1)
    let value = ''
    let closed = false
    for (j = j + 1; j < body.length; j++) {
      if (body[j] === '\\') { value += body[j + 1]; j++; continue }
      if (body[j] === "'") { closed = true; break }
      value += body[j]
    }
    if (!closed) { fail('解析失败：' + where + ' 里 ' + (m[1] ?? m[2]) + ' 的字符串未闭合'); break }
    out.set(m[1] ?? m[2], value)
    re.lastIndex = j + 1
  }
  return out
}

/** 嵌套对象 { k: { ... } } → Map(k → body)。 */
function nestedBodies(body) {
  const out = new Map()
  const re = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*\{/g
  let m
  while ((m = re.exec(body))) {
    const open = body.indexOf('{', m.index)
    const close = matchClose(body, open, '{', '}', QUOTES)
    if (close < 0) { fail('解析失败：EXCEPTIONS.' + m[1] + ' 的对象未闭合'); break }
    out.set(m[1], body.slice(open + 1, close))
    re.lastIndex = close
  }
  return out
}

/** { table: ['col', ...] } → Map(table → 列名数组)。 */
function arrayBodies(body) {
  const out = new Map()
  const re = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*\[/g
  let m
  while ((m = re.exec(body))) {
    const open = body.indexOf('[', m.index)
    const close = matchClose(body, open, '[', ']', QUOTES)
    if (close < 0) { fail('解析失败：DB_COLUMNS.' + m[1] + ' 的数组未闭合'); break }
    out.set(m[1], [...body.slice(open + 1, close).matchAll(/'([^']*)'/g)].map((x) => x[1]))
    re.lastIndex = close
  }
  return out
}

/** dto.ts 的导出接口 → Map(接口名 → [{ name, line }])。 */
function parseInterfaces(src) {
  const out = new Map()
  const re = /export\s+interface\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{/g
  let m
  while ((m = re.exec(src))) {
    const open = src.indexOf('{', m.index)
    const close = matchClose(src, open, '{', '}', QUOTES)
    if (close < 0) { fail('解析失败：' + rel(DTO_TS) + ' 里 ' + m[1] + ' 的花括号未闭合'); break }
    const body = src.slice(open + 1, close)
    const fields = []
    for (const fm of body.matchAll(/(?:^|\n)[ \t]*([A-Za-z_][A-Za-z0-9_]*)\??\s*:/g)) {
      fields.push({ name: fm[1], line: lineAt(src, open + 1 + fm.index) })
    }
    out.set(m[1], fields)
    re.lastIndex = close
  }
  return out
}

/* ── schema 收集：三处来源 ───────────────────────────────────── */

/** table → col → { file, line, from }；from ∈ sql-migration / rust-table / rust-add。 */
const schema = new Map()
const rustSources = []
/** CREATE TABLE 定义过的表 vs 只被 ensure_column / ALTER ADD COLUMN 碰过的表（后者若是新表就是 bug）。 */
const createTables = new Set()
const addedTables = new Set()

function addColumn(table, col, info) {
  const t = table.toLowerCase()
  if (info.from === 'rust-add') addedTables.add(t)
  else createTables.add(t)
  if (!schema.has(t)) schema.set(t, new Map())
  const cols = schema.get(t)
  if (!cols.has(col)) cols.set(col, info)
}

function collectDdl(sqlText, file, lineOf, from) {
  const re = /\bCREATE\s+(?:TEMP(?:ORARY)?\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*\(/gi
  let m
  while ((m = re.exec(sqlText))) {
    const table = m[1]
    const open = m.index + m[0].length - 1
    const close = matchClose(sqlText, open, '(', ')', "'" + '"')
    if (close < 0) {
      fail('解析失败：' + file + ':' + lineOf(m.index) + ' 的 CREATE TABLE ' + table + ' 括号未配平')
      continue
    }
    for (const seg of topSegments(sqlText.slice(open + 1, close))) {
      const flat = seg.text.replace(/\s+/g, ' ').trim()
      if (!flat) continue
      if (/^(primary|foreign|unique|constraint|check)\b/i.test(flat)) continue
      const cm = /^([A-Za-z_][A-Za-z0-9_]*)\s+\S/.exec(flat)
      if (!cm) {
        fail('解析失败：' + file + ':' + lineOf(open + 1 + seg.at) + ' 的 ' + table + ' 定义段读不出列名：' + flat)
        continue
      }
      addColumn(table, cm[1], { file, line: lineOf(open + 1 + seg.at), from })
    }
    re.lastIndex = close
  }
}

const columnsSrc = read(COLUMNS_TS)
const columnsMasked = maskTsComments(columnsSrc)
const dtoSrc = maskTsComments(read(DTO_TS))

const dbColumnsBody = objectBody(columnsMasked, 'export const DB_COLUMNS', rel(COLUMNS_TS))
const dtoFieldsBody = objectBody(columnsMasked, 'export const DTO_FIELDS', rel(COLUMNS_TS))
const dtoTablesBody = objectBody(columnsMasked, 'export const DTO_TABLES', rel(COLUMNS_TS))
const dtoNoTableBody = objectBody(columnsMasked, 'export const DTO_NO_TABLE', rel(COLUMNS_TS))
const exceptionsBody = objectBody(columnsMasked, 'export const EXCEPTIONS', rel(COLUMNS_TS))
if (!exceptionsBody) {
  console.error('check:columns 失败：无法从 ' + rel(COLUMNS_TS) + ' 解析出 EXCEPTIONS')
  process.exit(1)
}

const dbColumns = arrayBodies(dbColumnsBody)
const dtoFields = new Map([...nestedBodies(dtoFieldsBody)].map(([k, v]) => [k, flatStringMap(v, 'DTO_FIELDS.' + k)]))
const dtoTables = flatStringMap(dtoTablesBody, 'DTO_TABLES')
const dtoNoTable = flatStringMap(dtoNoTableBody, 'DTO_NO_TABLE')
const exceptionBuckets = nestedBodies(exceptionsBody)
const exDbColumns = flatStringMap(exceptionBuckets.get('DB_COLUMNS') ?? '', 'EXCEPTIONS.DB_COLUMNS')
const exDtoFields = flatStringMap(exceptionBuckets.get('DTO_FIELDS') ?? '', 'EXCEPTIONS.DTO_FIELDS')
const exRustCols = flatStringMap(exceptionBuckets.get('RUST_SQL_COLUMNS') ?? '', 'EXCEPTIONS.RUST_SQL_COLUMNS')

// 来源一：SQL 迁移文件
const sqlFiles = [...walk(MIGRATIONS_DIR, /\.sql$/)]
if (!sqlFiles.length) fail('找不到任何迁移文件：' + rel(MIGRATIONS_DIR) + '/*.sql')
for (const f of sqlFiles) {
  const src = read(f)
  collectDdl(src, rel(f), (i) => lineAt(src, i), 'sql-migration')
}

// 来源二、三：Rust 生产代码里的建表与补列
const dbRsFiles = [...walk(DB_DIR, /\.rs$/)].map((f) => ({ file: f, src: stripRustTests(read(f)) }))
let literalCount = 0
let sqlLiteralCount = 0
const stmtKinds = new Map()

for (const { file, src } of dbRsFiles) {
  for (const lit of rustLiterals(src)) {
    literalCount++
    const kind = /\b(select|insert\s+into|update|delete\s+from|create\s+(?:temp(?:orary)?\s+)?table|alter\s+table|pragma)\b/i.exec(lit.text)
    if (!kind) continue
    sqlLiteralCount++
    const word = kind[1].toLowerCase().split(/\s/)[0]
    stmtKinds.set(word, (stmtKinds.get(word) ?? 0) + 1)
    if (word === 'create') {
      collectDdl(lit.text, rel(file), () => lit.line, 'rust-table')
      rustSources.push(rel(file) + ':' + lit.line + ' 建表')
    }
    if (word === 'alter') {
      for (const am of lit.text.matchAll(/\bALTER\s+TABLE\s+([A-Za-z_][A-Za-z0-9_]*)\s+ADD\s+COLUMN\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z_][A-Za-z0-9_]*)/gi)) {
        addColumn(am[1], am[2], { file: rel(file), line: lit.line, from: 'rust-add' })
        rustSources.push(rel(file) + ':' + lit.line + ' 补列 ' + am[1] + '.' + am[2])
      }
    }
  }
  // ensure_column(conn, "tracks", "loudness_lufs", "REAL")：SQLite 没有 ADD COLUMN IF NOT EXISTS
  for (const em of src.matchAll(/ensure_column\s*\(\s*[^,]*,\s*"([^"]+)"\s*,\s*"([^"]+)"/g)) {
    const line = lineAt(src, em.index)
    addColumn(em[1], em[2], { file: rel(file), line, from: 'rust-add' })
    rustSources.push(rel(file) + ':' + line + ' 补列 ' + em[1] + '.' + em[2])
  }
}

/* ── (a) DB_COLUMNS 的列名必须真实存在 ───────────────────────── */

let dbColumnCount = 0
for (const [table, cols] of dbColumns) {
  dbColumnCount += cols.length
  const inSchema = schema.get(table)
  if (!inSchema) {
    fail('(a) DB_COLUMNS 登记了 schema 里不存在的表：' + table)
    continue
  }
  for (const col of cols) {
    if (!inSchema.has(col)) fail('(a) ' + rel(COLUMNS_TS) + ' 的 DB_COLUMNS.' + table + ' 收录了 schema 里没有的列：' + col)
  }
}

/* ── (b) Rust SQL 引用的列必须存在 ─────────────────────────────
 *
 * 位置覆盖：SELECT 列表 · INSERT 列表 · ON CONFLICT(...) · SET 左值（含 DO UPDATE SET）·
 * ORDER BY / GROUP BY 项 · 比较与 IN / LIKE / IS 左侧的标识符。位置解析不出来的进「显式跳过」。
 */

const selectRefs = []
const known = (t, c) => schema.get(t)?.has(c) ?? false
const STATEMENT_KINDS = new Set(['select', 'insert', 'update', 'delete', 'with'])
const OP_RE = /([A-Za-z_][A-Za-z0-9_]*)(?:\s*\.\s*([A-Za-z_][A-Za-z0-9_]*))?\s*(==|!=|<>|<=|>=|<|>|=|(?:not\s+)?(?:in|like|glob|match|between|is)\b)/gi

/** 语句里出现过的表与别名（FROM / JOIN / INTO / UPDATE 后面那个词）。 */
function statementTables(text) {
  const aliases = new Map()
  const tables = new Set()
  for (const m of text.matchAll(/\b(?:from|join|into|update)\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s+(?:as\s+)?([A-Za-z_][A-Za-z0-9_]*))?/gi)) {
    const t = m[1].toLowerCase()
    if (SQL_KEYWORDS.has(t)) continue   // 别把 DO UPDATE SET 的 SET 当表名
    tables.add(t)
    aliases.set(t, t)
    const a = m[2] ? m[2].toLowerCase() : null
    if (a && !SQL_KEYWORDS.has(a)) aliases.set(a, t)
  }
  const target = (/\b(?:insert\s+(?:or\s+\w+\s+)?into|update|delete\s+from)\s+([A-Za-z_][A-Za-z0-9_]*)/i.exec(text) ?? [])[1]
  return { aliases, tables, target: target ? target.toLowerCase() : null }
}

/** 每个 SELECT 出现处：列表起点、同层 FROM、FROM 后面那张表。 */
function selectCores(text, d, str) {
  const out = []
  const re = /\bselect\b/gi
  let m
  while ((m = re.exec(text))) {
    if (str[m.index]) continue
    const base = d[m.index]
    let from = -1
    for (let j = m.index + m[0].length; j < text.length; j++) {
      if (str[j]) continue
      if (d[j] < base) break
      if (text[j] === ')' && d[j] === base) break
      if (d[j] === base && wordAt(text, j, 'from')) { from = j; break }
    }
    if (from < 0) {
      skip('(b) SELECT 找不到同层 FROM，整条未做列校验 —— ' + snippet(text, m.index))
      continue
    }
    let k = from + 4
    while (k < text.length && /\s/.test(text[k])) k++
    const tm = /^([A-Za-z_][A-Za-z0-9_]*)/.exec(text.slice(k))
    const table = tm && !SQL_KEYWORDS.has(tm[1].toLowerCase()) ? tm[1].toLowerCase() : null
    out.push({ listStart: m.index + m[0].length, from, table })
  }
  return out
}

/** 从句 start 起到同层的 where/) 为止的正文（用于 SET / ORDER BY 这类子句）。 */
function clauseText(text, d, str, start, stops) {
  const base = d[start]
  for (let j = start; j < text.length; j++) {
    if (str[j]) continue
    if (d[j] < base) return text.slice(start, j)
    if (text[j] === ')' && d[j] === base) return text.slice(start, j)
    if (d[j] === base && stops.some((w) => wordAt(text, j, w))) return text.slice(start, j)
  }
  return text.slice(start)
}

function checkStatement(text, where) {
  const head = /\b(select|insert|update|delete|drop|create|alter|pragma|with)\b/i.exec(text)
  const kind = head ? head[1].toLowerCase() : ''
  if (!STATEMENT_KINDS.has(kind)) return   // CREATE/ALTER/PRAGMA/DROP 没有列清单（建表与补列已走 schema 收集）
  const { d, str } = scanFlags(text)
  const stmt = statementTables(text)
  const tables = [...stmt.tables].filter((t) => !SQLITE_INTERNAL.has(t))
  const unknown = tables.filter((t) => !schema.has(t))
  if (unknown.length) {
    for (const t of unknown) fail('(b) ' + where + ' 引用了 schema 里没有的表：' + t)
    skip('(b) ' + where + '：语句里有未识别的表，整条跳过列校验 —— ' + snippet(text, 0))
    return
  }
  if (!tables.length) {
    skip('(b) ' + where + '：语句里没有可识别的表（只查 SQLite 内建表或子查询），跳过列校验 —— ' + snippet(text, 0))
    return
  }
  const single = tables.length === 1 ? tables[0] : null
  const selectAliases = new Set([...text.matchAll(/\s+as\s+([A-Za-z_][A-Za-z0-9_]*)/gi)].map((m) => m[1].toLowerCase()))

  const checkIdent = (name, qualifier, label) => {
    const n = name.toLowerCase()
    if (qualifier) {
      const q = qualifier.toLowerCase()
      const t = q === 'excluded' ? stmt.target : stmt.aliases.get(q) ?? (schema.has(q) ? q : null)
      if (!t) { fail('(b) ' + where + ' 的 ' + label + ' 用了无法解析的表别名：' + q + '.' + n); return }
      if (!known(t, n)) fail('(b) ' + where + ' 的 ' + label + ' 引用了 ' + t + ' 不存在的列：' + n)
      return
    }
    selectRefs.push(n)
    if (single) {
      if (known(single, n)) return
      if (exRustCols.has(n)) { allow('EXCEPTIONS.RUST_SQL_COLUMNS：' + n + '（' + where + '）'); return }
      fail('(b) ' + where + ' 的 ' + label + ' 引用了 ' + single + ' 不存在的列：' + n)
      return
    }
    if ([...schema.keys()].some((t) => known(t, n))) return
    if (selectAliases.has(n)) return   // SELECT 列表里起的别名（ORDER BY 常引用它）
    if (exRustCols.has(n)) { allow('EXCEPTIONS.RUST_SQL_COLUMNS：' + n + '（' + where + '）'); return }
    fail('(b) ' + where + ' 的 ' + label + ' 引用了所有表里都不存在的列：' + n)
  }

  // ① SELECT 列表
  for (const core of selectCores(text, d, str)) {
    const coreTable = core.table && !SQLITE_INTERNAL.has(core.table) ? core.table : null
    for (const seg of topSegments(text.slice(core.listStart, core.from))) {
      const raw = seg.text.trim()
      if (!raw) continue
      const asM = /\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*$/i.exec(raw)
      const bare = (asM ? raw.slice(0, asM.index) : raw).replace(/^distinct\s+/i, '').trim()
      const idm = /^([A-Za-z_][A-Za-z0-9_]*)(?:\s*\.\s*([A-Za-z_][A-Za-z0-9_]*))?$/.exec(bare)
      if (!idm) {
        skip('(b) ' + where + '：SELECT 项不是裸列名，不校验 —— ' + raw)
        continue
      }
      const n = (idm[2] ?? idm[1]).toLowerCase()
      if (idm[2]) { checkIdent(n, idm[1], 'SELECT 列'); continue }
      if (coreTable) {
        selectRefs.push(n)
        if (known(coreTable, n)) continue
        if (exRustCols.has(n)) { allow('EXCEPTIONS.RUST_SQL_COLUMNS：' + n + '（' + where + '）'); continue }
        fail('(b) ' + where + ' 的 SELECT 引用了 ' + coreTable + ' 不存在的列：' + n)
        continue
      }
      checkIdent(n, null, 'SELECT 列')
    }
  }

  // ② INSERT 的显式列清单
  for (const m of text.matchAll(/\binsert\s+(?:or\s+\w+\s+)?into\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/gi)) {
    if (str[m.index]) continue
    const t = m[1].toLowerCase()
    const open = m.index + m[0].length - 1
    const close = matchClose(text, open, '(', ')', "'" + '"')
    if (close < 0) { skip('(b) ' + where + '：INSERT 列表括号未配平，不校验 —— ' + snippet(text, m.index)); continue }
    for (const seg of topSegments(text.slice(open + 1, close))) {
      const n = seg.text.trim()
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(n)) { skip('(b) ' + where + '：INSERT 列表项不是裸列名，不校验 —— ' + n); continue }
      selectRefs.push(n.toLowerCase())
      if (!SQLITE_INTERNAL.has(t) && !known(t, n)) fail('(b) ' + where + ' 的 INSERT 列表引用了 ' + t + ' 不存在的列：' + n)
    }
  }

  // ③ SET 左值（UPDATE ... SET 与 DO UPDATE SET 同一套）
  for (const m of text.matchAll(/\bset\b/gi)) {
    if (str[m.index]) continue
    const body = clauseText(text, d, str, m.index + m[0].length, ['where'])
    for (const seg of topSegments(body)) {
      const item = seg.text.trim()
      if (!item) continue
      const eq = item.indexOf('=')
      const lhs = (eq >= 0 ? item.slice(0, eq) : item).trim()
      const idm = /^([A-Za-z_][A-Za-z0-9_]*)(?:\s*\.\s*([A-Za-z_][A-Za-z0-9_]*))?$/.exec(lhs)
      if (!idm) { skip('(b) ' + where + '：SET 左值不是裸列名，不校验 —— ' + lhs); continue }
      checkIdent(idm[2] ?? idm[1], idm[2] ? idm[1] : null, 'SET ' + lhs)
    }
  }

  // ④ ON CONFLICT(...) 的冲突键
  for (const m of text.matchAll(/\bon\s+conflict\s*\(/gi)) {
    if (str[m.index]) continue
    const open = m.index + m[0].length - 1
    const close = matchClose(text, open, '(', ')', "'" + '"')
    if (close < 0) { skip('(b) ' + where + '：ON CONFLICT 括号未配平，不校验 —— ' + snippet(text, m.index)); continue }
    for (const seg of topSegments(text.slice(open + 1, close))) {
      const item = seg.text.trim().replace(/\s+(?:collate\s+[A-Za-z_][A-Za-z0-9_]*|asc|desc)$/i, '').trim()
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(item)) { skip('(b) ' + where + '：ON CONFLICT 项不是裸列名，不校验 —— ' + item); continue }
      checkIdent(item, null, 'ON CONFLICT ' + item)
    }
  }

  // ⑤ ORDER BY / GROUP BY 项
  for (const m of text.matchAll(/\b(?:order|group)\s+by\b/gi)) {
    if (str[m.index]) continue
    const body = clauseText(text, d, str, m.index + m[0].length, ['limit', 'order', 'group', 'having', 'union'])
    for (const seg of topSegments(body)) {
      const item = seg.text.trim()
      const idm = /^([A-Za-z_][A-Za-z0-9_]*)(?:\s*\.\s*([A-Za-z_][A-Za-z0-9_]*))?(?:\s+(?:collate\s+[A-Za-z_][A-Za-z0-9_]*|asc|desc))*$/i.exec(item)
      if (!idm) { skip('(b) ' + where + '：ORDER/GROUP BY 项不是裸列名，不校验 —— ' + item); continue }
      checkIdent(idm[2] ?? idm[1], idm[2] ? idm[1] : null, 'ORDER/GROUP BY ' + item)
    }
  }

  // ⑥ 比较、IN、LIKE、IS 左侧的标识符
  for (const m of text.matchAll(OP_RE)) {
    if (str[m.index]) continue
    checkIdent(m[2] ?? m[1], m[2] ? m[1] : null, '比较左侧 ' + m[0].replace(/\s+/g, ' ').trim())
  }
}

for (const { file, src } of dbRsFiles) {
  for (const lit of rustLiterals(src)) {
    if (!/\b(select|insert\s+(?:or\s+\w+\s+)?into|update\s+[A-Za-z_]|delete\s+from|with\s+[A-Za-z_])\b/i.test(lit.text)) continue
    checkStatement(lit.text, rel(file) + ':' + lit.line)
  }
}

for (const t of addedTables) {
  if (!createTables.has(t)) {
    fail('(b) 补列（ensure_column / ALTER TABLE ADD COLUMN）指向了一张没有 CREATE TABLE 定义的表：' + t +
      ' —— 运行时会当场报 no such table')
  }
}

/* ── (c) dto.ts 的字段必须映射到列 ───────────────────────────── */

const dtoIfaces = parseInterfaces(dtoSrc)
let dtoFieldCount = 0
for (const [iface, fields] of dtoIfaces) {
  dtoFieldCount += fields.length
  if (!dtoTables.has(iface) && !dtoNoTable.has(iface)) {
    fail('(c) ' + rel(DTO_TS) + ' 的 interface ' + iface + ' 没有分类：加进 ' + rel(COLUMNS_TS) + ' 的 DTO_TABLES（单表投影）或 DTO_NO_TABLE（附一行理由）')
  }
}
for (const [iface] of dtoNoTable) {
  if (!dtoIfaces.has(iface)) fail('(c) DTO_NO_TABLE 里的 ' + iface + ' 在 dto.ts 里不存在')
  if (dtoTables.has(iface)) fail('(c) ' + iface + ' 同时出现在 DTO_TABLES 与 DTO_NO_TABLE')
  if (!(dtoNoTable.get(iface) ?? '').trim()) fail('(c) DTO_NO_TABLE.' + iface + ' 没写理由')
}
for (const [iface, table] of dtoTables) {
  const fields = dtoIfaces.get(iface)
  if (!fields) { fail('(c) DTO_TABLES 里的 ' + iface + ' 在 dto.ts 里不存在（幽灵接口）'); continue }
  const cols = dbColumns.get(table)
  if (!cols) { fail('(c) DTO_TABLES.' + iface + ' 指向的表没在 DB_COLUMNS 登记：' + table); continue }
  const inSchema = schema.get(table)
  const declared = dtoFields.get(iface) ?? new Map()
  for (const f of fields) {
    const key = iface + '.' + f.name
    if (declared.has(f.name)) {
      const col = declared.get(f.name)
      if (!inSchema || !inSchema.has(col)) fail('(c) DTO_FIELDS.' + key + ' 指向的列不存在：' + table + '.' + col)
      // 列必须在 DB_COLUMNS 登记，或在 EXCEPTIONS.DB_COLUMNS 里说明为什么没登记（例如写死 NULL 的死列）
      else if (!cols.includes(col) && !exDbColumns.has(table + '.' + col)) {
        fail('(c) DTO_FIELDS.' + key + ' 指向的列没在 DB_COLUMNS.' + table + ' 登记，也没进 EXCEPTIONS.DB_COLUMNS：' + col)
      }
      continue
    }
    if (cols.includes(toSnake(f.name))) continue
    if (exDtoFields.has(key)) { allow('EXCEPTIONS.DTO_FIELDS：' + key + '（' + rel(DTO_TS) + ':' + f.line + '）'); continue }
    fail('(c) ' + rel(DTO_TS) + ':' + f.line + ' 的 ' + key + ' 既命中不了 ' + table + ' 的列（' + toSnake(f.name) + '），也不在 EXCEPTIONS.DTO_FIELDS 里')
  }
  for (const [field] of declared) {
    if (!fields.some((f) => f.name === field)) fail('(c) DTO_FIELDS.' + iface + ' 里的字段在 dto.ts 的 ' + iface + ' 中不存在：' + field)
  }
}

/* ── (d) 反向：schema 的列必须被登记或被例外放行 ───────────────── */

for (const [table, cols] of schema) {
  if (!dbColumns.has(table)) continue
  for (const [col, info] of cols) {
    if (dbColumns.get(table).includes(col)) continue
    const key = table + '.' + col
    if (exDbColumns.has(key)) { allow('EXCEPTIONS.DB_COLUMNS：' + key); continue }
    fail('(d) ' + info.file + ':' + info.line + ' 的 ' + key + ' 既没在 DB_COLUMNS.' + table + ' 登记，也不在 EXCEPTIONS.DB_COLUMNS 里')
  }
}

/* ── 例外清单自身的卫生：用不上的条目也算失败 ──────────────────── */

for (const [key, reason] of exDbColumns) {
  const dot = key.indexOf('.')
  const table = dot > 0 ? key.slice(0, dot) : ''
  const col = dot > 0 ? key.slice(dot + 1) : ''
  if (!table || !col) { fail('例外清单 DB_COLUMNS 的键不是 <表>.<列> 形式：' + key); continue }
  if (!schema.get(table) || !schema.get(table).has(col)) { fail('例外清单 DB_COLUMNS.' + key + ' 指向 schema 里不存在的列'); continue }
  if (dbColumns.get(table)?.includes(col)) fail('例外清单 DB_COLUMNS.' + key + ' 已用不上（该列已在 DB_COLUMNS 登记），请删掉')
  if (!reason.trim()) fail('例外清单 DB_COLUMNS.' + key + ' 没写理由')
}
for (const [key, reason] of exDtoFields) {
  const dot = key.indexOf('.')
  const iface = dot > 0 ? key.slice(0, dot) : ''
  const field = dot > 0 ? key.slice(dot + 1) : ''
  if (!iface || !field) { fail('例外清单 DTO_FIELDS 的键不是 <接口>.<字段> 形式：' + key); continue }
  const fields = dtoIfaces.get(iface)
  if (!fields) { fail('例外清单 DTO_FIELDS.' + key + ' 指向 dto.ts 里不存在的接口'); continue }
  if (!fields.some((f) => f.name === field)) { fail('例外清单 DTO_FIELDS.' + key + ' 指向 ' + iface + ' 里不存在的字段'); continue }
  if (dtoFields.get(iface)?.has(field)) fail('例外清单 DTO_FIELDS.' + key + ' 与 DTO_FIELDS 的显式登记冲突')
  const table = dtoTables.get(iface)
  if (table && dbColumns.get(table)?.includes(toSnake(field))) {
    fail('例外清单 DTO_FIELDS.' + key + ' 已用不上（' + toSnake(field) + ' 就是 ' + table + ' 的列），请删掉')
  }
  if (!reason.trim()) fail('例外清单 DTO_FIELDS.' + key + ' 没写理由')
}
for (const [key, reason] of exRustCols) {
  if (!allowed.some((a) => a.indexOf('RUST_SQL_COLUMNS：' + key) >= 0)) fail('例外清单 RUST_SQL_COLUMNS.' + key + ' 这次扫描里没被用到，请删掉')
  if (!reason.trim()) fail('例外清单 RUST_SQL_COLUMNS.' + key + ' 没写理由')
}

/* ── 输出 ───────────────────────────────────────────────────── */

for (const t of [...schema.keys()].filter((x) => !dbColumns.has(x))) {
  skip('schema 里的非领域表（不进 DB_COLUMNS，也不做反向校验）：' + t + '（' + [...schema.get(t).keys()].join('/') + '）')
}
for (const s of rustSources) skip('Rust 侧 schema 来源：' + s)
const kinds = [...stmtKinds.entries()].map(([k, n]) => k + ' ' + n).join(' / ')
skip('SQL 语句类型分布（CREATE/ALTER/PRAGMA/DROP 无列清单可查）：' + kinds)
skip('扫描的字符串字面量 ' + literalCount + ' 个，其中像 SQL 的 ' + sqlLiteralCount + ' 个；#[cfg(test)] 模块整体跳过')

const stats = [
  dbColumns.size + ' 张表 / ' + dbColumnCount + ' 个列名',
  dbRsFiles.length + ' 个 db/*.rs / ' + selectRefs.length + ' 处列引用',
  dtoTables.size + ' 个 DTO 接口 / ' + dtoFieldCount + ' 个字段',
  exDbColumns.size + exDtoFields.size + exRustCols.size + ' 条例外',
].join(' · ')

const uniqProblems = [...new Set(problems)]
if (uniqProblems.length) {
  console.error('check:columns 失败（' + uniqProblems.length + ' 项）：')
  for (const p of uniqProblems) console.error('  - ' + p)
  console.error('')
  console.error('对账口径：' + stats)
} else {
  console.log('check:columns 通过：' + stats)
}
console.log('已显式放行 ' + allowed.length + ' 处（例外清单命中）、显式跳过 ' + skipped.length + ' 类（不静默）' + (verbose() ? '' : '，--list 看明细'))
if (verbose()) {
  for (const a of allowed) console.log('  · 放行 ' + a)
  for (const s of skipped) console.log('  · 跳过 ' + s)
}
process.exit(uniqProblems.length ? 1 : 0)

function verbose() {
  return process.argv.includes('--list')
}

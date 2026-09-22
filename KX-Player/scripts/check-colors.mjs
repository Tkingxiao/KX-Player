#!/usr/bin/env node
/**
 * 色值对账：.ulpi/design/DESIGN.md 的 `## Color (locked)`（唯一事实源）↔ src/styles/theme.css ↔ src/** 组件
 *
 * 用法：node scripts/check-colors.mjs        （= npm run check:colors，已串进 npm run check）
 *       node scripts/check-colors.mjs --list  （额外逐条打印「已放行」的明细）
 *
 * 全绿：打印统计并 exit 0；有任何一处不一致：逐条打印并 exit 1。
 *
 * 四项校验（判据定义见 DESIGN.md `## Color (locked)` 开头三段）：
 *   (1) theme.css 里每个字面色值（hex 与 rgb/rgba 三元组，忽略 α）必须在 DESIGN.md 的
 *       Color 段里查得到 —— 查不到就是「代码里活着一个没有 role 的颜色」，E19 的原始症状；
 *   (2) 反向：DESIGN.md 的每个 hex 必须真被 src/ 用到（hex 字面量或十进制三元组任一形式）
 *       —— 用不上的 role 是僵尸条目，会误导下一个改主题的人；
 *   (3) src/styles/ 之外的字面色值同样要有 role；并且 **accent 的字面值在 src/styles/ 之外
 *       一律禁止**（必须写 `rgb(var(--accent-rgb) / α)`，否则用户改取色器时它不跟着改）；
 *       确有「值是数据不是样式」的场合，进 EXCEPTIONS 并写明理由；
 *   (4) DESIGN.md 表内写了 OKLCH 的行，OKLCH 反算回 hex 必须等于同行 hex —— 该列由 hex 派生，
 *       不是第二事实源；第 16 批发现整列与 hex 互相矛盾，就是缺这一条。
 *
 * 解析原则：正则 + 显式例外，笨但可读。**不静默跳过** —— 被例外放行的文件、例外里已经不需要的
 * 条目，全都在输出里点名；例外清单用不上同样判失败（免得清单越写越长、把真问题盖过去）。
 *
 * 已知边界（有意为之，不是漏报）：
 *   · 只解析 `## Color (locked)` 一节（到下一个 `## ` 为止）；Scales → Shadow 等处的
 *     rgba(0,0,0,α) 属于几何/α 描述，不进色板对账。
 *   · α 不参与比对：同一 hex 的不同不透明度视为同一个 role（结构层公式在 DESIGN.md 里就是按 α 表列的）。
 *   · 组件里的 `rgb(var(--accent-rgb))` 之类派生写法不含字面色值，天然不进比对。
 *   · 不解析 CSS 语义：`--x: 8px` 这类值里没有色值就不会被匹配；`#abcdef` 出现在字符串里也算色值。
 */

import { readFileSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const APP_ROOT = path.resolve(import.meta.dirname, '..')
const REPO_ROOT = path.resolve(APP_ROOT, '..')
const DESIGN_MD = path.join(REPO_ROOT, '.ulpi', 'design', 'DESIGN.md')
const THEME_CSS = path.join(APP_ROOT, 'src', 'styles', 'theme.css')
const SRC_DIR = path.join(APP_ROOT, 'src')
const STYLES_DIR = path.join(APP_ROOT, 'src', 'styles')

/** 色值是数据、不是样式的文件：整文件放行，但必须在输出里点名。 */
const EXCEPTIONS = [
  { file: 'src/stores/settings.ts', why: 'accent 默认值的落地处（`#e63a2e` 就是 DESIGN.md Accent 行本身）' },
  { file: 'src/composables/useThemeEffect.ts', why: '读不到已存 accent 时的同一个默认值回退' },
  { file: 'src/features/settings/SettingsModal.vue', why: '取色器的 PRESETS 候选色板：给用户挑的 accent，不是界面色' },
  { file: 'src/features/inspector/InspectorPanel.vue', why: 'DEFAULT_SUB_STYLE 的字幕色是传给 mpv 的数据，不是 CSS' },
]

const problems = []
const allowed = []
const fail = (msg) => problems.push(msg)

/* ── 色值解析 ─────────────────────────────────────────────────────────── */

const clamp = (v) => Math.max(0, Math.min(255, Math.round(v)))
const toHex = (r, g, b) => '#' + [r, g, b].map((v) => clamp(v).toString(16).padStart(2, '0')).join('')

/** 从一段文本里抠出所有字面色值，归一化成 #rrggbb（α 丢弃）。
 *  只认三种写法：`#rrggbb`、`rgb()/rgba()` 里的三个十进制数、自定义属性值恰好是 `N N N`
 *  （tokens.css 的 `--accent-rgb: 230 58 46` 就靠这一条）。**故意不**把任意三个相邻数字当颜色 ——
 *  否则 `box-shadow: 0 0 24 rgba(…)`、`padding: 2 0 0` 全都会变成色值，误报淹没真问题。 */
function colorLiterals(text) {
  const out = []
  for (const m of text.matchAll(/#([0-9a-fA-F]{6})\b/g)) out.push({ hex: '#' + m[1].toLowerCase(), raw: m[0] })
  for (const m of text.matchAll(/\brgba?\(([^)]*)\)/g)) {
    if (m[1].includes('var(')) continue           // 派生写法，不是字面值
    const nums = m[1].split(/[\s,/]+/).filter((s) => s !== '').slice(0, 3).map(Number)
    if (nums.length === 3 && nums.every((n) => Number.isInteger(n) && n >= 0 && n <= 255))
      out.push({ hex: toHex(...nums), raw: m[0] })
  }
  for (const m of text.matchAll(/--[\w-]+\s*:\s*(\d{1,3})\s+(\d{1,3})\s+(\d{1,3})\s*(?:;|$|\/\*)/g))
    out.push({ hex: toHex(+m[1], +m[2], +m[3]), raw: m[0].trim() })
  return out
}

/* ── OKLCH → hex（CSS Color 4），供校验 (4) ────────────────────────────── */

const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
const gam = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055)
function oklchToHex(Lp, C, Hdeg) {
  const h = (Hdeg * Math.PI) / 180
  const a = C * Math.cos(h)
  const b = C * Math.sin(h)
  const L = Lp / 100
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
  return toHex(
    gam(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s) * 255,
    gam(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s) * 255,
    gam(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s) * 255,
  )
}

/* ── 读 DESIGN.md 的 Color 段 ─────────────────────────────────────────── */

function readColorSection() {
  let md
  try {
    md = readFileSync(DESIGN_MD, 'utf8')
  } catch {
    fail(`读不到色值事实源：${path.relative(REPO_ROOT, DESIGN_MD)}（门禁依赖它，不能跳过）`)
    return null
  }
  const start = md.indexOf('\n## Color (locked)')
  if (start < 0) {
    fail('DESIGN.md 里找不到 `## Color (locked)` 一节 —— 事实源被改名或删掉了')
    return null
  }
  const rest = md.slice(start + 1)
  const next = rest.indexOf('\n## ', 1)
  const section = next < 0 ? rest : rest.slice(0, next)
  const offset = md.slice(0, start).split('\n').length
  return { section, offset }
}

const colorSection = readColorSection()
const docHexes = new Map()   // #rrggbb -> 首次出现的行号
let accentHex = null

if (colorSection) {
  const { section, offset } = colorSection
  const lines = section.split('\n')
  lines.forEach((line, i) => {
    const lineNo = offset + i
    for (const c of colorLiterals(line)) if (!docHexes.has(c.hex)) docHexes.set(c.hex, lineNo)
    // 表格行：| role | oklch(...) | hex | ... —— 校验 (4)
    const cells = line.trim().startsWith('|') ? line.split('|').map((c) => c.trim()) : null
    if (cells && cells.length > 3) {
      const role = cells[1]
      const ok = cells[2].match(/oklch\(\s*([\d.]+)%\s+([\d.]+)(?:\s+([\d.]+))?/)
      const hexCell = cells[3].match(/#([0-9a-fA-F]{6})/)
      const rgbCell = cells[3].match(/rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/)
      if (role === 'Accent' && hexCell) accentHex = '#' + hexCell[1].toLowerCase()
      if (!ok) return
      const computed = oklchToHex(+ok[1], +ok[2], ok[3] === undefined ? 0 : +ok[3])
      const stated = hexCell ? '#' + hexCell[1].toLowerCase() : rgbCell ? toHex(+rgbCell[1], +rgbCell[2], +rgbCell[3]) : null
      if (stated && computed !== stated)
        fail(`DESIGN.md:${lineNo} role「${role}」的 OKLCH 与 hex 互相对不上：oklch(…) 反算 = ${computed}，表里写的 = ${stated}（该列应由 hex 派生）`)
    }
  })
  if (!accentHex) fail('DESIGN.md 的 Color 段里找不到 role 为 `Accent` 的行，accent 字面量禁令无法执行')
}

/* ── (1) theme.css 的每个字面值都要有 role ─────────────────────────────── */

const themeText = readFileSync(THEME_CSS, 'utf8')
const themeLines = themeText.split('\n')
let themeChecked = 0
themeLines.forEach((line, i) => {
  const m = line.match(/^\s*(--[\w-]+)\s*:\s*(.+?);/)
  if (!m) return
  for (const c of colorLiterals(m[2])) {
    themeChecked++
    if (colorSection && !docHexes.has(c.hex))
      fail(`theme.css:${i + 1} 变量 ${m[1]} 的值 ${c.raw}（归一化 ${c.hex}）在 DESIGN.md 的 Color 段里没有 role`)
  }
})

/* ── (3) src/ 里 styles 之外的字面值 ───────────────────────────────────── */

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = path.join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (/\.(vue|css|ts)$/.test(name)) out.push(p)
  }
  return out
}

const exceptionFiles = new Set(EXCEPTIONS.map((e) => e.file))
const usedExceptions = new Set()
const srcFiles = walk(SRC_DIR).filter((p) => !p.startsWith(STYLES_DIR + path.sep))
let srcChecked = 0
for (const file of srcFiles) {
  const rel = path.relative(APP_ROOT, file).replaceAll('\\', '/')
  const exempt = exceptionFiles.has(rel)
  if (exempt) usedExceptions.add(rel)
  const text = readFileSync(file, 'utf8')
  text.split('\n').forEach((line, i) => {
    for (const c of colorLiterals(line)) {
      srcChecked++
      if (exempt) { allowed.push(`${rel}:${i + 1} ${c.raw} —— 整文件放行：${EXCEPTIONS.find((e) => e.file === rel).why}`); continue }
      if (accentHex && c.hex === accentHex)
        fail(`${rel}:${i + 1} 写死了 accent 字面值 ${c.raw}：请改用 rgb(var(--accent-rgb) / α) 形式，否则用户改取色器时它不跟随`)
      else if (colorSection && !docHexes.has(c.hex))
        fail(`${rel}:${i + 1} 字面色值 ${c.raw}（归一化 ${c.hex}）在 DESIGN.md 的 Color 段里没有 role`)
    }
  })
}
for (const e of EXCEPTIONS) if (!usedExceptions.has(e.file)) fail(`例外清单里的 ${e.file} 已经不再含任何字面色值，条目该删`)

/* ── (2) DESIGN.md 的每个 hex 必须真被用到 ─────────────────────────────── */

// 用到 = src/（含 theme.css）里出现过同一归一化色值：hex 写法与十进制三元组写法都算。
const used = new Set()
for (const text of [themeText, ...srcFiles.map((f) => readFileSync(f, 'utf8'))])
  for (const c of colorLiterals(text)) used.add(c.hex)
if (colorSection) {
  for (const [hex, lineNo] of docHexes)
    if (!used.has(hex)) fail(`DESIGN.md:${lineNo} 的 ${hex} 在 src/ 里没有任何使用者（僵尸 role）`)
}

/* ── 输出 ──────────────────────────────────────────────────────────────── */

if (process.argv.includes('--list') && allowed.length) {
  console.log('已放行（例外清单命中的字面色值）：')
  for (const a of allowed) console.log('  · ' + a)
}
const stats = [
  `DESIGN.md role ${docHexes.size} 个 hex`,
  `theme.css ${themeChecked} 处字面值`,
  `组件侧 ${srcChecked} 处字面值 / ${srcFiles.length} 个文件`,
  `例外 ${allowed.length} 处（${usedExceptions.size} 个文件）`,
].join(' · ')

if (problems.length) {
  console.error(`色值对账失败：${problems.length} 处\n${stats}`)
  for (const p of problems) console.error('  ✗ ' + p)
  process.exit(1)
}
console.log(`色值对账通过：${stats}`)

/** 模糊搜索：简繁归一 + 拼音首字母前缀匹配（移植自旧 utils.js）。 */

type SifyFn = (s: string) => string
type PinyinFn = (s: string, opts: Record<string, unknown>) => string[]

let _sify: SifyFn | null = null
let _pinyin: PinyinFn | null = null
let _libsLoaded = false

const _normCache = new Map<string, string>()
const _initialsCache = new Map<string, string>()
const MAX_SEARCH_CACHE = 20000

function cacheValue<T>(cache: Map<string, T>, key: string, compute: () => T): T {
  if (cache.has(key)) {
    const v = cache.get(key)!
    cache.delete(key)
    cache.set(key, v)
    return v
  }
  const v = compute()
  cache.set(key, v)
  if (cache.size > MAX_SEARCH_CACHE) {
    const first = cache.keys().next().value
    if (first !== undefined) cache.delete(first)
  }
  return v
}

export async function loadSearchLibs(): Promise<void> {
  if (_libsLoaded) return
  _libsLoaded = true
  try { const m = await import('chinese-conv'); _sify = (m as { sify?: SifyFn }).sify ?? ((s: string) => s) } catch { _sify = (s) => s }
  try { const m = await import('pinyin-pro'); _pinyin = (m as { pinyin?: PinyinFn }).pinyin ?? null } catch { _pinyin = null }
  _normCache.clear()
  _initialsCache.clear()
}

export function resetSearchCaches(): void {
  _normCache.clear()
  _initialsCache.clear()
}

function normalizeForSearch(s: string): string {
  if (!s) return ''
  return cacheValue(_normCache, s, () => (_sify ? _sify(s) : s).toLowerCase())
}

function getPinyinInitials(s: string): string {
  if (!s) return ''
  return cacheValue(_initialsCache, s, () => {
    if (!_pinyin) return s.toLowerCase()
    const simplified = _sify ? _sify(s) : s
    return _pinyin(simplified, { pattern: 'first', toneType: 'none', type: 'array' }).join('')
  })
}

/** 匹配打分：>=0 命中（越大越相关），-1 未命中。 */
export function fuzzyMatchScore(text: string, query: string): number {
  if (!text || !query) return -1
  const nt = normalizeForSearch(text)
  const nq = normalizeForSearch(query)
  if (!nt || !nq) return -1
  if (nt === nq) return 100
  if (nt.startsWith(nq)) return 90 - Math.min(nt.length, 20)
  const idx = nt.indexOf(nq)
  if (idx >= 0) return 70 - Math.min(idx, 30) - Math.min(nt.length - nq.length, 20) * 0.5
  if (!/[a-z]/.test(nq)) return -1
  const qi = getPinyinInitials(nq)
  const ti = getPinyinInitials(nt)
  if (!qi || !ti) return -1
  if (ti.startsWith(qi)) return 45 - Math.min(ti.length - qi.length, 15)
  return -1
}

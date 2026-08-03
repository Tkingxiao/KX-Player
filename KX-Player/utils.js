// === KX-Player Utility Functions ===
// Extracted from script.js for modularity and maintainability.

export const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

// === Fuzzy Search ===
let _sify = null, _pinyinFn = null
const _normCache = new Map()
const _initialsCache = new Map()
const MAX_SEARCH_CACHE = 20000

function _clearSearchCaches() {
  _normCache.clear()
  _initialsCache.clear()
}

function _cacheValue(cache, key, compute) {
  if (cache.has(key)) {
    const value = cache.get(key)
    cache.delete(key)
    cache.set(key, value)
    return value
  }
  const value = compute()
  cache.set(key, value)
  if (cache.size > MAX_SEARCH_CACHE) cache.delete(cache.keys().next().value)
  return value
}

export async function _loadSearchLibs() {
  try { const m = await import('chinese-conv'); _sify = m.sify } catch { _sify = (s) => s }
  try { const m = await import('pinyin-pro'); _pinyinFn = m.pinyin } catch { _pinyinFn = null }
  _clearSearchCaches()
}
_loadSearchLibs()

function _normalizeForSearch(s) {
  if (!s) return ''
  const raw = String(s)
  return _cacheValue(_normCache, raw, () => (_sify ? _sify(raw) : raw).toLowerCase())
}

function _getPinyinInitials(s) {
  if (!s) return ''
  const raw = String(s)
  return _cacheValue(_initialsCache, raw, () => {
    if (!_pinyinFn) return raw.toLowerCase()
    const simplified = _sify ? _sify(raw) : raw
    return _pinyinFn(simplified, { pattern: 'first', toneType: 'none', type: 'array' }).join('')
  })
}

/** Fuzzy match: returns true if query matches text via direct substring or pinyin initials. */
/** Fuzzy match score: returns a relevance score >= 0 if it matches, or -1 if not. */
export function fuzzyMatchScore(text, query) {
  if (!text || !query) return -1
  const nt = _normalizeForSearch(text)
  const nq = _normalizeForSearch(query)
  if (!nt || !nq) return -1
  // Direct text match (higher for exact / prefix / earlier substring)
  if (nt === nq) return 100
  if (nt.startsWith(nq)) return 90 - Math.min(nt.length, 20)
  const idx = nt.indexOf(nq)
  if (idx >= 0) return 70 - Math.min(idx, 30) - Math.min(nt.length - nq.length, 20) * 0.5
  // Pinyin initials match: only for queries containing latin letters, and only
  // as a consecutive prefix of the field initials. This avoids loose
  // subsequence false positives (e.g. "ld" matching "艾琳的"/"ald").
  if (!/[a-z]/.test(nq)) return -1
  const queryInitials = _getPinyinInitials(nq)
  const textInitials = _getPinyinInitials(nt)
  if (!queryInitials || !textInitials) return -1
  if (textInitials.startsWith(queryInitials)) return 45 - Math.min(textInitials.length - queryInitials.length, 15)
  return -1
}

export function fuzzyMatch(text, query) {
  if (!text || !query) return false
  const nt = _normalizeForSearch(text)
  const nq = _normalizeForSearch(query)
  if (nt.includes(nq)) return true
  const queryInitials = _getPinyinInitials(nq)
  if (!queryInitials) return false
  const textInitials = _getPinyinInitials(nt)
  if (textInitials.startsWith(queryInitials)) return true
  let ti = 0
  for (let qi = 0; qi < queryInitials.length && ti < textInitials.length; qi++) {
    while (ti < textInitials.length && textInitials[ti] !== queryInitials[qi]) ti++
    if (ti >= textInitials.length) return false
    ti++
  }
  return true
}

// === DOM Helpers ===
export function $(sel) { return document.querySelector(/^[#.]/.test(sel) ? sel : '#' + sel) }
export function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/`/g, '&#96;').replace(/\$/g, '&#36;') }

// === Path Helpers ===
export function pathJoin(a, b) { return a.replace(/[/\\]+$/, '') + '\\' + b }
export function isChildPath(child, parent) {
  const np = parent.replace(/\\/g, '/').replace(/\/+$/, '')
  const nc = child.replace(/\\/g, '/')
  return nc.startsWith(np + '/')
}
export function arrayMatchSorted(a, b) {
  if (a.length !== b.length) return false
  const sa = [...a].sort(), sb = [...b].sort()
  for (let i = 0; i < sa.length; i++) { if (sa[i] !== sb[i]) return false }
  return true
}
export function hashPath(p) {
  let h1 = 0, h2 = 0
  for (let i = 0; i < p.length; i++) {
    h1 = ((h1 << 5) - h1) + p.charCodeAt(i); h1 |= 0
    h2 = ((h2 << 7) - h2) + p.charCodeAt(i); h2 |= 0
  }
  return 'dsd' + Math.abs(h1).toString(36) + Math.abs(h2).toString(36)
}

// === Formatting ===
export function fmtTime(t) { if (!t || !isFinite(t)) return '00:00'; const m = Math.floor(t / 60), s = Math.floor(t % 60); return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0') }
export function isVideoFile(t) { return t && t.isVideo === true }
export function nName(t) { const p = String(t.path || '').replace(/\\/g, '/').split('/').pop() || ''; return p.replace(/\.[^/.]+$/, '') || p }
export function fmtFSize(bytes) { if (!bytes || !isFinite(bytes)) return ''; const u = ['B', 'KB', 'MB', 'GB']; let i = 0; while (bytes >= 1024 && i < u.length - 1) { bytes /= 1024; i++ } return bytes.toFixed(i > 0 ? 1 : 0) + ' ' + u[i] }

// === Folder Tree Helpers ===
export function collectAllTracks(node) {
  const out = []
  collectAllTracksInto(node, out)
  return out
}
export function collectAllTracksInto(node, out) {
  if (!node) return
  for (const t of node.tracks) out.push(t)
  for (const c of node.children) collectAllTracksInto(c, out)
}

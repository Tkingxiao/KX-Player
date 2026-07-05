// === KX-Player Utility Functions ===
// Extracted from script.js for modularity and maintainability.

export const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

// === Fuzzy Search ===
let _sify = null, _pinyinFn = null
export async function _loadSearchLibs() {
  try { const m = await import('chinese-conv'); _sify = m.sify } catch { _sify = (s) => s }
  try { const m = await import('pinyin-pro'); _pinyinFn = m.pinyin } catch { _pinyinFn = null }
}
_loadSearchLibs()

function _normalizeForSearch(s) {
  if (!s) return ''
  return (_sify ? _sify(s) : s).toLowerCase()
}

function _getPinyinInitials(s) {
  if (!s || !_pinyinFn) return s ? s.toLowerCase() : ''
  const simplified = _sify ? _sify(s) : s
  return _pinyinFn(simplified, { pattern: 'first', toneType: 'none', type: 'array' }).join('')
}

/** Fuzzy match: returns true if query matches text via direct substring or pinyin initials. */
export function fuzzyMatch(text, query) {
  if (!text || !query) return false
  const nt = _normalizeForSearch(text)
  const nq = _normalizeForSearch(query)
  if (nt.includes(nq)) return true
  const textInitials = _getPinyinInitials(nt)
  const queryInitials = _getPinyinInitials(nq)
  if (!queryInitials) return false
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
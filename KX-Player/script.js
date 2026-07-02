import { api } from './api.js'

const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

// === Fuzzy Search Utilities ===
let _sify = null, _pinyinFn = null
async function _loadSearchLibs() {
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

// Fuzzy match: returns true if query matches text via any of:
// 1. Direct substring match (after normalization)
// 2. Pinyin initials match
function fuzzyMatch(text, query) {
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


const S = {
  af: [], all: [], aI: -1, alI: -1, tI: -1,
  playing: false, cTime: 0, dur: 0,
  vol: 50, pVol: 50, muted: false,
  mode: 0, playingTid: null,
  favs: [],
  recents: [],
  view: 'all', q: '',
  theme: 'light', clr: '#E63A2E',
  ovl: 72, devId: '',
  bgData: null, bgPath: null, bgSize: 'cover', pls: [], aPl: null, aF: null,
  selMode: false, bgBlur: 0,
  listTextColor: null, listTextColorsCached: null,
  folderTree: [], folderStack: [], _syncingView: false,
  folderSort: 'name', // 'name' | 'time' | 'tracks'
  folderView: 'grid', // 'grid' | 'list'
  activeFp: null, // currently active folder path in sidebar
  _searchFolders: [],
  _folderMeta: null, // precomputed folder metadata { path: { trackCount, validChildCount, hasMusic, coverData } }
}

let fp = [], audio = new Audio(), lrc = [], pl = [], nI = 0
let _lastLrcActiveIdx = -1
let _idCounter = 0
let _scanRunning = false
let _loadTGeneration = 0
let stopWatchingFs = null
let lyricsManualScrollUntil = 0
let dsdState = {
  active: false,
  path: null,
  context: null,
  gainNode: null,
  buffer: null,
  source: null,
  startedAt: 0,
  pausedAt: 0,
  duration: 0,
  raf: 0,
}

function $(sel) { return document.querySelector(/^[#.]/.test(sel) ? sel : '#' + sel) }
// Cache frequently accessed DOM elements for performance
const _progressFill = $('progress-fill')
const _progressHandle = $('progress-handle')
const _progressCurrent = $('progress-current')
const _progressDuration = $('progress-duration')
function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/`/g, '&#96;').replace(/\$/g, '&#36;') }
function pathJoin(a, b) { return a.replace(/[/\\]+$/, '') + '\\' + b }
function isChildPath(child, parent) {
  const np = parent.replace(/\\/g, '/').replace(/\/+$/, '')
  const nc = child.replace(/\\/g, '/')
  return nc.startsWith(np + '/')
}
function arrayMatchSorted(a, b) {
  if (a.length !== b.length) return false
  const sa = [...a].sort(), sb = [...b].sort()
  for (let i = 0; i < sa.length; i++) { if (sa[i] !== sb[i]) return false }
  return true
}
function hashPath(p) {
  // Double hash to reduce collision risk for large libraries
  let h1 = 0, h2 = 0
  for (let i = 0; i < p.length; i++) {
    h1 = ((h1 << 5) - h1) + p.charCodeAt(i); h1 |= 0
    h2 = ((h2 << 7) - h2) + p.charCodeAt(i); h2 |= 0
  }
  return 'dsd' + Math.abs(h1).toString(36) + Math.abs(h2).toString(36)
}
function fmtTime(t) { if (!t || !isFinite(t)) return '00:00'; const m = Math.floor(t / 60), s = Math.floor(t % 60); return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0') }
function isVideoFile(t) { return t && t.isVideo === true }

// Pre-computed set of default-favorite track IDs for O(1) lookup during rendering
let _defaultFavIds = null
function getDefaultFavIdSet() {
  if (_defaultFavIds) return _defaultFavIds
  const fav = S.favs.find(f => f.isDefault)
  _defaultFavIds = fav ? new Set(fav.trackIds) : new Set()
  return _defaultFavIds
}
function isDefaultFavTrack(tid) { return getDefaultFavIdSet().has(tid) }
function invalidateFavCache() { _defaultFavIds = null }

// === Sync Playing State ===
function syncPlayingState() {
  if (S.playingTid) {
    const idx = pl.findIndex(t => t.id === S.playingTid)
    if (idx >= 0) {
      S.tI = idx
      nI = idx
    } else {
      S.tI = -1
    }
  } else if (S.tI >= 0 && S.tI < pl.length) {
    S.playingTid = pl[S.tI]?.id || null
  }
}

// === Resize throttler: suspend VL renders during window resize ===
let _resizeActive = false
let _resizeTimer = null
function _startResizeThrottle() {
  _resizeActive = true
  clearTimeout(_resizeTimer)
  _resizeTimer = setTimeout(() => { _resizeActive = false }, 300)
}

// === Virtual List ===
function virtualList(containerId, items, rowHeight, renderItem, onClick) {
  const c = $(containerId)
  if (!c) return
  // Remove previous listeners and observer to prevent accumulation
  if (c._vlRO) { c._vlRO.disconnect(); c._vlRO = null }
  if (c._vlScrollFn) { c.removeEventListener('scroll', c._vlScrollFn) }
  if (c._vlClickFn) { c.removeEventListener('click', c._vlClickFn) }
  if (c._vlDblClickFn) { c.removeEventListener('dblclick', c._vlDblClickFn) }
  c.innerHTML = ''
  if (!items.length) { c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u6682\u65e0\u5185\u5bb9</h3></div>'; return }
  const totalH = items.length * rowHeight
  const spacer = document.createElement('div'); spacer.style.height = totalH + 'px'; spacer.style.position = 'relative'
  const view = document.createElement('div'); view.style.position = 'absolute'; view.style.top = '0'; view.style.left = '0'; view.style.right = '0'
  spacer.appendChild(view); c.appendChild(spacer)
  const buffer = 10
  function render() {
    if (_resizeActive) return
    const scrollTop = c.scrollTop, clientH = c.clientHeight || 600
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - buffer)
    const end = Math.min(items.length, Math.ceil((scrollTop + clientH) / rowHeight) + buffer)
    view.style.top = (start * rowHeight) + 'px'
    let html = ''
    for (let i = start; i < end; i++) html += renderItem(items[i], i)
    view.innerHTML = html
  }
  c._vlRender = render; c._vlItems = items; render()
  const scrollFn = () => render()
  c._vlScrollFn = scrollFn
  c.addEventListener('scroll', scrollFn, { passive: true })
  let resizeTimer
  const ro = new ResizeObserver(() => {
    if (_resizeActive) return
    clearTimeout(resizeTimer)
    resizeTimer = setTimeout(render, 200)
  })
  ro.observe(c); c._vlRO = ro
  if (onClick) {
    const clickFn = e => {
      const playBtn = e.target.closest('.idx-play-btn')
      if (playBtn) { const row = e.target.closest('.song-row'); if (row && row.dataset.tid) onClick(row.dataset.tid, true) }
    }
    const dblClickFn = e => {
      const row = e.target.closest('.song-row')
      if (row && row.dataset.tid) onClick(row.dataset.tid, true)
    }
    c._vlClickFn = clickFn; c._vlDblClickFn = dblClickFn
    c.addEventListener('click', clickFn)
    c.addEventListener('dblclick', dblClickFn)
  }
}
function invalidateVL(containerId) { const c = $(containerId); if (c && c._vlRender) c._vlRender() }

// === Sync All Lists Playing State ===
function syncAllListsPlaying() {
  document.querySelectorAll('.song-row').forEach(row => {
    const tid = row.dataset.tid
    const isPlaying = S.playingTid && tid === S.playingTid
    row.classList.toggle('playing', !!isPlaying)
    row.classList.toggle('is-playing-state', isPlaying && S.playing)
    row.classList.toggle('is-paused-state', isPlaying && !S.playing)
  })
}

// === Cleanup Stale Track References ===
function cleanupStale(allIds) {
  S.recents = S.recents.filter(id => allIds.has(id))
  for (const f of S.favs) {
    f.trackIds = f.trackIds.filter(id => allIds.has(id))
  }
  for (const p of S.pls) {
    p.trackIds = p.trackIds.filter(id => allIds.has(id))
  }
}

// === Custom Confirm ===
function showConfirm(title, message) {
  return new Promise(resolve => {
    $('confirm-title').textContent = title
    $('confirm-message').textContent = message
    $('confirm-modal').classList.remove('hidden')
    const okBtn = $('confirm-ok-btn')
    const cancelBtn = $('confirm-cancel-btn')
    const cleanup = () => {
      $('confirm-modal').classList.add('hidden')
      okBtn.removeEventListener('click', okHandler)
      cancelBtn.removeEventListener('click', cancelHandler)
      document.removeEventListener('keydown', keyHandler)
    }
    const okHandler = () => { cleanup(); resolve(true) }
    const cancelHandler = () => { cleanup(); resolve(false) }
    const keyHandler = (e) => { if (e.key === 'Escape') { e.preventDefault(); cancelHandler() } if (e.key === 'Enter') { e.preventDefault(); okHandler() } }
    okBtn.addEventListener('click', okHandler)
    cancelBtn.addEventListener('click', cancelHandler)
    document.addEventListener('keydown', keyHandler)
    okBtn.focus()
  })
}

// === Toast ===
let tC = 0
function addT(fn) {
  const id = 'toast-' + ++tC; $('import-toasts').insertAdjacentHTML('beforeend',
  `<div class="import-toast" id="${id}"><div class="toast-header"><span class="toast-name">${esc(fn)}</span><span class="toast-status" id="${id}-status">\u626b\u63cf\u4e2d...</span></div><div class="toast-progress-bar"><div class="toast-progress-fill" id="${id}-bar" style="width:0%"></div></div><div class="toast-detail" id="${id}-detail"></div></div>`)
  return id
}
function updT(id, status, pct, detail) {
  const s = $(`${id}-status`), b = $(`${id}-bar`), d = $(`${id}-detail`)
  if (s) s.textContent = status
  if (b) { b.style.width = pct + '%'; if (pct > 0) b.closest('.toast-progress-bar').style.display = 'block' }
  if (d) d.textContent = detail || ''
}
function rmT(id) { const t = $(id); if (t) { setTimeout(() => { t.classList.add('toast-exit'); setTimeout(() => t.remove(), 500) }, 1500) } }

// === IndexedDB Storage ===
let idb = null
function openIDB() {
  return new Promise((resolve, reject) => {
    if (idb) return resolve(idb)
    const req = indexedDB.open('kx-player-db', 1)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains('settings')) db.createObjectStore('settings')
      if (!db.objectStoreNames.contains('cache')) db.createObjectStore('cache')
    }
    req.onsuccess = () => { idb = req.result; resolve(idb) }
    req.onerror = () => reject(req.error)
  })
}
async function idbSet(store, key, val) {
  const db = await openIDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, 'readwrite')
    tx.objectStore(store).put(val, key)
    tx.oncomplete = () => resolve()
    tx.onerror = () => reject(tx.error)
  })
}
async function idbGet(store, key) {
  const db = await openIDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, 'readonly')
    const req = tx.objectStore(store).get(key)
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error)
  })
}

function applyScanResult(result) {
  const rs = result?.artists || result || []
  S.folderTree = result?.folderTree || []
  S._folderMeta = buildFolderMeta(S.folderTree)
  const at = result?.allTracks ? result.allTracks.map(t => ({ ...t })) : []
  if (!at.length) {
    for (const a of rs) {
      for (const al of a.albums) {
        for (const t of al.tracks) {
          t.albumCoverData = al.coverData
          at.push(t)
        }
      }
    }
  }
  S.af = rs
  S.all = at
  return at
}

async function restartWatching() {
  try {
    if (stopWatchingFs) {
      stopWatchingFs()
      stopWatchingFs = null
    }
    await api.stopWatching()
  } catch (e) { /* ignore */ }

  if (!fp.length) return

  try {
    stopWatchingFs = await api.onFsChanged(() => { rescan() })
    await api.startWatching(fp)
  } catch (e) { /* ignore */ }
}

// === Settings ===
let saveTimer = null
function schedSave() { clearTimeout(saveTimer); saveTimer = setTimeout(saveS, 300) }
async function saveS() {
  try {
    const data = JSON.parse(JSON.stringify({
      folderPaths: fp, favs: S.favs, recents: S.recents, view: S.view, q: S.q, theme: S.theme, clr: S.clr, ovl: S.ovl, devId: S.devId,
      bgPath: S.bgPath, bgSize: S.bgSize, aI: S.aI, alI: S.alI, tI: S.tI, playing: S.playing, cTime: S.cTime,
      dur: S.dur, vol: S.vol, muted: S.muted, mode: S.mode, pls: S.pls, aPl: S.aPl, aF: S.aF,
      bgBlur: S.bgBlur, selMode: S.selMode, folderStack: S.folderStack, folderSort: S.folderSort, folderView: S.folderView, _imgEditState: S._imgEditState, listTextColor: S.listTextColor,
      titlebarOpacity: S.titlebarOpacity, playerOpacity: S.playerOpacity, sidebarOpacity: S.sidebarOpacity,
      playingTid: S.playingTid
    }))
    // Sync-save to main process FIRST for immediate disk persistence
    api.syncSaveSettings(data)
    await api.saveSettings(data)
    await idbSet('settings', 'state', data)
  } catch (e) { /* ignore */ }
}

async function loadS() {
  try {
    let s = await api.loadSettings()
    // Fallback to IndexedDB if file settings are empty
    if (!s || Object.keys(s).length === 0) {
      try { s = await idbGet('settings', 'state') || {} } catch (e) { /* ignore */ }
    }
    if (!s || Object.keys(s).length === 0) return
    if (typeof s.mode === 'number') S.mode = s.mode; if (Array.isArray(s.recents)) S.recents = s.recents
    if (s.view) S.view = s.view; if (s.q) S.q = s.q; if (s.theme) S.theme = s.theme; if (s.clr) S.clr = s.clr
    if (typeof s.ovl === 'number') S.ovl = s.ovl; if (s.devId) S.devId = s.devId
    if ('bgPath' in s) S.bgPath = s.bgPath; if (s.bgSize) S.bgSize = s.bgSize
    if (typeof s.bgBlur === 'number') S.bgBlur = s.bgBlur
    if (s._imgEditState) S._imgEditState = s._imgEditState
    if (typeof s.aI === 'number') S.aI = s.aI; if (typeof s.alI === 'number') S.alI = s.alI; if (typeof s.tI === 'number') S.tI = s.tI
    if (typeof s.vol === 'number') S.vol = s.vol; if (typeof s.muted === 'boolean') S.muted = s.muted
    if (typeof s.sidebarOpacity === 'number') S.sidebarOpacity = s.sidebarOpacity; else S.sidebarOpacity = 100
    if (typeof s.titlebarOpacity === 'number') S.titlebarOpacity = s.titlebarOpacity; else S.titlebarOpacity = 100
    if (typeof s.playerOpacity === 'number') S.playerOpacity = s.playerOpacity; else S.playerOpacity = 100
    if (s.selMode) S.selMode = s.selMode; if (s.listTextColor) S.listTextColor = s.listTextColor
    if (s.playingTid) S.playingTid = s.playingTid
    if (Array.isArray(s.favs)) S.favs = s.favs; if (Array.isArray(s.pls)) S.pls = s.pls
    if (s.aPl) S.aPl = s.aPl; if (s.aF) S.aF = s.aF
    if (Array.isArray(s.folderStack)) S.folderStack = s.folderStack
    if (s.folderSort) S.folderSort = s.folderSort
    if (s.folderView) S.folderView = s.folderView

    // Load background image from app data directory file
    if (S.bgPath) {
      try {
        const bgResult = await api.loadBgImage()
        if (bgResult && bgResult.dataUrl) {
          S.bgData = bgResult.dataUrl
        }
      } catch (e) { /* ignore */ }
    }
    // Fallback to old bgData in settings or IDB cache
    if (!S.bgData && s.bgData) {
      S.bgData = s.bgData
    }
    if (!S.bgData) {
      try { const cached = await idbGet('cache', 'bgImage'); if (cached) S.bgData = cached } catch (e) { /* ignore */ }
    }

    // Store folder paths (library data is loaded separately for faster startup)
    if (Array.isArray(s.folderPaths)) {
      fp = s.folderPaths.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
    }
  } catch (e) { /* ignore */ }
}

async function loadLibraryData() {
  try {
    if (!fp.length) return
    console.time('[startup] total')
    // Load library from database (with covers) or fall back to scanning
    let library = await api.loadLibrary()
    const cache = library ? null : await api.loadCache()
    if (library && Array.isArray(library.folderPaths)) {
      const libraryPaths = library.folderPaths.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
      if (arrayMatchSorted(fp, libraryPaths)) {
        applyScanResult(library)
      } else {
        library = null
      }
    } else if (cache && Array.isArray(cache.folderPaths)) {
      const cachePaths = cache.folderPaths.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
      if (arrayMatchSorted(fp, cachePaths) && cache.scanResult) {
        applyScanResult(cache.scanResult)
        library = cache.scanResult
      }
    }
    if (!library) {
      const result = await api.scanFoldersWithProgress(fp)
      applyScanResult(result)
    }
    const allIds = new Set(S.all.map(t => t.id))
    cleanupStale(allIds)
    await restartWatching()
    console.timeEnd('[startup] total')
  } catch (e) { /* ignore */ }
}

// === Theme ===
function apTh() {
  const root = document.documentElement, isDark = S.theme === 'dark'
  root.style.setProperty('--accent', S.clr)
  const [r, g, b] = hex2rgb(S.clr)
  root.style.setProperty('--accent-rgb', `${r} ${g} ${b}`)
  root.style.setProperty('--accent-light', `rgb(${Math.min(255, r + 30)},${Math.min(255, g + 10)},${Math.min(255, b + 20)})`)
  root.style.setProperty('--accent-bg', `rgba(${r},${g},${b},0.35)`)
  root.style.setProperty('--accent-r', r); root.style.setProperty('--accent-g', g); root.style.setProperty('--accent-b', b)
  if (isDark) {
    root.style.setProperty('--bg', '#0d0d12'); root.style.setProperty('--bg-card', 'rgba(22,22,30,0.92)')
    root.style.setProperty('--bg-sidebar', 'rgba(16,16,22,0.84)'); root.style.setProperty('--bg-player', 'rgba(20,20,28,0.95)')
    root.style.setProperty('--bg-input', 'rgba(255,255,255,0.08)'); root.style.setProperty('--bg-hover', 'rgba(255,255,255,0.06)')
    root.style.setProperty('--bg-active', 'rgba(255,255,255,0.1)'); root.style.setProperty('--text', '#f0f0f5')
    root.style.setProperty('--text-sub', '#b8b8c0'); root.style.setProperty('--text-muted', '#606070')
    root.style.setProperty('--list-text', '#f0f0f5'); root.style.setProperty('--list-text-sub', '#b8b8c0')
    root.style.setProperty('--panel-text', '#ffffff'); root.style.setProperty('--panel-line', 'rgba(255,255,255,0.2)')
    root.style.setProperty('--lyrics-text', '#ffffff'); root.style.setProperty('--lyrics-text-sub', '#ffffff')
    root.style.setProperty('--lyrics-empty', '#ffffff')
    root.style.setProperty('--lyrics-border', '#ffffff')
    root.style.setProperty('--border', 'rgba(255,255,255,0.1)'); root.style.setProperty('--modal-bg', 'rgba(16,16,22,0.98)')
    root.style.setProperty('--modal-overlay', 'rgba(0,0,0,0.55)')
  } else {
    root.style.setProperty('--bg', '#F5F5F7'); root.style.setProperty('--bg-card', 'rgba(255,255,255,0.92)')
    root.style.setProperty('--bg-sidebar', 'rgba(255,255,255,0.84)'); root.style.setProperty('--bg-player', 'rgba(255,255,255,0.95)')
    root.style.setProperty('--bg-input', 'rgba(0,0,0,0.06)'); root.style.setProperty('--bg-hover', 'rgba(0,0,0,0.04)')
    root.style.setProperty('--bg-active', 'rgba(0,0,0,0.06)'); root.style.setProperty('--text', '#1c1c1e')
    root.style.setProperty('--text-sub', '#3a3a3c'); root.style.setProperty('--text-muted', '#8e8e93')
    root.style.setProperty('--list-text', '#1c1c1e'); root.style.setProperty('--list-text-sub', '#3a3a3c')
    root.style.setProperty('--panel-text', '#000000'); root.style.setProperty('--panel-line', 'rgba(0,0,0,0.2)')
    root.style.setProperty('--lyrics-text', '#000000'); root.style.setProperty('--lyrics-text-sub', '#000000')
    root.style.setProperty('--lyrics-empty', '#000000')
    root.style.setProperty('--lyrics-border', '#000000')
    root.style.setProperty('--border', 'rgba(0,0,0,0.1)'); root.style.setProperty('--modal-bg', 'rgba(252,252,255,0.98)')
    root.style.setProperty('--modal-overlay', 'rgba(0,0,0,0.35)')
  }
  // Sidebar opacity
  const sidebarEl = $('sidebar'), titlebarEl = $('titlebar'), playerBarEl = $('player-bar')
  const sidebarAlpha = (S.sidebarOpacity ?? 100) / 100
  const sbBg = isDark ? `rgba(16,16,22,${sidebarAlpha})` : `rgba(245,245,247,${sidebarAlpha})`
  sidebarEl.style.backgroundColor = sbBg
  sidebarEl.style.borderRight = `1.5px solid ${isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)'}`
  // Titlebar opacity
  const titlebarAlpha = (S.titlebarOpacity ?? 100) / 100
  const tbBg = isDark ? `rgba(16,16,22,${titlebarAlpha})` : `rgba(255,255,255,${titlebarAlpha})`
  titlebarEl.style.backgroundColor = tbBg
  // Player bar opacity
  const playerAlpha = (S.playerOpacity ?? 100) / 100
  const pbBg = isDark ? `rgba(20,20,28,${playerAlpha})` : `rgba(255,255,255,${playerAlpha})`
  playerBarEl.style.backgroundColor = pbBg
  const panelContent = document.querySelector('.panel-content')
  if (panelContent) panelContent.style.backgroundColor = pbBg
}

function apThBg() {
  if (!S.bgData) { $('bg-img').removeAttribute('src'); $('bg-layer').style.opacity = 1; $('bg-layer').style.filter = ''; return }
  const bgEl = $('bg-layer'), bgImg = $('bg-img')
  bgImg.src = S.bgData
  bgEl.style.opacity = S.ovl / 100
  bgEl.style.filter = `blur(${S.bgBlur || 0}px)`
  const vw = window.innerWidth, vh = window.innerHeight
  if (S._imgEditState && (S._imgEditState.zoomPct || S._imgEditState.zoom)) {
    if (!S._imgEditState.zoomPct && S._imgEditState.zoom) { S._imgEditState.zoomPct = S._imgEditState.zoom; delete S._imgEditState.zoom }
    const z = S._imgEditState.zoomPct / 100
    const natW = S._imgEditState.natW || bgImg.naturalWidth || vw
    const natH = S._imgEditState.natH || bgImg.naturalHeight || vh
    const fillScale = Math.max(vw / natW, vh / natH)
    const scale = fillScale * z
    const imgW = natW * scale, imgH = natH * scale
    bgImg.style.width = imgW + 'px'
    bgImg.style.height = imgH + 'px'
    const savedVW = S._imgEditState.vw || vw
    const savedVH = S._imgEditState.vh || vh
    const offX = (S._imgEditState.posX || 0) * (vw / savedVW)
    const offY = (S._imgEditState.posY || 0) * (vh / savedVH)
    bgImg.style.left = ((vw - imgW) / 2 + offX) + 'px'
    bgImg.style.top = ((vh - imgH) / 2 + offY) + 'px'
  } else {
    const iw = bgImg.naturalWidth || vw, ih = bgImg.naturalHeight || vh
    const scale = S.bgSize === 'contain' ? Math.min(vw / iw, vh / ih) : Math.max(vw / iw, vh / ih)
    const sw = iw * scale, sh = ih * scale
    bgImg.style.width = sw + 'px'; bgImg.style.height = sh + 'px'
    bgImg.style.left = ((vw - sw) / 2) + 'px'
    bgImg.style.top = ((vh - sh) / 2) + 'px'
  }
  // Invalidate cached luminance colors
  S.listTextColorsCached = null
  recalcListTextColor()
}

function recalcListTextColor() {
  if (S.listTextColorsCached && S.listTextColorsCached.bgData === S.bgData) {
    applyCachedListTextColor()
    return
  }

  if (!S.bgData) {
    S.listTextColorsCached = null
    return
  }

  const img = new Image()
  img.onload = () => {
    try {
      const canvas = document.createElement('canvas')
      const ctx = canvas.getContext('2d', { willReadFrequently: true })
      if (!ctx) { applyDefaultListTextColor(); return }

      const sampleSize = 50
      canvas.width = sampleSize
      canvas.height = sampleSize
      ctx.drawImage(img, 0, 0, sampleSize, sampleSize)

      const imageData = ctx.getImageData(0, 0, sampleSize, sampleSize)
      const data = imageData.data

      let totalLuminance = 0
      let pixelCount = 0

      for (let i = 0; i < data.length; i += 4) {
        const r = data[i]
        const g = data[i + 1]
        const b = data[i + 2]
        const a = data[i + 3]

        if (a < 128) continue

        const luminance = 0.299 * r + 0.587 * g + 0.114 * b
        totalLuminance += luminance
        pixelCount++
      }

      const avgLuminance = pixelCount > 0 ? totalLuminance / pixelCount : 128

      let colors
      if (avgLuminance > 128) {
        colors = { bgData: S.bgData, '--list-text': '#000000', '--list-text-sub': '#000000', '--lyrics-text': '#000000', '--lyrics-text-sub': '#333333', '--lyrics-empty': '#000000' }
      } else {
        colors = { bgData: S.bgData, '--list-text': '#ffffff', '--list-text-sub': '#ffffff', '--lyrics-text': '#ffffff', '--lyrics-text-sub': '#cccccc', '--lyrics-empty': '#ffffff' }
      }

      S.listTextColorsCached = colors
      applyCachedListTextColor()
    } catch (e) {
      applyDefaultListTextColor()
    }
  }

  img.onerror = () => applyDefaultListTextColor()
  img.src = S.bgData
}

function applyCachedListTextColor() {
  if (S.listTextColorsCached) {
    const c = S.listTextColorsCached
    for (const [prop, val] of Object.entries(c)) {
      if (prop === 'bgData') continue
      document.documentElement.style.setProperty(prop, val)
    }
  }
}

function applyDefaultListTextColor() {
  const isDark = S.theme === 'dark'
  document.documentElement.style.setProperty('--list-text', isDark ? '#f0f0f5' : '#1c1c1e')
  document.documentElement.style.setProperty('--list-text-sub', isDark ? '#b8b8c0' : '#3a3a3c')
  document.documentElement.style.setProperty('--lyrics-text', isDark ? '#f0f0f5' : '#1c1c1e')
  document.documentElement.style.setProperty('--lyrics-text-sub', isDark ? '#b8b8c0' : '#3a3a3c')
  document.documentElement.style.setProperty('--lyrics-empty', isDark ? '#ffffff' : '#000000')
}

function apMode() {
  const icons = {
    0: { name: '\u987a\u5e8f\u5faa\u73af', show: ['mode-icon-sequential'], hide: ['mode-icon-shuffle', 'mode-icon-repeat1', 'mode-icon-once'] },
    1: { name: '\u968f\u673a\u64ad\u653e', show: ['mode-icon-shuffle'], hide: ['mode-icon-sequential', 'mode-icon-repeat1', 'mode-icon-once'] },
    2: { name: '\u5355\u66f2\u5faa\u73af', show: ['mode-icon-repeat1'], hide: ['mode-icon-sequential', 'mode-icon-shuffle', 'mode-icon-once'] },
    3: { name: '\u64ad\u5b8c\u505c\u6b62', show: ['mode-icon-once'], hide: ['mode-icon-sequential', 'mode-icon-shuffle', 'mode-icon-repeat1'] }
  }
  const cfg = icons[S.mode] || icons[0]
  $('btn-mode').title = cfg.name
  cfg.show.forEach(id => { $(id).style.display = '' })
  cfg.hide.forEach(id => { $(id).style.display = 'none' })
}

// === Color ===
function hex2rgb(h) { const v = parseInt(h.slice(1), 16); return [(v >> 16) & 255, (v >> 8) & 255, v & 255] }
function rgb2hsl(r, g, b) {
  r /= 255; g /= 255; b /= 255; const M = Math.max(r, g, b), m = Math.min(r, g, b), d = M - m, l = (M + m) / 2
  let h = 0, s = d === 0 ? 0 : l > 0.5 ? d / (2 - M - m) : d / (M + m)
  if (d !== 0) { if (M === r) h = ((g - b) / d + (g < b ? 6 : 0)) * 60; else if (M === g) h = ((b - r) / d + 2) * 60; else h = ((r - g) / d + 4) * 60 }
  return { h: Math.round(h), s: Math.round(s * 100), l: Math.round(l * 100) }
}
function h2hsl(hex) { const [r, g, b] = hex2rgb(hex); return rgb2hsl(r, g, b) }

function hsvToRgb(h, s, v) {
  s /= 100; v /= 100;
  const c = v * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = v - c;
  let r, g, b;
  if (h < 60)       [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else              [r, g, b] = [c, 0, x];
  return [
    Math.round((r + m) * 255),
    Math.round((g + m) * 255),
    Math.round((b + m) * 255)
  ];
}

// === Canvas Color Picker (HSV ring+square, following zhling.html) ===
let cpCanvas, cpCtx
let cpHue = 0, cpSat = 100, cpVal = 100
let cpDraggingHue = false, cpDraggingBox = false

const CP_SIZE = 260
const CP_CENTER = CP_SIZE / 2
const CP_OUTER_RADIUS = 115
const CP_INNER_RADIUS = 82
const CP_BOX_SIZE = 100

function cpDrawHueRing() {
  cpCtx.save()
  cpCtx.translate(CP_CENTER, CP_CENTER)
  cpCtx.rotate(-Math.PI / 2)
  for (let angle = 0; angle < 360; angle++) {
    const start = (angle - 1) * Math.PI / 180
    const end = angle * Math.PI / 180
    cpCtx.beginPath()
    cpCtx.arc(0, 0, CP_OUTER_RADIUS, start, end)
    cpCtx.arc(0, 0, CP_INNER_RADIUS, end, start, true)
    cpCtx.closePath()
    cpCtx.fillStyle = `hsl(${angle}, 100%, 50%)`
    cpCtx.fill()
  }
  cpCtx.restore()
}

function cpDrawSVBox() {
  const x = CP_CENTER - CP_BOX_SIZE / 2
  const y = CP_CENTER - CP_BOX_SIZE / 2
  const img = cpCtx.createImageData(CP_BOX_SIZE, CP_BOX_SIZE)
  const [r0, g0, b0] = hsvToRgb(cpHue, 100, 100)
  for (let row = 0; row < CP_BOX_SIZE; row++) {
    for (let col = 0; col < CP_BOX_SIZE; col++) {
      const s = col / CP_BOX_SIZE
      const v = 1 - row / CP_BOX_SIZE
      const r = Math.round((r0 * s + 255 * (1 - s)) * v)
      const g = Math.round((g0 * s + 255 * (1 - s)) * v)
      const b = Math.round((b0 * s + 255 * (1 - s)) * v)
      const idx = (row * CP_BOX_SIZE + col) * 4
      img.data[idx] = r; img.data[idx + 1] = g; img.data[idx + 2] = b; img.data[idx + 3] = 255
    }
  }
  cpCtx.putImageData(img, x, y)
}

function cpDrawIndicators() {
  // Hue indicator
  const hueAngle = (cpHue - 90) * Math.PI / 180
  const ringR = (CP_OUTER_RADIUS + CP_INNER_RADIUS) / 2
  const hx = CP_CENTER + Math.cos(hueAngle) * ringR
  const hy = CP_CENTER + Math.sin(hueAngle) * ringR
  cpCtx.beginPath()
  cpCtx.arc(hx, hy, 7, 0, Math.PI * 2)
  cpCtx.strokeStyle = '#fff'; cpCtx.lineWidth = 3; cpCtx.stroke()
  cpCtx.fillStyle = `hsl(${cpHue}, 100%, 50%)`; cpCtx.fill()

  // SV box indicator
  const bx = CP_CENTER - CP_BOX_SIZE / 2
  const by = CP_CENTER - CP_BOX_SIZE / 2
  const sx = bx + (cpSat / 100) * CP_BOX_SIZE
  const sy = by + ((100 - cpVal) / 100) * CP_BOX_SIZE
  const [cr, cg, cb] = hsvToRgb(cpHue, cpSat, cpVal)
  cpCtx.beginPath()
  cpCtx.arc(sx, sy, 6, 0, Math.PI * 2)
  cpCtx.strokeStyle = cpVal > 55 ? '#000' : '#fff'
  cpCtx.lineWidth = 2; cpCtx.stroke()
  cpCtx.fillStyle = `rgb(${cr},${cg},${cb})`; cpCtx.fill()
}

function cpDraw() {
  cpCtx.clearRect(0, 0, CP_SIZE, CP_SIZE)
  cpDrawHueRing()
  cpDrawSVBox()
  cpDrawIndicators()
}

function cpGetPos(e) {
  const canvas = $('cp-canvas')
  const rect = canvas.getBoundingClientRect()
  const cx = e.touches ? e.touches[0].clientX : e.clientX
  const cy = e.touches ? e.touches[0].clientY : e.clientY
  return {
    x: (cx - rect.left) * (CP_SIZE / rect.width),
    y: (cy - rect.top) * (CP_SIZE / rect.height)
  }
}

function cpGetTarget(x, y) {
  const dx = x - CP_CENTER, dy = y - CP_CENTER
  const dist = Math.sqrt(dx * dx + dy * dy)
  if (dist >= CP_INNER_RADIUS - 8 && dist <= CP_OUTER_RADIUS + 8) return 'hue'
  const bx = CP_CENTER - CP_BOX_SIZE / 2
  const by = CP_CENTER - CP_BOX_SIZE / 2
  if (x >= bx && x <= bx + CP_BOX_SIZE && y >= by && y <= by + CP_BOX_SIZE) return 'box'
  return null
}

function cpUpdateHue(x, y) {
  const dx = x - CP_CENTER, dy = y - CP_CENTER
  let angle = Math.atan2(dy, dx) * 180 / Math.PI
  cpHue = (angle + 90 + 360) % 360
  cpApplyColor()
}

function cpUpdateBox(x, y) {
  const bx = CP_CENTER - CP_BOX_SIZE / 2
  const by = CP_CENTER - CP_BOX_SIZE / 2
  cpSat = Math.max(0, Math.min(100, ((x - bx) / CP_BOX_SIZE) * 100))
  cpVal = Math.max(0, Math.min(100, (1 - (y - by) / CP_BOX_SIZE) * 100))
  cpApplyColor()
}

function cpApplyColor() {
  const [r, g, b] = hsvToRgb(cpHue, cpSat, cpVal)
  S.clr = '#' + [r, g, b].map(v => v.toString(16).padStart(2, '0')).join('')
  cpDraw(); apTh(); schedSave()
}

function cpHandleStart(e) {
  e.preventDefault()
  const p = cpGetPos(e)
  const t = cpGetTarget(p.x, p.y)
  if (t === 'hue') { cpDraggingHue = true; cpUpdateHue(p.x, p.y) }
  if (t === 'box') { cpDraggingBox = true; cpUpdateBox(p.x, p.y) }
}

function cpHandleMove(e) {
  if (!cpDraggingHue && !cpDraggingBox) return
  e.preventDefault()
  const p = cpGetPos(e)
  if (cpDraggingHue) cpUpdateHue(p.x, p.y)
  if (cpDraggingBox) cpUpdateBox(p.x, p.y)
}

function cpHandleEnd() { cpDraggingHue = false; cpDraggingBox = false }

function initColorPicker() {
  const canvas = $('cp-canvas')
  if (!canvas) return
  cpCanvas = canvas
  cpCtx = canvas.getContext('2d', { willReadFrequently: true })
  canvas.width = CP_SIZE; canvas.height = CP_SIZE
  canvas.removeEventListener('mousedown', cpHandleStart)
  canvas.removeEventListener('mousemove', cpHandleMove)
  window.removeEventListener('mouseup', cpHandleEnd)
  canvas.removeEventListener('touchstart', cpHandleStart)
  canvas.removeEventListener('touchmove', cpHandleMove)
  window.removeEventListener('touchend', cpHandleEnd)
  canvas.addEventListener('mousedown', cpHandleStart)
  canvas.addEventListener('mousemove', cpHandleMove)
  window.addEventListener('mouseup', cpHandleEnd)
  canvas.addEventListener('touchstart', cpHandleStart, { passive: false })
  canvas.addEventListener('touchmove', cpHandleMove, { passive: false })
  window.addEventListener('touchend', cpHandleEnd)
  const hsl = h2hsl(S.clr)
  const s = hsl.s / 100, l = hsl.l / 100
  const v = l + s * Math.min(l, 1 - l)
  const sat = v === 0 ? 0 : 2 * (1 - l / v) * 100
  cpHue = hsl.h; cpSat = Math.round(sat); cpVal = Math.round(v * 100)
  cpDraw()
}

function cpSyncFromState() {
  const hsl = h2hsl(S.clr)
  const s = hsl.s / 100, l = hsl.l / 100
  const v = l + s * Math.min(l, 1 - l)
  const sat = v === 0 ? 0 : 2 * (1 - l / v) * 100
  cpHue = hsl.h; cpSat = Math.round(sat); cpVal = Math.round(v * 100)
  if (cpCtx) cpDraw()
}
function updSUI() {
  const themeDarkEl = $('theme-dark'), themeLightEl = $('theme-light')
  const sbOpacityEl = $('sidebar-opacity'), sbOpacityValEl = $('sidebar-opacity-val')
  const tbOpacityEl = $('titlebar-opacity'), tbOpacityValEl = $('titlebar-opacity-val')
  const plOpacityEl = $('player-opacity'), plOpacityValEl = $('player-opacity-val')
  const bgPreviewEl = $('bg-preview'), bgPreviewWrapEl = $('bg-preview-wrap'), btnBgUploadEl = $('btn-bg-upload')
  themeDarkEl.classList.toggle('active', S.theme === 'dark'); themeLightEl.classList.toggle('active', S.theme === 'light')
  sbOpacityEl.value = S.sidebarOpacity ?? 100; sbOpacityValEl.textContent = (S.sidebarOpacity ?? 100) + '%'
  tbOpacityEl.value = S.titlebarOpacity ?? 100; tbOpacityValEl.textContent = (S.titlebarOpacity ?? 100) + '%'
  plOpacityEl.value = S.playerOpacity ?? 100; plOpacityValEl.textContent = (S.playerOpacity ?? 100) + '%'
  if (S.bgData) { bgPreviewEl.style.backgroundImage = `url(${S.bgData})`; bgPreviewWrapEl.classList.remove('hidden'); btnBgUploadEl.classList.add('hidden') } else { bgPreviewWrapEl.classList.add('hidden'); btnBgUploadEl.classList.remove('hidden') }
  document.querySelectorAll('.cp-preset').forEach(b => b.classList.toggle('active', b.dataset.clr === S.clr))
}

// === Audio ===
async function loadT(idx) {
  if (idx < 0 || idx >= pl.length) return
  const gen = ++_loadTGeneration
  nI = idx
  const t = pl[idx]
  stopDsdPlayback(false)
  if (isDsdTrack(t)) {
    audio.pause()
    audio.removeAttribute('src')
    audio.load()
    await loadLrcForTrack(t)
    if (gen !== _loadTGeneration) return
    await playDsdTrack(t, 0).catch(e => { console.error('DSD playback failed:', e); const tid = addT('DSD 播放失败'); updT(tid, '错误', 0, e.message); rmT(tid) })
    if (gen !== _loadTGeneration) return
    S.tI = idx; S.playing = true
    return
  }
  audio.src = 'file:///' + t.path.replace(/\\/g, '/')
  await loadLrcForTrack(t)
  if (gen !== _loadTGeneration) return
  if (S.devId && audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
  await audio.play().catch(() => { /* ignore */ })
  if (gen !== _loadTGeneration) return
  S.tI = idx; S.playing = true
}

async function loadLrcForTrack(t) {
  lrc = []
  _lastLrcActiveIdx = -1
  try {
    const sep = t.path.includes('\\') ? '\\' : '/'
    const lastSep = t.path.lastIndexOf(sep)
    const dir = lastSep >= 0 ? t.path.substring(0, lastSep) : ''
    const baseName = lastSep >= 0 ? t.path.substring(lastSep + 1) : t.path
    const lastDot = baseName.lastIndexOf('.')
    const nameWithoutExt = lastDot >= 0 ? baseName.substring(0, lastDot) : baseName
    const ext = lastDot >= 0 ? baseName.substring(lastDot).toLowerCase() : ''

    // Try LRC first, then VTT, then SRT
    let lyricsContent = null
    let subtitleFormat = null // null = LRC, 'vtt', 'srt'

    const lrcPath = dir + sep + nameWithoutExt + '.lrc'
    if (await api.fileExists(lrcPath)) {
      lyricsContent = await api.readTextFile(lrcPath)
    }

    // If no LRC, try VTT and SRT files
    if (!lyricsContent) {
      const subtitleExts = ['.vtt', '.srt']
      for (const subExt of subtitleExts) {
        // For audio files like song.mp3, try song.mp3.vtt (double extension pattern)
        const doublePath = dir + sep + baseName + subExt
        if (await api.fileExists(doublePath)) {
          lyricsContent = await api.readTextFile(doublePath)
          subtitleFormat = subExt.slice(1) // 'vtt' or 'srt'
          break
        }
        // Also try song.vtt (single extension)
        const singlePath = dir + sep + nameWithoutExt + subExt
        if (await api.fileExists(singlePath)) {
          lyricsContent = await api.readTextFile(singlePath)
          subtitleFormat = subExt.slice(1)
          break
        }
      }
    }

    if (!lyricsContent) return

    if (subtitleFormat === 'vtt' || subtitleFormat === 'srt') {
      // Parse WebVTT / SRT format
      const lines = lyricsContent.split(/\r?\n/)
      // VTT uses period: 00:00:21.813, SRT uses comma: 00:00:21,813
      const timestampRegex = /(\d{2}):(\d{2}):(\d{2})[.,](\d{3})\s*-->\s*\d{2}:\d{2}:\d{2}[.,]\d{3}/
      const seen = new Map()
      let i = 0
      // Skip WEBVTT header
      if (subtitleFormat === 'vtt' && lines[0] && lines[0].trim().toUpperCase().startsWith('WEBVTT')) i = 1
      for (; i < lines.length; i++) {
        const line = lines[i]
        const tsMatch = line.match(timestampRegex)
        if (!tsMatch) continue
        const hours = parseInt(tsMatch[1])
        const min = parseInt(tsMatch[2])
        const sec = parseInt(tsMatch[3])
        const ms = parseInt(tsMatch[4])
        const time = hours * 3600 + min * 60 + sec + ms / 1000
        // Next non-empty line is the text
        let text = ''
        for (let j = i + 1; j < lines.length; j++) {
          const nextLine = lines[j].trim()
          if (!nextLine) break
          // Skip cue identifiers (numeric-only lines)
          if (/^\d+$/.test(nextLine)) continue
          text = nextLine
          break
        }
        if (text && !seen.has(time)) {
          seen.set(time, text)
        }
      }
      lrc = [...seen.entries()].map(([time, text]) => ({ time, text }))
      lrc.sort((a, b) => a.time - b.time)
    } else {
      // Parse LRC format
      const lines = lyricsContent.split(/\r?\n/)
      const timeRegex = /\[(\d{2}):(\d{2})(?:\.(\d{2,3}))?\]/g
      const textRegex = /\[\d{2}:\d{2}(?:\.\d{2,3})?\]/g
      const seen = new Map()
      for (const line of lines) {
        const timestamps = [...line.matchAll(timeRegex)]
        if (timestamps.length === 0) continue
        const text = line.replace(textRegex, '').trim()
        if (!text) continue
        for (const ts of timestamps) {
          const min = parseInt(ts[1])
          const sec = parseInt(ts[2])
          const ms = ts[3] ? parseInt(ts[3].padEnd(3, '0')) : 0
          const time = min * 60 + sec + ms / 1000
          if (!seen.has(time)) {
            seen.set(time, text)
          }
        }
      }
      lrc = [...seen.entries()].map(([time, text]) => ({ time, text }))
      lrc.sort((a, b) => a.time - b.time)
    }
  } catch (e) { /* ignore */ }
}

function playT(idx, keepView) {
  if (idx < 0 || idx >= pl.length) return
  const t = pl[idx]
  S.playingTid = t.id
  if (!S.recents.includes(t.id)) { S.recents.unshift(t.id); if (S.recents.length > 200) S.recents.length = 200 }
  if (!keepView && S.view !== 'lyrics') {
    S.prevView = S.view
    S.view = 'lyrics'
    activeLrcTab = 'lyrics'
  }
  updPUI(t, true)
  const oldTI = S.tI
  S.tI = idx
  const needLoad = isDsdTrack(t)
    ? (idx !== oldTI) || !dsdState.active || dsdState.path !== t.path
    : (idx !== oldTI) || dsdState.active || (audio.src === '' || !audio.src.includes(t.path.replace(/\\/g, '/')))
  if (needLoad) {
    loadT(idx).then(() => {
      if (S.view === 'lyrics') renderContent()
    })
    // Skip synchronous render with stale lyrics; wait for loadT to finish.
  } else {
    renderContent()
  }
  S.playing = true; updPlayBtn(); invalidateVL('vl-songs'); syncAllListsPlaying(); renderPanel(); schedSave()
}

function updPUI(t, skipLrc) {
  const cd = t.coverData || t.albumCoverData
  const titleEl = $('player-title'), artistEl = $('player-artist')
  const coverEl = $('player-cover'), coverImgEl = $('player-cover-img')
  titleEl.textContent = t.name
  const artist = t.metaArtist || t.artist || '\u4f5a\u540d'
  const isVid = isVideoFile(t)
  artistEl.textContent = artist + (isVid ? ' \u00b7 \u89c6\u9891-\u4ec5\u97f3\u9891\u6a21\u5f0f' : '')
  if (cd) {
    coverImgEl.src = cd; coverImgEl.style.display = ''
    coverEl.querySelector('.cover-placeholder').style.display = 'none'
  } else {
    coverImgEl.style.display = 'none'; coverEl.querySelector('.cover-placeholder').style.display = ''
    const ph = coverEl.querySelector('.cover-placeholder svg')
    if (ph) {
      if (isVid) {
        ph.outerHTML = '<svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="20" height="18" rx="2"/><polygon points="10,8 16,12 10,16"/></svg>'
      } else {
        ph.outerHTML = '<svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>'
      }
    }
  }
  if (!skipLrc && S.view === 'lyrics') renderLrcContent()
}

function hEnd() {
  S.cTime = getPlaybackCurrentTime(); S.dur = getPlaybackDuration()
  if (isNaN(S.dur)) return
  if (S.mode === 2) {
    if (isCurrentTrackDsd()) playT(S.tI, true)
    else { audio.currentTime = 0; audio.play().catch(() => { /* ignore */ }) }
  }
  else if (S.mode === 3) { S.playing = false; updPlayBtn(); updPUI(pl[S.tI]); syncAllListsPlaying() }
  else nxt()
}

function nxt() {
  if (pl.length === 0) return
  if (S.mode === 1) {
    if (pl.length === 1) { playT(0, true); return }
    let r; do { r = Math.floor(Math.random() * pl.length) } while (r === S.tI)
    playT(r, true); return
  }
  playT((S.tI + 1) % pl.length, true)
}
function prv() { if (pl.length === 0) return; playT(S.tI <= 0 ? pl.length - 1 : S.tI - 1, true) }

// === Player UI ===
function updPlayBtn() { $('icon-play').style.display = S.playing ? 'none' : ''; $('icon-pause').style.display = S.playing ? '' : 'none' }

// === Render ===
function renderAll() {
  invalidateFavCache()
  renderSB(); renderContent()
  renderPanel(); updPlayBtn(); syncAllListsPlaying(); schedSave()
}

function renderSB() {
  const folderListEl = $('folder-list')
  const favListEl = $('fav-list')
  const plListEl = $('playlist-list')
  // Build folder HTML in one pass instead of multiple insertAdjacentHTML
  let folderHtml = ''
  if (fp.length && S.folderTree.length) {
    const treeRootPaths = new Set(S.folderTree.map(n => n.path.replace(/\\/g, '/')))
    for (const p of fp) {
      const np = p.replace(/\\/g, '/')
      if (!treeRootPaths.has(np)) continue
      const top = p.replace(/\\/g, '/').replace(/\/$/, '').split('/').pop() || p
      const isActive = S.activeFp === p
      folderHtml += `<button class="folder-item${isActive ? ' active' : ''}" data-fp="${esc(p)}"><svg class="folder-icon" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg><span class="folder-name">${esc(top)}</span></button>`
    }
  }
  folderListEl.innerHTML = folderHtml
  favListEl.innerHTML = S.favs.map(f => `<button class="fav-sidebar-item${S.aF === f.id ? ' active' : ''}" data-fvid="${f.id}" title="${f.isDefault ? '\u9ed8\u8ba4\u6536\u85cf\u5939' : '\u53cc\u51fb\u91cd\u547d\u540d'}"><span class="fav-sidebar-name">${esc(f.name)}</span><span class="fav-sidebar-count">${f.trackIds.length}</span></button>`).join('')
  plListEl.innerHTML = S.pls.map(p => `<button class="playlist-sidebar-item${S.aPl === p.id ? ' active' : ''}" data-plid="${p.id}" title="\u53cc\u51fb\u91cd\u547d\u540d"><span class="pl-sidebar-name">${esc(p.name)}</span><span class="pl-sidebar-count">${p.trackIds.length}</span></button>`).join('')
  const favSection = favListEl.closest('.nav-section')
  if (favSection) favSection.classList.toggle('empty', S.favs.length === 0)
  const plSection = plListEl.closest('.nav-section')
  if (plSection) plSection.classList.toggle('empty', S.pls.length === 0)
  const navs = document.querySelectorAll('#sidebar-nav .nav-item')
  navs.forEach(n => {
    n.classList.remove('active')
    if (n.dataset.view === S.view && !S.activeFp) n.classList.add('active')
  })
}

function renderContent() {
  const bc = $('breadcrumb'), ca = $('content-area')
  syncPlayingState()
  const main = $('content')
  if (main) main.classList.toggle('locked', S.view === 'lyrics')
  if (S.view === 'lyrics') { renderLyricsFullView(); return }
  if (S.aF) {
    const fav = S.favs.find(f => f.id === S.aF)
    if (fav) {
      bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-fav-back">← 返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">${esc(fav.name)}</button>`
      const tks = fav.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean)
      pl = tks
      ca.innerHTML = renderFContent(fav, tks); return
    }
  }
  if (S.aPl) {
    const plObj = S.pls.find(p => p.id === S.aPl)
    if (plObj) {
      bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-pl-back">← 返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">${esc(plObj.name)}</button>`
      const tks = plObj.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean)
      pl = tks
      ca.innerHTML = renderPContent(plObj, tks); return
    }
  }
  if (S.view === 'all') { renderFolderAll(); return }
  if (S.view === 'recent') { S.prevView = null; renderRecentView(); return }
  if (S.view === 'tools') { S.prevView = null; renderToolsContent(); return }
}

function _folderMtime(node) {
  const meta = S._folderMeta || {}
  const nMeta = meta[node.path] || {}
  let latest = 0
  if (nMeta.hasMusic) {
    for (const t of (node.tracks || [])) { if (t.fileMtime > latest) latest = t.fileMtime }
    for (const c of (node.children || [])) {
      const cm = _folderMtime(c)
      if (cm > latest) latest = cm
    }
  }
  return latest
}

function sortFolders(arr) {
  const mode = S.folderSort || 'name'
  const sorted = [...arr]
  if (mode === 'time') {
    sorted.sort((a, b) => _folderMtime(b) - _folderMtime(a))
  } else if (mode === 'tracks') {
    const meta = S._folderMeta || {}
    sorted.sort((a, b) => ((meta[b.path] || {}).trackCount || 0) - ((meta[a.path] || {}).trackCount || 0))
  } else {
    sorted.sort((a, b) => a.name.localeCompare(b.name))
  }
  return sorted
}

function folderSortBtnHTML() {
  const modes = [['name', '名称'], ['time', '最近修改'], ['tracks', '曲目数']]
  const isList = S.folderView === 'list'
  return `<div class="folder-sort-bar">${modes.map(([k, label]) => `<button class="folder-sort-btn${S.folderSort === k ? ' active' : ''}" data-fsort="${k}">${label}</button>`).join('')}<div class="folder-view-toggle"><button class="folder-sort-btn${!isList ? ' active' : ''}" data-fview="grid" title="网格视图"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/></svg></button><button class="folder-sort-btn${isList ? ' active' : ''}" data-fview="list" title="列表视图"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg></button></div></div>`
}

function folderListRowHTML(n) {
  const meta = S._folderMeta || {}
  const nMeta = meta[n.path] || {}
  const coverBg = nMeta.coverData
  const trackCount = nMeta.trackCount || n.trackCount || 0
  const validChildCount = nMeta.validChildCount ?? n.children.filter(c => hasMusicRecursive(c)).length
  const subtitle = trackCount ? `${trackCount} 首${n.children.length ? ` · ${validChildCount} 子文件夹` : ''}` : `${validChildCount} 个子文件夹`
  return `<div class="folder-list-row" data-fp="${esc(n.path)}"><div class="folder-list-cover">${coverBg ? `<img src="${coverBg}" alt="" />` : '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>'}</div><div class="folder-list-info"><div class="folder-list-name">${esc(n.name)}</div><div class="folder-list-sub">${subtitle}</div></div></div>`
}

function renderFolderAll() {
  const bc = $('breadcrumb'), ca = $('content-area')
  if (S.q) {
    bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-search-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">搜索结果</button>`
    if (!pl.length && !S._searchFolders.length) { ca.innerHTML = emptyS('未找到匹配的内容', '请尝试其他搜索关键词', false); return }
    let html = ''
    if (S._searchFolders.length) {
      const isList = S.folderView === 'list'
      const folderHtml = isList ? S._searchFolders.map(n => folderListRowHTML(n)).join('') : S._searchFolders.map(n => folderCardHTML(n)).join('')
      const containerCls = isList ? 'folder-list' : 'artist-grid'
      html += `<div class="section-title">文件夹<span>${S._searchFolders.length} 个</span></div><div class="${containerCls}">${folderHtml}</div>`
    }
    if (pl.length) {
      if (html) html += '<div style="height:20px"></div>'
      html += `<div class="section-title">音乐<span>${pl.length} 首</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="search"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>播放全部</button>${tableVT('vl-search', pl, (idx) => playT(idx, true))}`
    }
    ca.innerHTML = html
    return
  }
  const tree = S.folderTree || []
  const meta = S._folderMeta || {}
  if (!tree.length || !tree.some(n => meta[n.path]?.hasMusic)) {
    ca.innerHTML = emptyS('\u8fd8\u6ca1\u6709\u97f3\u4e50\u6587\u4ef6\u5939', '\u70b9\u51fb\u5bfc\u5165\u6587\u4ef6\u5939\u5f00\u59cb', true)
    bc.innerHTML = `<button class="breadcrumb-item current">\u5168\u90e8\u97f3\u4e50</button>`
    return
  }
  if (S.folderStack.length === 0) {
    bc.innerHTML = `<button class="breadcrumb-item current">\u5168\u90e8\u97f3\u4e50</button>`
    const validRoots = sortFolders(tree.filter(n => meta[n.path]?.hasMusic))
    const isList = S.folderView === 'list'
    const html = isList ? validRoots.map(n => folderListRowHTML(n)).join('') : validRoots.map(n => folderCardHTML(n)).join('')
    const containerCls = isList ? 'folder-list' : 'artist-grid'
    ca.innerHTML = `<div class="section-title">\u6587\u4ef6\u5939<span>${validRoots.length} \u4e2a\u6587\u4ef6\u5939</span></div>${folderSortBtnHTML()}<div class="${containerCls}">${html}</div>`
    return
  }
  const node = findNodeByPath(tree, S.folderStack[S.folderStack.length - 1])
  if (!node) { S.folderStack = []; renderFolderAll(); return }
  renderFolderNode(node)
}

function findCoverInNode(n) {
  if (n.coverData) return n.coverData
  for (const t of (n.tracks || [])) { if (t.coverData) return t.coverData }
  // Children are sorted by name; recurse in order
  for (const c of (n.children || []).slice().sort((a, b) => a.name.localeCompare(b.name))) { const r = findCoverInNode(c); if (r) return r }
  return null
}

function folderCardHTML(n) {
  const meta = S._folderMeta || {}
  const nMeta = meta[n.path] || {}
  const coverBg = nMeta.coverData
  const trackCount = nMeta.trackCount || n.trackCount || 0
  const validChildCount = nMeta.validChildCount ?? n.children.filter(c => hasMusicRecursive(c)).length
  const subtitle = trackCount ? `${trackCount} \u9996\u97f3\u4e50${n.children.length ? ` \u00b7 ${validChildCount} \u5b50\u6587\u4ef6\u5939` : ''}` : `${validChildCount} \u4e2a\u5b50\u6587\u4ef6\u5939`
  return `<div class="card folder-card" data-fp="${esc(n.path)}"><div class="card-cover folder-card-cover">${coverBg ? `<img src="${coverBg}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg></div>'}</div><div class="card-body"><div class="card-title">${esc(n.name)}</div><div class="card-subtitle">${subtitle}</div></div></div>`
}

function tableH(tracks) {
  const cols = S.selMode ? '32px 40px 1.2fr 0.9fr 0.9fr 40px 60px 48px' : '40px 1.2fr 0.9fr 0.9fr 40px 60px 48px'
  const checks = S.selMode ?
    `<div class="song-row-header" style="grid-template-columns:${cols}"><div class="song-row-check"></div><div>#</div><div>\u6807\u9898</div><div>\u827a\u672f\u5bb6</div><div>\u4e13\u8f91</div><div></div><div>\u65f6\u957f</div><div></div></div>` : ''
  const items = tracks.map((t, i) => {
    const isPlaying = S.playingTid && t.id === S.playingTid
    const playState = isPlaying ? (S.playing ? 'is-playing-state' : 'is-paused-state') : ''
    const isVid = isVideoFile(t)
    const liked = isDefaultFavTrack(t.id)
    return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}">${S.selMode ? `<div class="song-row-check"><input type="checkbox" data-tid="${t.id}" /></div>` : ''}<div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(t.name)}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${liked ? ' liked' : ''}" data-tid="${t.id}">${liked ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
  }).join('')
  return `<div class="song-table">${checks}${items}</div>`
}

function _trackRowHTML(t, i, cols) {
  const isPlaying = S.playingTid && t.id === S.playingTid
  const playState = isPlaying ? (S.playing ? 'is-playing-state' : 'is-paused-state') : ''
  const isVid = isVideoFile(t)
  const liked = isDefaultFavTrack(t.id)
  return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}">${S.selMode ? `<div class="song-row-check"><input type="checkbox" data-tid="${t.id}" /></div>` : ''}<div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(t.name)}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${liked ? ' liked' : ''}" data-tid="${t.id}">${liked ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
}

function tableVT(containerId, tracks, onClick) {
  const cols = S.selMode ? '32px 40px 1.2fr 0.9fr 0.9fr 40px 60px 48px' : '40px 1.2fr 0.9fr 0.9fr 40px 60px 48px'
  const header = S.selMode ?
    `<div class="song-row-header" style="grid-template-columns:${cols}"><div class="song-row-check"></div><div>#</div><div>\u6807\u9898</div><div>\u827a\u672f\u5bb6</div><div>\u4e13\u8f91</div><div></div><div>\u65f6\u957f</div><div></div></div>` : ''
  const html = `<div class="song-table">${header}<div class="vl-container" id="${containerId}"></div></div>`
  requestAnimationFrame(() => {
    if (!tracks.length) {
      const c = $(containerId)
      if (c) c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u6682\u65e0\u5185\u5bb9</h3></div>'
      return
    }
    virtualList(containerId, tracks, 46, (t, i) => _trackRowHTML(t, i, cols), (tid, keepView) => { if (onClick) { const idx = tracks.findIndex(tk => tk.id === tid); if (idx >= 0) onClick(idx, keepView) } })
  })
  return html
}

function emptyS(title, desc, btn) {
  return `<div class="empty-state"><div class="empty-state-icon"><svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div><h3>${title}</h3><p>${desc}</p>${btn ? `<button class="btn-primary" id="empty-import"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>\u5bfc\u5165\u6587\u4ef6\u5939</button>` : ''}</div>`
}

// === Folder View ===
function findNodeByPath(tree, path) {
  if (!tree || !path) return null
  const np = path.replace(/\\/g, '/')
  for (const n of tree) {
    const npath = n.path.replace(/\\/g, '/')
    if (npath === np) return n
    const r = findNodeByPath(n.children, np)
    if (r) return r
  }
  return null
}

function renderFolderNode(node) {
  const bc = $('breadcrumb')
  // Build breadcrumb HTML in one pass
  let bcHtml = ''
  if (S.folderStack.length > 0) {
    bcHtml = `<button class="btn-breadcrumb-back" id="btn-folder-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span>`
  }
  bcHtml += `<button class="breadcrumb-item" data-fp-root="">\u5168\u90e8\u97f3\u4e50</button>`
  for (let i = 0; i < S.folderStack.length; i++) {
    const fp_i = S.folderStack[i]; const fn = findNodeByPath(S.folderTree, fp_i); const nm = fn ? fn.name : fp_i.split(/[\\/]/).pop()
    bcHtml += `<span class="breadcrumb-sep">/</span><button class="breadcrumb-item${i === S.folderStack.length - 1 ? ' current' : ''}" data-fp="${esc(fp_i)}">${esc(nm)}</button>`
  }
  bc.innerHTML = bcHtml
  const meta = S._folderMeta || {}
  const validChildren = sortFolders(node.children.filter(c => meta[c.path]?.hasMusic))
  let html = ''

  // Subfolder cards only (click to navigate)
  if (validChildren.length > 0) {
    const isList = S.folderView === 'list'
    const childHtml = isList ? validChildren.map(c => folderListRowHTML(c)).join('') : validChildren.map(c => folderCardHTML(c)).join('')
    const containerCls = isList ? 'folder-list' : 'artist-grid'
    html += `<div class="section-title">\u5b50\u6587\u4ef6\u5939<span>${validChildren.length} \u4e2a</span></div>${folderSortBtnHTML()}<div class="${containerCls}">${childHtml}</div>`
  }

  // Direct tracks in this folder
  if (node.tracks.length > 0) {
    const allTracks = [...node.tracks]
    pl = allTracks
    html += `<div class="section-title" style="margin-top:${validChildren.length > 0 ? '24px' : '0'}">\u97f3\u4e50<span>${allTracks.length} \u9996</span></div><button class="btn-primary" style="margin-bottom:12px" data-pfolder="${esc(node.path)}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button><div class="song-table" id="vl-songs"></div>`
  }
  $('content-area').innerHTML = html || '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u7a7a\u6587\u4ef6\u5939</h3></div>'
  if (node.tracks.length > 0) {
    virtualList('vl-songs', [...node.tracks], 46, (t, i) => {
      const cols = '40px 1.2fr 0.9fr 0.9fr 40px 60px 48px'
      const isPlaying = S.playingTid && t.id === S.playingTid
      const playState = isPlaying ? (S.playing ? 'is-playing-state' : 'is-paused-state') : ''
      const isVid = isVideoFile(t)
      const liked = isDefaultFavTrack(t.id)
      return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}"><div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(t.name)}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${liked ? ' liked' : ''}" data-tid="${t.id}">${liked ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
    }, (tid, keepView) => { const idx = pl.findIndex(t => t.id === tid); if (idx >= 0) playT(idx, keepView) })
  }
}

function navigateFolder(path) {
  const node = findNodeByPath(S.folderTree, path)
  if (!node) return
  S.activeFp = node.path
  if (S.folderStack[S.folderStack.length - 1] !== node.path) S.folderStack.push(node.path)
  S.view = 'all'; renderAll(); schedSave()
}

function navigateFolderUp() {
  if (S.folderStack.length <= 1) { S.folderStack = []; S.activeFp = null }
  else { S.folderStack.pop() }
  S.view = 'all'; renderAll(); schedSave()
}

function navigateFolderTo(path) {
  if (!path) { S.folderStack = []; S.activeFp = null }
  else {
    const idx = S.folderStack.indexOf(path)
    if (idx >= 0) { S.folderStack = S.folderStack.slice(0, idx + 1) }
    else { S.folderStack.push(path) }
    // Find the node for this path to set activeFp
    const node = findNodeByPath(S.folderTree, path)
    if (node) S.activeFp = node.path
  }
  S.view = 'all'; renderAll(); schedSave()
}

// === Recent View ===
function renderRecentView() {
  $('breadcrumb').innerHTML = `<button class="breadcrumb-item current">\u6700\u8fd1\u64ad\u653e</button>`
  const tks = S.recents.map(id => S.all.find(t => t.id === id)).filter(Boolean)
  if (!tks.length) { $('content-area').innerHTML = emptyS('\u8fd8\u6ca1\u6709\u64ad\u653e\u8bb0\u5f55', '\u5f00\u59cb\u64ad\u653e\u97f3\u4e50\u540e\u4f1a\u81ea\u52a8\u8bb0\u5f55', false); return }
  pl = tks
  $('content-area').innerHTML = `<div class="section-title">\u6700\u8fd1\u64ad\u653e<span>${tks.length} \u9996</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="recent"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button>${tableVT('vl-recent', tks, (idx) => playT(idx, true))}`
}

// === Lyrics ===
let activeLrcTab = 'lyrics'
function renderLrcContent() {
  _lastLrcActiveIdx = -1
  const t = S.playingTid ? S.all.find(x => x.id === S.playingTid) : null
  const container = $('content-area')
  if (!container) return
  const cd = t ? (t.coverData || t.albumCoverData) : null
  const lrcHtml = buildLrcLines(lrc)
  container.innerHTML = t ? `<div class="lyrics-page-actions"><button class="lyrics-action-btn" data-lact="folder">\u6240\u5728\u6587\u4ef6\u5939</button><button class="lyrics-action-btn" data-lact="copy">\u590d\u5236\u8def\u5f84</button></div><div class="lyrics-content-layout"><div class="lyrics-content-left"><div class="lyrics-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="56" height="56" fill="none" stroke="currentColor" stroke-width="1"><circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/></svg></div>'}</div></div><div class="lyrics-content-right"><div class="lyrics-content-info"><div class="lc-title">${esc(t.name)}</div><div class="lc-artist">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}${isVideoFile(t) ? ' \u00b7 \u89c6\u9891-\u4ec5\u97f3\u9891\u6a21\u5f0f' : ''}</div></div><div class="lyrics-tab-btns"><button class="lyrics-tab-btn${activeLrcTab === 'lyrics' ? ' active' : ''}" data-ltab="lyrics">\u6b4c\u8bcd</button><button class="lyrics-tab-btn${activeLrcTab === 'meta' ? ' active' : ''}" data-ltab="meta">\u4fe1\u606f</button></div><div class="lyrics-container-wrapper"><div class="lyrics-lines-scroll${activeLrcTab !== 'lyrics' ? ' hidden' : ''}" id="lyrics-lines-scroll">${lrcHtml || '<div class="lc-empty">\u6682\u65e0\u6b4c\u8bcd</div>'}</div><div class="lyrics-meta-panel${activeLrcTab !== 'meta' ? ' hidden' : ''}" id="lyrics-meta-panel"><div class="meta-row"><span class="meta-label">\u6807\u9898</span><span class="meta-value">${esc(t.name)}</span></div><div class="meta-row"><span class="meta-label">\u827a\u672f\u5bb6</span><span class="meta-value">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}</span></div><div class="meta-row"><span class="meta-label">\u4e13\u8f91</span><span class="meta-value">${esc(t.album || '')}</span></div><div class="meta-row"><span class="meta-label">\u683c\u5f0f</span><span class="meta-value">${t.format.toUpperCase()}${isVideoFile(t) ? ' (\u89c6\u9891)' : ''}</span></div><div class="meta-row"><span class="meta-label">\u65f6\u957f</span><span class="meta-value">${fmtTime(t.duration)}</span></div><div class="meta-row"><span class="meta-label">\u6587\u4ef6</span><span class="meta-value">${esc(t.path)}</span></div></div></div></div></div>` : '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u672a\u5728\u64ad\u653e</h3></div>'
  const lines = container.querySelectorAll('.lc-line')
  const scroll = $('lyrics-lines-scroll')
  if (scroll && !scroll.dataset.manualBound) {
    scroll.dataset.manualBound = '1'
    scroll.addEventListener('wheel', () => markLyricsManualScroll(), { passive: true })
    scroll.addEventListener('touchstart', () => markLyricsManualScroll(), { passive: true })
    scroll.addEventListener('pointerdown', () => markLyricsManualScroll(), { passive: true })
  }
  if (scroll && lines.length > 0) {
    const activeLine = scroll.querySelector('.lc-line.active') || lines[0]
    if (activeLine) {
      requestAnimationFrame(() => {
        const containerH = scroll.clientHeight
        const lineH = activeLine.clientHeight
        const layout = container.querySelector('.lyrics-content-layout')
        const layoutH = layout ? layout.clientHeight : containerH
        const layoutCY = layoutH / 2
        const scrollRect = scroll.getBoundingClientRect()
        const layoutRect = layout ? layout.getBoundingClientRect() : scrollRect
        const scrollTopOffset = scrollRect.top - layoutRect.top
        const scrollTarget = activeLine.offsetTop + (lineH / 2) + scrollTopOffset - layoutCY
        const maxScroll = scroll.scrollHeight - containerH
        const finalScroll = Math.max(0, Math.min(scrollTarget, maxScroll))
        scroll.scrollTo({ top: finalScroll, behavior: 'auto' })
      })
    }
  }
}

function buildLrcLines(lrcData) {
  if (!lrcData || !lrcData.length) return ''
  let activeIdx = 0
  for (let i = lrcData.length - 1; i >= 0; i--) {
    if (S.cTime >= lrcData[i].time) { activeIdx = i; break }
  }
  const html = lrcData.map((l, i) => `<div class="lc-line${i === activeIdx ? ' active' : ''}" data-lidx="${i}"><span class="lc-text">${esc(l.text)}</span><span class="lc-time">${fmtTime(l.time)}</span></div>`).join('')
  return html
}

function renderLyricsFullView() {
  $('breadcrumb').innerHTML = '<button class="btn-breadcrumb-back" id="btn-lyrics-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>\u8fd4\u56de</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">\u6b4c\u8bcd</button>'
  renderLrcContent()
}

function goBackFromLyrics() {
  if (S.prevView) { S.view = S.prevView; S.prevView = null }
  else { S.view = 'all' }
  activeLrcTab = 'lyrics'
  renderAll()
  schedSave()
}

function getDefaultFav() {
  return S.favs.find(f => f.isDefault) || null
}

function toggleDefaultFavorite(tid) {
  const fav = getDefaultFav()
  if (!fav) return
  if (fav.trackIds.includes(tid)) rFF(fav.id, tid)
  else a2F(fav.id, tid)
}

async function copyText(text, okText = '已复制') {
  if (!text) return
  try {
    const ok = await api.clipboardWriteText(text)
    if (ok) {
      const tid = addT(okText)
      updT(tid, okText, 100, '')
      rmT(tid)
    }
  } catch { /* ignore */ }
}

function markLyricsManualScroll(ms = 2200) {
  lyricsManualScrollUntil = Date.now() + ms
}

function isLyricsManualScrolling() {
  return Date.now() < lyricsManualScrollUntil
}

function clearRecents() {
  S.recents = []
  if (S.view === 'recent') renderAll()
  else schedSave()
}

function isDsdTrack(track) {
  const ext = (track?.format || '').toLowerCase()
  return ext === 'dsf' || ext === 'dff' || ext === 'dsd'
}

function isCurrentTrackDsd() {
  const track = pl[S.tI]
  return isDsdTrack(track)
}

function getPlaybackDuration() {
  return dsdState.active ? dsdState.duration : audio.duration
}

function getPlaybackCurrentTime() {
  if (!dsdState.active || !dsdState.context) return audio.currentTime
  if (!S.playing) return dsdState.pausedAt || 0
  return Math.min(dsdState.duration, Math.max(0, dsdState.pausedAt + (dsdState.context.currentTime - dsdState.startedAt)))
}

function stopDsdPlayback(resetTime = true) {
  if (dsdState.raf) {
    cancelAnimationFrame(dsdState.raf)
    dsdState.raf = 0
  }
  if (dsdState.source) {
    try { dsdState.source.onended = null } catch { /* ignore */ }
    try { dsdState.source.stop() } catch { /* ignore */ }
    try { dsdState.source.disconnect() } catch { /* ignore */ }
  }
  if (dsdState.context) {
    try { dsdState.context.close() } catch { /* ignore */ }
  }
  dsdState.source = null
  dsdState.context = null
  dsdState.gainNode = null
  dsdState.buffer = null
  dsdState.active = false
  dsdState.path = null
  dsdState.startedAt = 0
  if (resetTime) dsdState.pausedAt = 0
  dsdState.duration = 0
}

function syncDsdVolume() {
  if (!dsdState.active || !dsdState.gainNode) return
  dsdState.gainNode.gain.value = S.muted ? 0 : (S.vol / 100)
}

function startDsdProgressLoop() {
  if (!dsdState.active) return
  if (dsdState.raf) cancelAnimationFrame(dsdState.raf)
  const tick = () => {
    if (!dsdState.active) return
    if (S.playing) {
      S.cTime = getPlaybackCurrentTime()
      if (!_progressDragging) {
        const p = S.dur ? (S.cTime / S.dur) * 100 : 0
        _progressFill.style.width = p + '%'
        _progressHandle.style.left = p + '%'
        _progressCurrent.textContent = fmtTime(S.cTime)
      }
    }
    dsdState.raf = requestAnimationFrame(tick)
  }
  dsdState.raf = requestAnimationFrame(tick)
}

async function playDsdTrack(track, seekTime = 0) {
  stopDsdPlayback(false)
  audio.pause()
  audio.removeAttribute('src')
  audio.load()
  const decoded = await api.dsdDecodePcm(track.path)
  if (!decoded || !decoded.ok || !decoded.pcmBase64) throw new Error(decoded?.error || 'DSD 解码失败')

  const pcmBytes = Uint8Array.from(atob(decoded.pcmBase64), c => c.charCodeAt(0))
  const pcmView = new Int16Array(pcmBytes.buffer, pcmBytes.byteOffset, Math.floor(pcmBytes.byteLength / 2))
  const channels = decoded.channels || 2
  const sampleRate = decoded.sampleRate || 44100
  const frames = Math.floor(pcmView.length / channels)
  const context = new AudioContext({ sampleRate })
  if (context.state === 'suspended') await context.resume()
  const gainNode = context.createGain()
  const buffer = context.createBuffer(channels, frames, sampleRate)

  for (let channel = 0; channel < channels; channel++) {
    const channelData = buffer.getChannelData(channel)
    for (let frame = 0; frame < frames; frame++) {
      const sample = pcmView[frame * channels + channel] || 0
      channelData[frame] = sample / 32768
    }
  }

  const source = context.createBufferSource()
  source.buffer = buffer
  source.connect(gainNode)
  gainNode.connect(context.destination)

  dsdState.active = true
  dsdState.path = track.path
  dsdState.context = context
  dsdState.gainNode = gainNode
  dsdState.buffer = buffer
  dsdState.source = source
  dsdState.duration = buffer.duration
  dsdState.pausedAt = seekTime
  dsdState.startedAt = context.currentTime
  syncDsdVolume()

  source.onended = () => {
    if (!dsdState.active) return
    const endedNaturally = S.playing && (getPlaybackCurrentTime() >= dsdState.duration - 0.05)
    stopDsdPlayback()
    if (endedNaturally) hEnd()
  }

  source.start(0, Math.max(0, Math.min(seekTime, buffer.duration - 0.01)))
  S.dur = buffer.duration
  _progressDuration.textContent = fmtTime(S.dur)
  S.cTime = seekTime
  S.tI = pl.findIndex(item => item.id === track.id)
  S.playingTid = track.id
  S.playing = true
  updPlayBtn()
  updPlayStateClass()
  startDsdProgressLoop()
}

function goToTrackFolder(tid) {
  const track = S.all.find(t => t.id === tid)
  if (!track) return
  const normalizedPath = track.path.replace(/\\/g, '/')
  let cursor = normalizedPath.replace(/\/[^/]+$/, '')
  let node = findNodeByPath(S.folderTree, cursor)
  while (!node && cursor.includes('/')) {
    cursor = cursor.replace(/\/[^/]+$/, '')
    node = findNodeByPath(S.folderTree, cursor)
  }
  if (!node) return
  const stack = []
  let current = node.path
  while (current) {
    stack.unshift(current)
    const parent = current.replace(/\/[^/]+$/, '')
    if (parent === current || !findNodeByPath(S.folderTree, parent)) break
    current = parent
  }
  S.activeFp = stack[0] || node.path
  S.folderStack = stack
  S.view = 'all'
  S.aI = -1
  S.alI = -1
  S.aPl = null
  S.aF = null
  renderAll()
  schedSave()
}

function openLyricsForTrack(tid) {
  const idxInCurrent = pl.findIndex(t => t.id === tid)
  if (idxInCurrent >= 0) {
    if (S.view !== 'lyrics') S.prevView = S.view
    playT(idxInCurrent)
    return
  }
  pl = S.all
  const idx = pl.findIndex(t => t.id === tid)
  if (idx >= 0) {
    if (S.view !== 'lyrics') S.prevView = S.view
    playT(idx)
  }
}

// === Favorites/Playlists ===
function mkP(n) { n = n || '\u65b0\u5217\u8868'; const id = 'pl-' + Date.now() + '-' + (++_idCounter); S.pls.push({ id, name: n, trackIds: [], coverData: null }); schedSave(); renderAll(); requestAnimationFrame(() => startRename('pl', id)) }
async function rmP(id) { const p = S.pls.find(x => x.id === id); if (!p) return; const ok = await showConfirm('删除播放列表', `确定删除播放列表“${p.name}”吗？`); if (!ok) return; S.pls = S.pls.filter(x => x.id !== id); if (S.aPl === id) { S.aPl = null; S.view = 'all' } schedSave(); renderAll() }
function rnP(id) { startRename('pl', id) }
function _resetPlayingIfRemoved(tid) {
  if (S.playingTid !== tid) return
  S.playingTid = null; S.view = 'all'; S.aF = null; S.aPl = null
  audio.pause(); S.playing = false; lrc = []
}
function a2P(pid, tid) { const p = S.pls.find(x => x.id === pid); if (!p || p.trackIds.includes(tid)) return; p.trackIds.push(tid); if (!p.coverData) { const t = S.all.find(x => x.id === tid); if (t) p.coverData = t.coverData || t.albumCoverData } schedSave(); renderAll() }
function rFP(pid, tid) { const p = S.pls.find(x => x.id === pid); if (!p) return; p.trackIds = p.trackIds.filter(id => id !== tid); if (!p.trackIds.length) p.coverData = null; _resetPlayingIfRemoved(tid); schedSave(); renderAll() }
function mkF(n) { n = n || '\u65b0\u6536\u85cf\u5939'; const id = 'fav-' + Date.now() + '-' + (++_idCounter); S.favs.push({ id, name: n, trackIds: [], isDefault: false }); schedSave(); renderAll(); requestAnimationFrame(() => startRename('fav', id)) }
async function rmF(id) { const f = S.favs.find(x => x.id === id); if (!f) return; const ok = await showConfirm('删除收藏夹', `确定删除收藏夹“${f.name}”吗？`); if (!ok) return; S.favs = S.favs.filter(x => x.id !== id); if (S.aF === id) { S.aF = null; S.view = 'all' } schedSave(); renderAll() }
function rnF(id) { startRename('fav', id) }
function a2F(fid, tid) { const f = S.favs.find(x => x.id === fid); if (!f || f.trackIds.includes(tid)) return; f.trackIds.push(tid); schedSave(); renderAll() }
function rFF(fid, tid) { const f = S.favs.find(x => x.id === fid); if (!f) return; f.trackIds = f.trackIds.filter(id => id !== tid); _resetPlayingIfRemoved(tid); schedSave(); renderAll() }

function startRename(type, id) {
  const isFav = type === 'fav', list = isFav ? S.favs : S.pls, item = list.find(x => x.id === id)
  if (!item || (isFav && item.isDefault)) return
  let el = null, isSidebar = false
  if (isFav) { el = document.querySelector(`.fav-sidebar-item[data-fvid="${CSS.escape(id)}"] .fav-sidebar-name`); if (el) isSidebar = true; if (!el) el = document.querySelector(`.pl-content-name[data-fvid="${CSS.escape(id)}"]`) }
  else { el = document.querySelector(`.playlist-sidebar-item[data-plid="${CSS.escape(id)}"] .pl-sidebar-name`); if (el) isSidebar = true; if (!el) el = document.querySelector(`.pl-content-name[data-plid="${CSS.escape(id)}"]`) }
  if (!el) return
  const origHTML = el.innerHTML, w = Math.max(80, el.offsetWidth || 120)
  const inp = document.createElement('input'); inp.className = 'rename-input'; inp.value = item.name; inp.style.width = w + 'px'
  if (isSidebar) { inp.style.fontSize = '13px'; inp.style.fontWeight = '400' }
  el.innerHTML = ''; el.appendChild(inp); inp.focus(); inp.select()
  const finish = () => { const v = inp.value.trim(); if (v && v !== item.name) { item.name = v; schedSave(); renderAll() } else { el.innerHTML = origHTML } }
  inp.addEventListener('blur', finish); inp.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); inp.blur() } if (e.key === 'Escape') { inp.value = item.name; inp.blur() } })
}

function renderPContent(plObj, tks) {
  const td = tks.find(t => t.coverData || t.albumCoverData), cd = td ? td.coverData || td.albumCoverData : null
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">\u64ad\u653e\u5217\u8868</div><div class="pl-content-name" data-plid="${plObj.id}" title="\u53cc\u51fb\u91cd\u547d\u540d">${esc(plObj.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-ppl="${plObj.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button><button class="btn-danger" data-delpl="${plObj.id}">\u5220\u9664</button></div></div></div>${tableVT('vl-pl-' + plObj.id, tks, (idx) => playT(idx, true))}`
}
function renderFContent(fav, tks) {
  const td = tks.find(t => t.coverData || t.albumCoverData), cd = td ? td.coverData || td.albumCoverData : null
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">收藏夹</div><div class="pl-content-name" data-fvid="${fav.id}" title="${fav.isDefault ? '默认收藏夹' : '双击重命名'}">${esc(fav.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-pfav="${fav.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>播放全部</button>${!fav.isDefault ? `<button class="btn-danger" data-delfv="${fav.id}">删除</button>` : ''}</div></div></div>${tableVT('vl-fv-' + fav.id, tks, (idx) => playT(idx, true))}`
}

// === Scan ===
async function importFolder() {
  if (_scanRunning) return
  _scanRunning = true
  try {
    const result = await api.openFolder(); if (!result || !result.length) return
    const normalized = result.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
    for (const p of normalized) { if (!fp.includes(p)) fp.push(p) }
    const tid = addT('\u6b63\u5728\u626b\u63cf\u97f3\u4e50\u6587\u4ef6...')
    api.removeScanProgressListener()
    let removeProgress = null
    removeProgress = api.onScannerProgress((data) => { updT(tid, `${data.stage || '\u89e3\u6790\u4e2d...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
    const r = await api.scanFoldersWithProgress(fp)
    const at = applyScanResult(r)
    const allIds = new Set(at.map(t => t.id))
    cleanupStale(allIds)
    S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []; S.activeFp = null
    pl = at
    // Check if currently playing track still exists in new scan results
    if (S.playingTid && !allIds.has(S.playingTid)) { S.playingTid = null; S.playing = false; audio.pause(); lrc = [] }
    if (r.fileCount > 0) { updT(tid, '\u5b8c\u6210\u2714', 100, `\u5171 ${r.fileCount || at.length} \u9996\u97f3\u4e50`); rmT(tid) } else { updT(tid, '\u672a\u627e\u5230\u97f3\u4e50', 0, '\u8bf7\u68c0\u67e5\u6587\u4ef6\u5939\u5185\u5bb9'); setTimeout(() => rmT(tid), 5000) }
    await restartWatching()
    renderAll()
  } catch (e) { alert('\u5bfc\u5165\u5931\u8d25: ' + e.message) }
  finally { _scanRunning = false; removeProgress?.() }
}

async function rescan() {
  if (_scanRunning) return
  if (!fp.length) {
    S.af = []; S.all = []; S.folderTree = []; S.folderStack = []; S._folderMeta = null
    pl = []; S.playingTid = null; S.tI = -1; audio.pause()
    cleanupStale(new Set())
    renderAll(); schedSave()
    return
  }
  const tid = addT('\u91cd\u65b0\u626b\u63cf...')
  api.removeScanProgressListener()
  const removeProgress = api.onScannerProgress((data) => { updT(tid, `${data.stage || '\u89e3\u6790\u4e2d...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
  const removeStage = api.onScannerStage((stage) => { const e = document.getElementById(tid + '-status'); if (e) e.textContent = stage })
  try {
    const currentTrackId = pl.length > 0 && S.tI >= 0 ? pl[S.tI]?.id : null
    console.time('[total] scan->show')
    console.time('[scan] IPC wait')
    const r = await api.scanFoldersWithProgress(fp)
    console.timeEnd('[scan] IPC wait')
    console.time('[scan] applyAndRender')
    const at = applyScanResult(r)
    const allIds = new Set(at.map(t => t.id))
    cleanupStale(allIds)
    if (S.aF) {
      const fav = S.favs.find(f => f.id === S.aF)
      pl = fav ? fav.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean) : at
    } else if (S.aPl) {
      const plObj = S.pls.find(p => p.id === S.aPl)
      pl = plObj ? plObj.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean) : at
    } else {
      pl = at
    }
    if (currentTrackId) {
      const newIdx = pl.findIndex(t => t.id === currentTrackId)
      if (newIdx >= 0) S.tI = newIdx
    }
    await restartWatching()
    updT(tid, '\u5b8c\u6210\u2714', 100, `\u5171 ${r.fileCount || at.length} \u9996`)
    rmT(tid)
    renderAll()
    console.timeEnd('[scan] applyAndRender')
    console.timeEnd('[total] scan->show')
  } catch (e) { updT(tid, '\u5931\u8d25', 0, e.message) }
  finally { _scanRunning = false; removeProgress?.(); removeStage?.() }
}

// === Panel ===
function renderPanel() {
  const b = $('panel-body')
  if (!pl.length) { b.innerHTML = '<div class="panel-empty">\u64ad\u653e\u5217\u8868\u4e3a\u7a7a</div>'; $('panel-count').textContent = '0 \u9996'; return }
  $('panel-count').textContent = pl.length + ' \u9996'
  b.innerHTML = pl.map((t, i) => `<div class="panel-track${i === nI ? ' playing' : ''}" data-pidx="${i}"><span class="pt-idx">${i + 1}</span><div class="pt-info"><div class="pt-title">${esc(t.name)}</div><div class="pt-artist">${esc(t.metaArtist || t.artist)}</div></div></div>`).join('')
}

function playAll(tracks) { if (!tracks || !tracks.length) return; pl = tracks; syncPlayingState(); if (S.view !== 'lyrics') S.prevView = S.view; S.view = 'lyrics'; activeLrcTab = 'lyrics'; playT(0); renderPanel() }

// === ctx ===
function showCtx(e, ci) {
  const m = $('ctx-menu'), ps = $('ctx-playlist-sub')
  const groups = []

  if (ci?.tid) {
    const track = S.all.find(t => t.id === ci.tid)
    const liked = !!getDefaultFav()?.trackIds.includes(ci.tid)
    groups.push([
      `<button data-a="playnow">\u7acb\u5373\u64ad\u653e</button>`,
      `<button data-a="openlyrics">\u6253\u5f00\u6b4c\u8bcd</button>`
    ])
    groups.push([
      `<button data-a="togglelike">${liked ? '\u79fb\u51fa\u6211\u7684\u559c\u7231' : '\u52a0\u5165\u6211\u7684\u559c\u7231'}</button>`,
      `<button data-a="copyname">\u590d\u5236\u6b4c\u540d</button>`,
      `<button data-a="copypath">\u590d\u5236\u6587\u4ef6\u8def\u5f84</button>`
    ])
    if (track) {
      groups.push([`<button data-a="gofolder">\u8df3\u8f6c\u5230\u6240\u5728\u6587\u4ef6\u5939</button>`])
    }

    if (S.favs.length > 0) groups.push([`<button data-a="addfav">\u6dfb\u52a0\u5230\u6536\u85cf\u5939...</button>`])
    if (S.pls.length > 0) groups.push([`<button data-a="addpl">\u6dfb\u52a0\u5230\u64ad\u653e\u5217\u8868...</button>`])

    // Remove from playlist
    if (ci?.pid) {
      const plObj = S.pls.find(p => p.id === ci.pid)
      if (plObj) groups.push([`<button data-a="rmfrompl" class="danger">\u4ece\u64ad\u653e\u5217\u8868\u4e2d\u79fb\u9664</button>`])
    }

    // Remove from favorites
    if (ci?.fid) {
      const favObj = S.favs.find(f => f.id === ci.fid)
      if (favObj) groups.push([`<button data-a="rmfromfav" class="danger">\u4ece\u6536\u85cf\u5939\u4e2d\u79fb\u9664</button>`])
    }

    // Always show at least something for track context
    if (groups.length === 0) groups.push([`<button data-a="close">\u5173\u95ed</button>`])
  } else {
    // Not on a track: show rename/delete for playlist/favorites
    if (ci?.pid) {
      groups.push([
        `<button data-a="rnpl">\u91cd\u547d\u540d</button>`,
        `<button data-a="delpl" class="danger">\u5220\u9664\u5217\u8868</button>`
      ])
    } else if (ci?.fid && !S.favs.find(f => f.id === ci.fid)?.isDefault) {
      groups.push([
        `<button data-a="rnfv">\u91cd\u547d\u540d</button>`,
        `<button data-a="delfv" class="danger">\u5220\u9664\u6536\u85cf\u5939</button>`
      ])
    }
  }

  // Build HTML with separators between groups
  let html = ''
  for (let i = 0; i < groups.length; i++) {
    if (i > 0) html += '<hr>'
    html += groups[i].join('')
  }
  if (!html) return
  m.innerHTML = html; m.classList.remove('hidden')
  // Force reflow to get actual dimensions
  m.offsetHeight
  const menuW = m.offsetWidth, menuH = m.offsetHeight
  // Position main menu within viewport
  let mx = e.clientX, my = e.clientY
  if (mx + menuW > window.innerWidth) mx = window.innerWidth - menuW - 5
  if (my + menuH > window.innerHeight) my = window.innerHeight - menuH - 5
  m.style.left = Math.max(5, mx) + 'px'
  m.style.top = Math.max(5, my) + 'px'
  m._menuX = Math.max(5, mx); m._menuY = Math.max(5, my)
  m.onclick = function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'playnow' && ci?.tid) { const idx = pl.findIndex(t => t.id === ci.tid); if (idx >= 0) playT(idx, true); else openLyricsForTrack(ci.tid); hC(); return }
    if (b.dataset.a === 'openlyrics' && ci?.tid) { openLyricsForTrack(ci.tid); hC(); return }
    if (b.dataset.a === 'togglelike' && ci?.tid) { toggleDefaultFavorite(ci.tid); hC(); renderAll(); return }
    if (b.dataset.a === 'copyname' && ci?.tid) { const track = S.all.find(t => t.id === ci.tid); if (track) copyText(track.name, '已复制歌名'); hC(); return }
    if (b.dataset.a === 'copypath' && ci?.tid) { const track = S.all.find(t => t.id === ci.tid); if (track) copyText(track.path, '已复制路径'); hC(); return }
    if (b.dataset.a === 'gofolder' && ci?.tid) { goToTrackFolder(ci.tid); hC(); return }
    if (b.dataset.a === 'addfav') { showFavPicker(ci.tid, m._menuX, m._menuY, menuH); return }
    if (b.dataset.a === 'addpl') { showPlPicker(ci.tid, m._menuX, m._menuY, menuH); return }
    if (b.dataset.a === 'rmfrompl' && ci?.tid && ci?.pid) { rFP(ci.pid, ci.tid); hC(); return }
    if (b.dataset.a === 'rmfromfav' && ci?.tid && ci?.fid) { rFF(ci.fid, ci.tid); hC(); return }
    if (b.dataset.a === 'rnpl' && ci?.pid) { rnP(ci.pid); hC(); return }
    if (b.dataset.a === 'delpl' && ci?.pid) { hC(); rmP(ci.pid); return }
    if (b.dataset.a === 'rnfv' && ci?.fid) { rnF(ci.fid); hC(); return }
    if (b.dataset.a === 'delfv' && ci?.fid) { hC(); rmF(ci.fid); return }
    hC()
  }
  ps.classList.add('hidden')
}

function showFavAndPlPicker(e, tid) {
  const fakeCi = { tid }
  if (S.aPl) fakeCi.pid = S.aPl
  if (S.aF) fakeCi.fid = S.aF
  showCtx(e, fakeCi)
}

function showFavPicker(tid, baseX, baseY) {
  const ps = $('ctx-playlist-sub')
  ps.innerHTML = S.favs.map(f => `<button data-ffid="${f.id}">${f.isDefault ? '\u2605 ' : ''}${esc(f.name)}</button>`).join('')
  ps.classList.remove('hidden')
  // Force reflow
  ps.offsetHeight
  const psW = ps.offsetWidth, psH = ps.offsetHeight
  // Try right side first, if no space try left
  let psX = baseX + 200
  if (psX + psW > window.innerWidth) psX = baseX - psW
  // Adjust vertical to stay within viewport
  let psY = baseY
  if (psY + psH > window.innerHeight) psY = window.innerHeight - psH - 5
  if (psY < 5) psY = 5
  ps.style.left = Math.max(5, psX) + 'px'
  ps.style.top = psY + 'px'
  ps.onclick = function (ev) { const b = ev.target.closest('button'); if (b && b.dataset.ffid) { a2F(b.dataset.ffid, tid); ps.classList.add('hidden'); hC(); renderAll() } }
}

function showPlPicker(tid, baseX, baseY) {
  const ps = $('ctx-playlist-sub')
  ps.innerHTML = S.pls.map(p => `<button data-plid="${p.id}">${esc(p.name)}</button>`).join('')
  ps.classList.remove('hidden')
  // Force reflow
  ps.offsetHeight
  const psW = ps.offsetWidth, psH = ps.offsetHeight
  // Try right side first, if no space try left
  let psX = baseX + 200
  if (psX + psW > window.innerWidth) psX = baseX - psW
  // Adjust vertical to stay within viewport
  let psY = baseY
  if (psY + psH > window.innerHeight) psY = window.innerHeight - psH - 5
  if (psY < 5) psY = 5
  ps.style.left = Math.max(5, psX) + 'px'
  ps.style.top = psY + 'px'
  ps.onclick = function (ev) { const b = ev.target.closest('button'); if (b && b.dataset.plid) { a2P(b.dataset.plid, tid); ps.classList.add('hidden'); hC() } }
}

const _ctxMenu = $('ctx-menu'), _ctxPlaylistSub = $('ctx-playlist-sub')
function hC() { _ctxMenu.classList.add('hidden'); _ctxPlaylistSub.classList.add('hidden') }

// === Tools ===
const toolsState = { extractFiles: [], convertFiles: [], convertFmt: 'mp3', extractFmt: 'mp3', extractRunning: false, convertRunning: false }

function renderToolsContent() {
  $('breadcrumb').innerHTML = `<button class="breadcrumb-item current">\u5de5\u5177</button>`
  $('content-area').innerHTML = `
    <div class="tools-container">
      <div class="tools-section" id="tools-extract">
        <h3>\u89c6\u9891\u63d0\u53d6\u97f3\u9891</h3>
        <p>\u4ece\u89c6\u9891\u6587\u4ef6\u4e2d\u63d0\u53d6\u97f3\u9891\u8f68\uff0c\u652f\u6301 MP4/MKV/AVI/MOV \u7b49\u683c\u5f0f\u3002</p>
        <div class="tools-dropzone" id="tools-extract-dropzone">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
          <div class="drop-text">\u62d6\u62fd\u89c6\u9891\u6587\u4ef6\u5230\u6b64\u5904</div>
          <div class="drop-hint">\u6216\u70b9\u51fb\u9009\u62e9\u6587\u4ef6\uff08\u652f\u6301\u591a\u9009\uff09</div>
        </div>
        <div class="tools-format-select" id="tools-extract-fmts">
          <button class="tools-format-opt active" data-fmt="mp3">MP3</button>
          <button class="tools-format-opt" data-fmt="aac">AAC</button>
          <button class="tools-format-opt" data-fmt="flac">FLAC</button>
          <button class="tools-format-opt" data-fmt="wav">WAV</button>
          <button class="tools-format-opt" data-fmt="ogg">OGG</button>
        </div>
        <div class="tools-overall-progress hidden" id="extract-overall">
          <div class="tools-overall-text" id="extract-overall-text">\u603b\u8fdb\u5ea6: 0/0</div>
          <div class="tools-progress-bar"><div class="tools-progress-fill" id="extract-overall-fill"></div></div>
        </div>
        <div class="tools-file-list" id="tools-extract-files"></div>
        <div class="tools-actions hidden" id="extract-actions">
          <button class="btn-primary" id="btn-extract-start">\u5f00\u59cb\u63d0\u53d6</button>
          <button class="btn-secondary" id="btn-extract-clear">\u6e05\u7a7a\u5217\u8868</button>
        </div>
      </div>
      <div class="tools-section" id="tools-convert">
        <h3>\u97f3\u9891\u683c\u5f0f\u8f6c\u6362</h3>
        <p>\u5c06\u97f3\u9891\u6587\u4ef6\u6279\u91cf\u8f6c\u6362\u4e3a\u5176\u4ed6\u683c\u5f0f\u3002</p>
        <div class="tools-dropzone" id="tools-convert-dropzone">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
          <div class="drop-text">\u62d6\u62fd\u97f3\u9891\u6587\u4ef6\u5230\u6b64\u5904</div>
          <div class="drop-hint">\u6216\u70b9\u51fb\u9009\u62e9\u6587\u4ef6\uff08\u652f\u6301\u591a\u9009\uff09</div>
        </div>
        <div class="tools-format-select" id="tools-convert-fmts">
          <button class="tools-format-opt active" data-fmt="mp3">MP3</button>
          <button class="tools-format-opt" data-fmt="wav">WAV</button>
          <button class="tools-format-opt" data-fmt="flac">FLAC</button>
          <button class="tools-format-opt" data-fmt="aac">AAC</button>
          <button class="tools-format-opt" data-fmt="ogg">OGG</button>
          <button class="tools-format-opt" data-fmt="m4a">M4A</button>
        </div>
        <div class="tools-overall-progress hidden" id="convert-overall">
          <div class="tools-overall-text" id="convert-overall-text">\u603b\u8fdb\u5ea6: 0/0</div>
          <div class="tools-progress-bar"><div class="tools-progress-fill" id="convert-overall-fill"></div></div>
        </div>
        <div class="tools-file-list" id="tools-convert-files"></div>
        <div class="tools-actions hidden" id="convert-actions">
          <button class="btn-primary" id="btn-convert-start">\u5f00\u59cb\u8f6c\u6362</button>
          <button class="btn-secondary" id="btn-convert-clear">\u6e05\u7a7a\u5217\u8868</button>
        </div>
      </div>
    </div>`
  setupToolsEvents()
}

function renderToolFileList(id, files, section) {
  const el = $(id); if (!el) return
  el.innerHTML = files.map((f, i) => {
    const fn = typeof f === 'string' ? (f.split(/[\\/]/).pop()) : (f.name || f.path?.split(/[\\/]/).pop() || String(f))
    const status = f._status || ''
    const statusText = status === 'done' ? '\u5b8c\u6210' : status === 'error' ? '\u5931\u8d25' : status === 'running' ? '\u5904\u7406\u4e2d...' : ''
    const titleAttr = f._errorMsg ? ` title="${esc(f._errorMsg)}"` : ''
    const css = f._pct !== undefined ? `background:linear-gradient(to right,var(--accent) 0%,var(--accent) ${f._pct}%,transparent ${f._pct}%)` : ''
    return `<div class="tools-file-item${status ? ' ' + status : ''}" style="${css}" data-tfi="${i}" data-tf-section="${section}"${titleAttr}><span class="file-name">${esc(fn)}</span><span class="file-status">${statusText}</span>${(!status || status === 'error') ? `<button class="file-remove" data-idx="${i}">&times;</button>` : ''}</div>`
  }).join('')
}

let _toolsEventsSetup = false

function setupToolsEvents() {
  if (_toolsEventsSetup) return
  _toolsEventsSetup = true
  toolsState.extractFiles = []; toolsState.convertFiles = []

  // Event delegation on document.body (persistent, survives innerHTML replacement)
  document.body.addEventListener('click', e => {
    // Dropzone click -> open file dialog
    const dz = e.target.closest('.tools-dropzone')
    if (dz) {
      const section = dz.id.includes('extract') ? 'extract' : 'convert'
      handleToolDropzoneClick(section)
      return
    }
    // Format selector click
    const fmtBtn = e.target.closest('.tools-format-opt')
    if (fmtBtn) {
      const container = fmtBtn.closest('.tools-format-select')
      container.querySelectorAll('.tools-format-opt').forEach(x => x.classList.remove('active'))
      fmtBtn.classList.add('active')
      if (container.id.includes('extract')) toolsState.extractFmt = fmtBtn.dataset.fmt
      else toolsState.convertFmt = fmtBtn.dataset.fmt
      return
    }
    // File remove click
    const removeBtn = e.target.closest('.file-remove')
    if (removeBtn) {
      const item = removeBtn.closest('.tools-file-item')
      const section = item ? item.dataset.tfSection : null
      if (section) {
        const arr = section === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
        arr.splice(parseInt(removeBtn.dataset.idx), 1)
        const id = section === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
        const actionsId = section === 'extract' ? 'extract-actions' : 'convert-actions'
        const overallId = section === 'extract' ? 'extract-overall' : 'convert-overall'
        renderToolFileList(id, arr, section)
        if (arr.length === 0) { $(actionsId).classList.add('hidden'); $(overallId).classList.add('hidden') }
      }
      return
    }
    // Start button
    const startBtn = e.target.closest('[id^="btn-"][id$="-start"]')
    if (startBtn) {
      if (startBtn.id === 'btn-extract-start' && toolsState.extractFiles.length) startExtractBatch([...toolsState.extractFiles])
      else if (startBtn.id === 'btn-convert-start' && toolsState.convertFiles.length) startConvertBatch([...toolsState.convertFiles])
      return
    }
    // Clear button
    const clearBtn = e.target.closest('[id^="btn-"][id$="-clear"]')
    if (clearBtn) {
      if (clearBtn.id === 'btn-extract-clear') {
        toolsState.extractFiles = []; renderToolFileList('tools-extract-files', [], 'extract')
        $('extract-actions').classList.add('hidden'); $('extract-overall').classList.add('hidden')
      } else if (clearBtn.id === 'btn-convert-clear') {
        toolsState.convertFiles = []; renderToolFileList('tools-convert-files', [], 'convert')
        $('convert-actions').classList.add('hidden'); $('convert-overall').classList.add('hidden')
      }
    }
  })

  // Drag-and-drop delegation on document
  document.addEventListener('dragover', e => {
    const dz = e.target.closest('.tools-dropzone')
    if (dz) { e.preventDefault(); dz.classList.add('drag-over') }
    else { e.preventDefault() }
  }, { passive: false })
  document.addEventListener('dragleave', e => {
    const dz = e.target.closest('.tools-dropzone')
    if (dz) dz.classList.remove('drag-over')
  })
  document.addEventListener('drop', e => {
    const dz = e.target.closest('.tools-dropzone')
    if (dz && e.dataTransfer && e.dataTransfer.files) {
      e.preventDefault(); dz.classList.remove('drag-over')
      const section = dz.id.includes('extract') ? 'extract' : 'convert'
      const arr = section === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
      const existingPaths = new Set(arr.map(x => typeof x === 'string' ? x : x.path))
      for (const f of e.dataTransfer.files) {
        const fp = f.path || f.name; if (!existingPaths.has(fp)) { arr.push(fp); existingPaths.add(fp) }
      }
      const id = section === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
      const actionsId = section === 'extract' ? 'extract-actions' : 'convert-actions'
      renderToolFileList(id, arr, section)
      if (arr.length > 0) $(actionsId).classList.remove('hidden')
    } else if (!e.target.closest('.tools-dropzone')) {
      e.preventDefault()
    }
  })
}

async function handleToolDropzoneClick(section) {
  const files = await api.openAudioFiles()
  if (!files || !files.length) return
  const arr = section === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
  const existingPaths = new Set(arr.map(x => typeof x === 'string' ? x : x.path))
  for (const f of files) { if (!existingPaths.has(f)) { arr.push(f); existingPaths.add(f) } }
  const id = section === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
  const actionsId = section === 'extract' ? 'extract-actions' : 'convert-actions'
  renderToolFileList(id, arr, section)
  if (arr.length > 0) $(actionsId).classList.remove('hidden')
}

async function runFfmpegBatch(files, section, fmt, codecMap) {
  const runningKey = section === 'extract' ? 'extractRunning' : 'convertRunning'
  if (toolsState[runningKey]) return
  toolsState[runningKey] = true
  const overall = $(`${section}-overall`); overall.classList.remove('hidden')
  const overallText = $(`${section}-overall-text`); const overallFill = $(`${section}-overall-fill`)
  overallText.textContent = `\u603b\u8fdb\u5ea6: 0/${files.length}`
  overallFill.style.width = '0%'

  const entries = files.map(f => typeof f === 'string' ? { path: f, _status: 'pending', _pct: 0 } : f)
  const fileListId = `tools-${section}-files`

  for (let i = 0; i < entries.length; i++) {
    const f = entries[i]
    const fpath = f.path
    f._status = 'running'; f._pct = 10
    renderToolFileList(fileListId, entries, section)
    overallText.textContent = `\u603b\u8fdb\u5ea6: \u7b2c${i + 1}/\u5171${entries.length} \u4e2a \u5904\u7406\u4e2d...`
    overallFill.style.width = ((i / entries.length) * 100 + 5) + '%'

    const baseName = fpath.replace(/\.[^.]+$/, '')
    const outPath = baseName + '.' + fmt
    const args = ['-i', fpath, ...(codecMap[fmt] || codecMap.mp3), '-y', outPath]

    try {
      const result = await api.ffmpegExec(args)
      if (result.code !== 0) throw new Error(result.stderr?.split('\n').slice(-3).join(' ') || 'ffmpeg \u9000\u51fa\u7801: ' + result.code)
      f._status = 'done'; f._pct = 100
    } catch (e) {
      f._status = 'error'; f._pct = undefined
      f._errorMsg = e.message || '\u672a\u77e5\u9519\u8bef'
    }
    renderToolFileList(fileListId, entries, section)
    overallFill.style.width = (((i + 1) / entries.length) * 100) + '%'
  }
  const doneCount = entries.filter(f => f._status === 'done').length
  overallText.textContent = `\u5b8c\u6210: ${doneCount}/${entries.length} \u6210\u529f`
  toolsState[runningKey] = false
}

async function startExtractBatch(files) {
  const codecMap = { mp3: ['-vn', '-acodec', 'libmp3lame', '-q:a', '2'], flac: ['-vn', '-acodec', 'flac'], wav: ['-vn', '-acodec', 'pcm_s16le'], ogg: ['-vn', '-acodec', 'libvorbis', '-q:a', '4'], aac: ['-vn', '-acodec', 'aac', '-b:a', '192k'] }
  await runFfmpegBatch(files, 'extract', toolsState.extractFmt || 'mp3', codecMap)
}

async function startConvertBatch(files) {
  const codecMap = { mp3: ['-acodec', 'libmp3lame', '-q:a', '2'], flac: ['-acodec', 'flac'], wav: ['-acodec', 'pcm_s16le'], ogg: ['-acodec', 'libvorbis', '-q:a', '4'], m4a: ['-acodec', 'aac', '-b:a', '192k'], aac: ['-acodec', 'aac', '-b:a', '192k'] }
  await runFfmpegBatch(files, 'convert', toolsState.convertFmt || 'mp3', codecMap)
}

// === Events ===
$('content-area').addEventListener('click', e => {
  const sortBtn = e.target.closest('[data-fsort]')
  if (sortBtn) {
    S.folderSort = sortBtn.dataset.fsort; schedSave(); renderContent(); return
  }
  const viewBtn = e.target.closest('[data-fview]')
  if (viewBtn) {
    S.folderView = viewBtn.dataset.fview; schedSave(); renderContent(); return
  }
  const lAct = e.target.closest('[data-lact]')
  if (lAct) {
    const currentTrack = S.playingTid ? S.all.find(t => t.id === S.playingTid) : null
    if (lAct.dataset.lact === 'folder' && currentTrack) { goToTrackFolder(currentTrack.id); return }
    if (lAct.dataset.lact === 'copy' && currentTrack) { copyText(currentTrack.path, '已复制路径'); return }
  }
  // Check like button FIRST before song-row (it's a child of song-row)
  const like = e.target.closest('.song-row-like'); if (like) {
    e.stopPropagation(); const tid = like.dataset.tid
    showFavAndPlPicker(e, tid); return
  }
  const playBtn = e.target.closest('.idx-play-btn')
  if (playBtn) {
    const row = e.target.closest('.song-row'); if (row && row.dataset.tid) { const idx = pl.findIndex(tk => tk.id === row.dataset.tid); if (idx >= 0) playT(idx, true); return }
  }
  const t = e.target.closest('.song-row'); if (t && t.dataset.tid) { /* single click does nothing, use dblclick or play button */ return }
  if (e.target.closest('[data-pa]')) {
    const el = e.target.closest('[data-pa]'); const [ai, al_] = el.dataset.pa.split(':').map(Number)
    if (!isNaN(ai)) { const a = S.af[ai]; if (al_ !== undefined && !isNaN(al_)) { if (a) playAll(a.albums[al_]?.tracks || []); return } const tks = a ? [].concat(...a.albums.map(al => al.tracks)) : []; playAll(tks) }
    return
  }
  if (e.target.closest('[data-ppl]')) { const el = e.target.closest('[data-ppl]'); const pid = el.dataset.ppl; const p = S.pls.find(x => x.id === pid); if (p) { const tks = p.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean); playAll(tks) } return }
  if (e.target.closest('[data-pfav]')) { const el = e.target.closest('[data-pfav]'); const fid = el.dataset.pfav; const f = S.favs.find(x => x.id === fid); if (f) { const tks = f.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean); playAll(tks) } return }
  if (e.target.closest('[data-pall]')) { playAll(S.all); return }
  if (e.target.closest('[data-pfolder]')) {
    const el = e.target.closest('[data-pfolder]'); const fpPath = el.dataset.pfolder; const n = findNodeByPath(S.folderTree, fpPath)
    if (n) { const all = collectAllTracks(n); if (all.length) { pl = all; playT(0); renderPanel() } }
    return
  }
  if (e.target.closest('[data-delpl]')) { rmP(e.target.closest('[data-delpl]').dataset.delpl); return }
  if (e.target.closest('[data-delfv]')) { rmF(e.target.closest('[data-delfv]').dataset.delfv); return }
  if (e.target.closest('[data-bc]')) return
  if (e.target.closest('#empty-import')) { importFolder(); return }
  if (e.target.closest('.pl-content-name')) return
  if (e.target.closest('[data-fp]')) { const fpVal = e.target.closest('[data-fp]').dataset.fp; if (fpVal) { navigateFolder(fpVal) } return }
  const chk = e.target.closest('.song-row-check input'); if (chk) { e.stopPropagation(); return }
  // Lyrics tab switch
  const lcl = e.target.closest('.lc-line'); if (lcl && lcl.dataset.lidx) {
    const time = lrc[parseInt(lcl.dataset.lidx)]
    if (time && time.time !== undefined) {
      if (isCurrentTrackDsd()) {
        const track = pl[S.tI]
        if (!track) return
        const wasPlaying = S.playing
        dsdState.pausedAt = time.time
        stopDsdPlayback(false)
        S.cTime = time.time
        $('progress-current').textContent = fmtTime(time.time)
        if (wasPlaying) {
          playDsdTrack(track, time.time).catch(() => {})
        } else {
          updPlayBtn()
          updPlayStateClass()
        }
      } else {
        audio.currentTime = time.time
      }
    }
    return
  }
  if (e.target.closest('[data-ltab]')) {
    const tab = e.target.closest('[data-ltab]').dataset.ltab
    activeLrcTab = tab
    document.querySelectorAll('.lyrics-tab-btn').forEach(b => b.classList.remove('active'))
    e.target.closest('.lyrics-tab-btn').classList.add('active')
    const ls = $('lyrics-lines-scroll'); if (ls) ls.classList.toggle('hidden', tab !== 'lyrics')
    const mp = $('lyrics-meta-panel'); if (mp) mp.classList.toggle('hidden', tab !== 'meta')
    return
  }
})

$('content-area').addEventListener('dblclick', e => {
  const sr = e.target.closest('.song-row'); if (sr && sr.dataset.tid) { const idx = pl.findIndex(tk => tk.id === sr.dataset.tid); if (idx >= 0) playT(idx, true); return }
  const cn = e.target.closest('.pl-content-name'); if (!cn) return
  if (cn.dataset.plid) startRename('pl', cn.dataset.plid); else if (cn.dataset.fvid) startRename('fav', cn.dataset.fvid)
})
$('content-area').addEventListener('contextmenu', e => {
  const sr = e.target.closest('.song-row'); if (sr) { e.preventDefault(); showCtx(e, { tid: sr.dataset.tid, pid: S.aPl, fid: S.aF }); return }
  const fc = e.target.closest('.folder-card[data-fp]'); if (fc) { e.preventDefault(); showFolderCtx(e, fc.dataset.fp); return }
  if (S.view === 'lyrics' && S.playingTid) { e.preventDefault(); showCtx(e, { tid: S.playingTid, pid: S.aPl, fid: S.aF }); return }
  if (S.view === 'all' && S.folderStack && S.folderStack.length) { e.preventDefault(); showFolderCtx(e, S.folderStack[S.folderStack.length - 1]); return }
  if (S.view === 'recent') { e.preventDefault(); showRecentCtx(e); return }
  if (S.aPl) { e.preventDefault(); showPlaylistEmptyCtx(e, S.aPl); return }
  if (S.aF) { e.preventDefault(); showFavoriteEmptyCtx(e, S.aF); return }
})

function showFolderCtx(e, folderPath) {
  const m = $('ctx-menu')
  const folderName = folderPath.split(/[\\/]/).pop() || folderPath
  m.innerHTML = `<button data-a="fopen">打开文件夹</button><button data-a="fshow">在文件资源管理器中打开</button><button data-a="fcopy">复制文件夹路径</button><hr><button data-a="frescan">重新扫描此库</button><button data-a="frem" class="danger">从扫描列表移除“${esc(folderName)}”</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 160) + 'px'
  m.onclick = async function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'fopen') { S.activeFp = folderPath; S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = [folderPath]; renderAll(); schedSave(); hC(); return }
    if (b.dataset.a === 'fshow') { await api.showItemInFolder(folderPath); hC(); return }
    if (b.dataset.a === 'fcopy') { copyText(folderPath, '已复制文件夹路径'); hC(); return }
    if (b.dataset.a === 'frescan') { hC(); await rescan(); return }
    if (b.dataset.a === 'frem') {
      hC()
      const ok = await showConfirm('移除文件夹', `确定从扫描列表移除文件夹“${folderName}”吗？`)
      if (!ok) return
      fp = fp.filter(p => p !== folderPath)
      S.folderTree = S.folderTree.filter(n => n.path !== folderPath && !isChildPath(n.path, folderPath))
      S._folderMeta = buildFolderMeta(S.folderTree)
      await rescan()
      schedSave()
      return
    }
    hC()
  }
  $('ctx-playlist-sub').classList.add('hidden')
}

function showRecentCtx(e) {
  const m = $('ctx-menu')
  m.innerHTML = `<button data-a="playrecent">播放最近播放</button><button data-a="clearrecent" class="danger">清空最近播放</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px'
  m.onclick = async function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'playrecent') {
      const tks = S.recents.map(id => S.all.find(t => t.id === id)).filter(Boolean)
      if (tks.length) playAll(tks)
      hC()
      return
    }
    if (b.dataset.a === 'clearrecent') {
      hC()
      const ok = await showConfirm('清空最近播放', '确定清空最近播放记录吗？')
      if (!ok) return
      clearRecents()
      schedSave()
      return
    }
    hC()
  }
}

function showPlaylistEmptyCtx(e, plid) {
  const m = $('ctx-menu')
  m.innerHTML = `<button data-a="playall">播放此列表</button><button data-a="rename">重命名</button><button data-a="delete" class="danger">删除播放列表</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px'
  m.onclick = function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'playall') { const plObj = S.pls.find(p => p.id === plid); const tks = plObj ? plObj.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean) : []; if (tks.length) playAll(tks); hC(); return }
    if (b.dataset.a === 'rename') { rnP(plid); hC(); return }
    if (b.dataset.a === 'delete') { rmP(plid); hC(); return }
    hC()
  }
}

function showFavoriteEmptyCtx(e, fvid) {
  const fav = S.favs.find(f => f.id === fvid)
  if (!fav) return
  const m = $('ctx-menu')
  m.innerHTML = `<button data-a="playall">播放此收藏夹</button>${fav.isDefault ? '' : '<button data-a="rename">重命名</button><button data-a="delete" class="danger">删除收藏夹</button>'}`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 120) + 'px'
  m.onclick = function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'playall') { const tks = fav.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean); if (tks.length) playAll(tks); hC(); return }
    if (b.dataset.a === 'rename') { rnF(fvid); hC(); return }
    if (b.dataset.a === 'delete') { rmF(fvid); hC(); return }
    hC()
  }
}

$('#breadcrumb').addEventListener('click', e => {
  const b = e.target.closest('[data-bc="all"]'); if (b) { S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []; S.activeFp = null; S.prevView = null; activeLrcTab = 'lyrics'; renderAll(); schedSave(); return }
  const a = e.target.closest('[data-bc="artist"]'); if (a) { S.alI = -1; renderAll(); schedSave(); return }
  const fpEl = e.target.closest('[data-fp]'); if (fpEl) { navigateFolderTo(fpEl.dataset.fp); return }
  if (e.target.closest('[data-fp-root]')) { S.activeFp = null; S.folderStack = []; S.view = 'all'; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-folder-back')) { navigateFolderUp(); return }
  if (e.target.closest('#btn-lyrics-back')) {
    goBackFromLyrics(); return
  }
  if (e.target.closest('#btn-search-back')) { exitSearch(); return }
  if (e.target.closest('#btn-pl-back')) { S.aPl = null; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-fav-back')) { S.aF = null; renderAll(); schedSave(); return }
})

function exitSearch() {
  $('search-input').value = ''; S.q = ''; S._searchFolders = []; $('search-clear').classList.add('hidden'); $('search-back').classList.remove('visible')
  if (S.prevView) {
    S.view = S.prevView; S.aF = S._prevAF || null; S.aPl = S._prevAPl || null
    // Restore folder navigation state
    if (S._prevFolderStack) { S.folderStack = S._prevFolderStack; S._prevFolderStack = null }
    if (S._prevActiveFp !== undefined) { S.activeFp = S._prevActiveFp; S._prevActiveFp = undefined }
    // Restore the playlist/fav track list
    if (S.aF) { const fav = S.favs.find(f => f.id === S.aF); pl = fav ? fav.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean) : S.all }
    else if (S.aPl) { const plObj = S.pls.find(p => p.id === S.aPl); pl = plObj ? plObj.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean) : S.all }
    else { pl = S.all }
    activeLrcTab = 'lyrics'; S.prevView = null; S._prevAF = null; S._prevAPl = null
  }
  else { pl = S.all; S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = [] }
  syncPlayingState()
  if (S.playingTid) { const idx = pl.findIndex(t => t.id === S.playingTid); S.tI = idx >= 0 ? idx : -1; nI = idx >= 0 ? idx : 0 }
  renderAll(); schedSave()
}

$('#sidebar-nav').addEventListener('click', async e => {
  if (S.q) exitSearch()
  const ni = e.target.closest('.nav-item'); if (ni) { if (ni.dataset.view) { S.prevView = null; S.view = ni.dataset.view; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.activeFp = null; if (ni.dataset.view === 'all') S.folderStack = []; activeLrcTab = 'lyrics'; renderAll(); schedSave(); return } if (ni.id === 'btn-add-folder') { await importFolder(); return } }
  const fa = e.target.closest('[data-fa]'); if (fa) { const k = fa.dataset.fa; const xf = S.xf || new Set(); xf.has(k) ? xf.delete(k) : xf.add(k); S.xf = xf; renderSB(); schedSave(); return }
  const fpEl = e.target.closest('.folder-item[data-fp]'); if (fpEl) {
    S.activeFp = fpEl.dataset.fp
    S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null
    const node = findNodeByPath(S.folderTree, fpEl.dataset.fp)
    S.folderStack = [node ? node.path : fpEl.dataset.fp]
    renderAll(); schedSave(); return
  }
  if (e.target.closest('[data-fvid]')) { S.activeFp = null; S.view = 'all'; S.aF = e.target.closest('[data-fvid]').dataset.fvid; S.aPl = null; S.aI = -1; S.alI = -1; S.prevView = null; renderAll(); schedSave(); return }
  if (e.target.closest('[data-plid]')) { S.activeFp = null; S.view = 'all'; S.aPl = e.target.closest('[data-plid]').dataset.plid; S.aF = null; S.aI = -1; S.alI = -1; S.prevView = null; renderAll(); schedSave(); return }
  if (e.target.id === 'btn-new-fav') { mkF(); return }
  if (e.target.id === 'btn-new-playlist') { mkP(); return }
})
$('#sidebar-nav').addEventListener('dblclick', e => {
  const fvid = e.target.closest('[data-fvid]'); if (fvid) { e.preventDefault(); startRename('fav', fvid.dataset.fvid); return }
  const plid = e.target.closest('[data-plid]'); if (plid) { e.preventDefault(); startRename('pl', plid.dataset.plid); return }
})
$('#sidebar-nav').addEventListener('contextmenu', e => {
  const fi = e.target.closest('.folder-item[data-fp]'); if (fi) {
    e.preventDefault(); showSidebarFolderCtx(e, fi.dataset.fp); return
  }
  const pi = e.target.closest('[data-plid]'); if (pi && pi.classList.contains('playlist-sidebar-item')) {
    e.preventDefault(); showSidebarPlCtx(e, pi.dataset.plid); return
  }
  const fvi = e.target.closest('[data-fvid]'); if (fvi && fvi.classList.contains('fav-sidebar-item')) {
    e.preventDefault(); showSidebarFavCtx(e, fvi.dataset.fvid); return
  }
})

function showSidebarFolderCtx(e, folderPath) {
  return showFolderCtx(e, folderPath)
}

function showSidebarPlCtx(e, plid) {
  const m = $('ctx-menu')
  const plObj = S.pls.find(p => p.id === plid); if (!plObj) return
  m.innerHTML = `<button data-a="srnpl">重命名</button><button data-a="sdelpl" class="danger">删除播放列表</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 200) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px'
  m.onclick = function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'srnpl') { rnP(plid); hC(); return }
    if (b.dataset.a === 'sdelpl') { rmP(plid); hC(); return }
    hC()
  }
  $('ctx-playlist-sub').classList.add('hidden')
}

function showSidebarFavCtx(e, fvid) {
  const m = $('ctx-menu')
  const fav = S.favs.find(f => f.id === fvid); if (!fav) return
  m.innerHTML = `<button data-a="srnfv">重命名</button><button data-a="sdelfv" class="danger">删除收藏夹</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 200) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px'
  m.onclick = function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'srnfv') { rnF(fvid); hC(); return }
    if (b.dataset.a === 'sdelfv') { rmF(fvid); hC(); return }
    hC()
  }
  $('ctx-playlist-sub').classList.add('hidden')
}

$('btn-rescan').addEventListener('click', rescan)
// Player cover always goes to lyrics view
$('player-cover').addEventListener('click', () => {
  if (pl.length > 0 && S.tI >= 0) { if (S.view !== 'lyrics') S.prevView = S.view; S.view = 'lyrics'; activeLrcTab = 'lyrics'; renderAll(); schedSave() }
})

// Playback
$('btn-play').addEventListener('click', () => {
  if (S.playing) {
    if (dsdState.active) {
      dsdState.pausedAt = getPlaybackCurrentTime()
      stopDsdPlayback(false)
    } else {
      audio.pause()
    }
    S.playing = false
  } else {
    if (!pl.length && S.all.length) { pl = S.all; playT(0) }
    else if (pl.length) {
      if (isCurrentTrackDsd()) {
        const track = pl[S.tI >= 0 ? S.tI : 0]
        playDsdTrack(track, dsdState.pausedAt || S.cTime || 0).then(() => {
          S.playing = true
          updPlayBtn()
          updPlayStateClass()
        }).catch(() => {})
      } else {
        audio.play().catch(() => { })
        S.playing = true
      }
    }
  }
  updPlayBtn(); schedSave()
})
$('btn-prev').addEventListener('click', prv)
$('btn-next').addEventListener('click', nxt)
$('btn-mode').addEventListener('click', () => { S.mode = (S.mode + 1) % 4; apMode(); schedSave() })

;(function initVolumeBar() {
  const bar = $('volume-bar')
  let dragging = false

  function setVolFromMouse(e) {
    const r = bar.getBoundingClientRect()
    const p = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width))
    S.vol = Math.round(p * 100); S.pVol = S.vol
    if (S.muted) { S.muted = false }
    audio.volume = S.vol / 100
    $('volume-fill').style.width = S.vol + '%'; $('volume-text').textContent = S.vol
  }

  bar.addEventListener('mousedown', e => {
    e.preventDefault()
    dragging = true
    setVolFromMouse(e)
  })
  document.addEventListener('mousemove', e => {
    if (!dragging) return
    setVolFromMouse(e)
  })
  document.addEventListener('mouseup', e => {
    if (!dragging) return
    dragging = false
    setVolFromMouse(e)
    schedSave()
  })
})()

$('btn-volume').addEventListener('click', () => {
  S.muted = !S.muted
  audio.volume = S.muted ? 0 : S.vol / 100
  syncDsdVolume()
  $('volume-fill').style.width = S.muted ? '0%' : S.vol + '%'
  $('volume-text').textContent = S.muted ? '0' : S.vol; schedSave()
})

// Progress bar dragging state (shared with timeupdate handler)
let _progressDragging = false

;(function initProgressBar() {
  const bar = $('progress-bar')
  const fill = $('progress-fill')
  const handle = $('progress-handle')

  function getRatio(e) {
    const r = bar.getBoundingClientRect()
    return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width))
  }

  function showProgress(ratio) {
    fill.classList.add('no-transition')
    fill.style.width = (ratio * 100) + '%'
    handle.style.left = (ratio * 100) + '%'
    const duration = getPlaybackDuration()
    if (duration) _progressCurrent.textContent = fmtTime(ratio * duration)
  }

  bar.addEventListener('mousedown', e => {
    if (!getPlaybackDuration()) return
    _progressDragging = true
    e.preventDefault()
    showProgress(getRatio(e))
  })

  window.addEventListener('mousemove', e => {
    if (!_progressDragging) return
    showProgress(getRatio(e))
  })

  window.addEventListener('mouseup', e => {
    if (!_progressDragging) return
    _progressDragging = false
    fill.classList.remove('no-transition')
    const nextTime = getRatio(e) * getPlaybackDuration()
    if (isCurrentTrackDsd()) {
      const track = pl[S.tI]
      if (!track) return
      const shouldResume = S.playing
      dsdState.pausedAt = nextTime
      stopDsdPlayback(false)
      S.cTime = nextTime
      $('progress-current').textContent = fmtTime(nextTime)
      if (shouldResume) {
        playDsdTrack(track, nextTime).then(() => {
          S.playing = true
          updPlayBtn()
          updPlayStateClass()
        }).catch(() => {})
      }
    } else {
      audio.currentTime = nextTime
    }
  })
})()
$('search-input').addEventListener('keydown', e => {
  if (e.key === 'Enter') {
    const q = $('search-input').value
    S.q = q; $('search-back').classList.toggle('visible', !!S.q); $('search-clear').classList.toggle('hidden', !S.q); $('search-input').classList.toggle('has-back', !!S.q)
    if (!S.prevView) { S.prevView = S.view; S._prevAF = S.aF; S._prevAPl = S.aPl; S._prevFolderStack = [...S.folderStack]; S._prevActiveFp = S.activeFp }
    S.view = 'all'; S.aF = null; S.aPl = null; S.folderStack = []; S.activeFp = null
    if (S.tI >= 0 && pl[S.tI]) S.playingTid = pl[S.tI].id
    // Search by track metadata (fuzzy: substring + pinyin initials)
    pl = S.all.filter(t => fuzzyMatch(t.name, q) || fuzzyMatch(t.artist, q) || fuzzyMatch(t.metaArtist, q) || fuzzyMatch(t.album, q))
    // Search by folder name (fuzzy)
    function findFoldersByName(nodes) {
      const results = []
      for (const n of nodes) {
        if (fuzzyMatch(n.name, q)) { results.push(n); continue }
        if (n.children.length) results.push(...findFoldersByName(n.children))
      }
      return results
    }
    S._searchFolders = findFoldersByName(S.folderTree)
    syncPlayingState()
    renderAll(); schedSave()
  }
})
$('search-back').addEventListener('click', () => exitSearch())
$('search-clear').addEventListener('click', () => exitSearch())
// Audio events
function updPlayStateClass() {
  syncAllListsPlaying()
}
audio.addEventListener('play', () => { S.playing = true; updPlayBtn(); updPlayStateClass() })
audio.addEventListener('pause', () => { S.playing = false; updPlayBtn(); updPlayStateClass() })
audio.addEventListener('timeupdate', () => {
  if (dsdState.active) return
  if (!S.playing) return; S.cTime = audio.currentTime
  if (_progressDragging) return
  const p = S.dur ? (S.cTime / S.dur) * 100 : 0
  _progressFill.style.width = p + '%'; _progressHandle.style.left = p + '%'
  _progressCurrent.textContent = fmtTime(S.cTime)
  // Update lyrics highlight and auto-scroll
  if (lrc && lrc.length && S.view === 'lyrics' && activeLrcTab === 'lyrics') {
    const scroll = $('lyrics-lines-scroll')
    const lines = scroll ? scroll.querySelectorAll('.lc-line') : []
    let activeIdx = -1
    for (let i = lrc.length - 1; i >= 0; i--) {
      if (S.cTime >= lrc[i].time) { activeIdx = i; break }
    }
    // Only toggle old and new active line instead of iterating all
    if (activeIdx !== _lastLrcActiveIdx) {
      if (_lastLrcActiveIdx >= 0 && _lastLrcActiveIdx < lines.length) lines[_lastLrcActiveIdx].classList.remove('active')
      if (activeIdx >= 0 && activeIdx < lines.length) lines[activeIdx].classList.add('active')
      _lastLrcActiveIdx = activeIdx
    }
    if (activeIdx >= 0 && scroll && !isLyricsManualScrolling()) {
      requestAnimationFrame(() => {
        const activeLine = scroll.querySelector(`.lc-line[data-lidx="${activeIdx}"]`)
        if (activeLine) {
          const containerH = scroll.clientHeight
          const lineH = activeLine.clientHeight
          const layout = scroll.closest('.lyrics-content-layout')
          const layoutH = layout ? layout.clientHeight : containerH
          const layoutCY = layoutH / 2
          const scrollRect = scroll.getBoundingClientRect()
          const layoutRect = layout ? layout.getBoundingClientRect() : scrollRect
          const scrollTopOffset = scrollRect.top - layoutRect.top
          const scrollTarget = activeLine.offsetTop + (lineH / 2) + scrollTopOffset - layoutCY
          const maxScroll = scroll.scrollHeight - containerH
          const finalScroll = Math.max(0, Math.min(scrollTarget, maxScroll))
          scroll.scrollTo({ top: finalScroll, behavior: 'smooth' })
        }
      })
    }
  }
})
audio.addEventListener('loadedmetadata', () => { S.dur = audio.duration; _progressDuration.textContent = fmtTime(S.dur) })
audio.addEventListener('ended', () => { if (!dsdState.active) hEnd() })
audio.addEventListener('error', () => { if (!dsdState.active) { S.playing = false; updPlayBtn() } })

// Modals
$('settings-modal').addEventListener('click', e => {
  if (e.target.closest('.modal-overlay') || e.target.closest('.modal-close')) { $('settings-modal').classList.add('hidden') }
})
$('img-editor-modal').addEventListener('click', e => {
  if (e.target.closest('.modal-overlay') || e.target.closest('.modal-close')) { $('img-editor-modal').classList.add('hidden') }
})

// Theme
$('btn-theme').addEventListener('click', () => { $('settings-modal').classList.remove('hidden'); updSUI(); initColorPicker() })
let themeDebounceTimer
$('theme-dark').addEventListener('click', () => { clearTimeout(themeDebounceTimer); themeDebounceTimer = setTimeout(() => { S.theme = 'dark'; updSUI(); apTh(); schedSave() }, 50) })
$('theme-light').addEventListener('click', () => { clearTimeout(themeDebounceTimer); themeDebounceTimer = setTimeout(() => { S.theme = 'light'; updSUI(); apTh(); schedSave() }, 50) })
let _sidebarOpacityTimer, _titlebarOpacityTimer, _playerOpacityTimer
$('sidebar-opacity').addEventListener('input', e => {
  S.sidebarOpacity = parseInt(e.target.value)
  $('sidebar-opacity-val').textContent = S.sidebarOpacity + '%'
  apTh()
  clearTimeout(_sidebarOpacityTimer)
  _sidebarOpacityTimer = setTimeout(schedSave, 200)
})
$('titlebar-opacity').addEventListener('input', e => {
  S.titlebarOpacity = parseInt(e.target.value)
  $('titlebar-opacity-val').textContent = S.titlebarOpacity + '%'
  apTh()
  clearTimeout(_titlebarOpacityTimer)
  _titlebarOpacityTimer = setTimeout(schedSave, 200)
})
$('player-opacity').addEventListener('input', e => {
  S.playerOpacity = parseInt(e.target.value)
  $('player-opacity-val').textContent = S.playerOpacity + '%'
  apTh()
  clearTimeout(_playerOpacityTimer)
  _playerOpacityTimer = setTimeout(schedSave, 200)
})
// === Color Picker Canvas ===

// CP preset click
$('cp-presets').addEventListener('click', e => {
  const b = e.target.closest('.cp-preset'); if (!b) return
  S.clr = b.dataset.clr; updSUI(); cpSyncFromState(); apTh(); schedSave()
})

// BG
$('bg-preview').addEventListener('click', () => $('btn-bg-upload').click())
$('btn-bg-upload').addEventListener('click', async () => {
  try {
    if (!window.electronAPI || !window.electronAPI.selectBgImage) {
      alert('electronAPI.selectBgImage 不可用，请重启应用')
      return
    }
    const r = await window.electronAPI.selectBgImage()
    if (!r || !r.path) return
    if (!r.dataUrl && r.path) {
      r.dataUrl = await window.electronAPI.readAsDataURL(r.path)
    }
    if (!r.dataUrl) return
    // Save image to app data directory
    const saved = await api.saveBgImage(r.dataUrl)
    if (!saved) return
    S.bgData = r.dataUrl; S.bgPath = 'kx-player-bg.png'
    $('bg-preview').style.backgroundImage = `url(${r.dataUrl})`
    $('bg-preview-wrap').classList.remove('hidden')
    $('btn-bg-upload').classList.add('hidden')
    updSUI(); apTh(); apThBg(); schedSave()
  } catch (e) { alert('选择背景图片失败: ' + e.message) }
})
$('btn-bg-remove').addEventListener('click', async () => {
  S.bgData = null; S.bgPath = null; S.bgBlur = 0
  $('bg-preview').style.backgroundImage = ''
  $('bg-preview-wrap').classList.add('hidden')
  $('btn-bg-upload').classList.remove('hidden')
  await api.removeBgImage()
  idbSet('cache', 'bgImage', null).catch(() => {})
  apThBg(); apTh(); schedSave()
})

// Image editor
let imgDrag = { active: false, startX: 0, startY: 0, posX: 0, posY: 0 }
$('btn-edit-bg').addEventListener('click', () => { if (!S.bgData) return; openImgEditor() })
$('img-zoom').addEventListener('input', e => { $('img-zoom-val').textContent = e.target.value + '%'; applyImgTransform() })
$('img-opacity').addEventListener('input', e => { $('img-opacity-val').textContent = e.target.value + '%'; applyImgTransform() })
$('img-blur').addEventListener('input', e => { $('img-blur-val').textContent = e.target.value + 'px'; applyImgTransform() })
$('btn-zoom-in').addEventListener('click', () => { const r = $('img-zoom'); r.value = Math.min(400, parseInt(r.value) + 10); $('img-zoom-val').textContent = r.value + '%'; applyImgTransform() })
$('btn-zoom-out').addEventListener('click', () => { const r = $('img-zoom'); r.value = Math.max(10, parseInt(r.value) - 10); $('img-zoom-val').textContent = r.value + '%'; applyImgTransform() })
$('img-edit-img').addEventListener('dragstart', e => { e.preventDefault() })
$('btn-img-auto-fit').addEventListener('click', autoFitImg)
$('btn-img-reset').addEventListener('click', () => { imgDrag.posX = 0; imgDrag.posY = 0; autoFitImg() })
$('btn-img-save').addEventListener('click', () => { commitImgChanges(); $('img-editor-modal').classList.add('hidden'); recalcListTextColor() })

// Image editor drag
const imgPreview = $('img-editor-preview')
imgPreview.addEventListener('mousedown', e => {
  e.preventDefault(); imgDrag.active = true
  imgDrag.startX = e.clientX - imgDrag.posX; imgDrag.startY = e.clientY - imgDrag.posY
})
document.addEventListener('mousemove', e => {
  if (!imgDrag.active) return
  imgDrag.posX = e.clientX - imgDrag.startX; imgDrag.posY = e.clientY - imgDrag.startY
  applyImgTransform()
})
document.addEventListener('mouseup', () => { if (imgDrag.active) { imgDrag.active = false; commitImgChanges() } })
imgPreview.addEventListener('wheel', e => {
  e.preventDefault()
  const r = $('img-zoom'); const delta = e.deltaY > 0 ? -5 : 5
  r.value = Math.max(10, Math.min(400, parseInt(r.value) + delta))
  $('img-zoom-val').textContent = r.value + '%'
  applyImgTransform(); commitImgChanges()
})

function openImgEditor() {
  if (!S.bgData) return
  const imgEditEl = $('img-edit-img'), previewEl = $('img-editor-preview')
  const opacityEl = $('img-opacity'), opacityValEl = $('img-opacity-val')
  const blurEl = $('img-blur'), blurValEl = $('img-blur-val')
  const zoomEl = $('img-zoom'), zoomValEl = $('img-zoom-val')
  imgEditEl.src = S.bgData
  const vw = window.innerWidth, vh = window.innerHeight
  previewEl.style.height = Math.round(320 * vh / vw) + 'px'
  S._imgEditState = S._imgEditState || {}
  opacityEl.value = S.ovl; opacityValEl.textContent = S.ovl + '%'
  blurEl.value = S.bgBlur || 0; blurValEl.textContent = (S.bgBlur || 0) + 'px'
  $('img-editor-modal').classList.remove('hidden')
  imgEditEl.onload = () => {
    if (!S._imgEditState.zoomPct) {
      autoFitImg()
    } else {
      applyImgTransform()
    }
  }
  if (S._imgEditState.zoomPct) {
    zoomEl.value = S._imgEditState.zoomPct; zoomValEl.textContent = S._imgEditState.zoomPct + '%'
    // Restore offset: stored as viewport px, convert back to preview px
    const natW = S._imgEditState.natW || 1, natH = S._imgEditState.natH || 1
    const vFill = Math.max(vw / natW, vh / natH)
    const z = S._imgEditState.zoomPct / 100
    const mainImgW = natW * vFill * z
    const previewPh = Math.round(320 * vh / vw)
    const pFill = Math.max(320 / natW, previewPh / natH)
    const previewImgW = natW * pFill * z
    const ratio = previewImgW / mainImgW
    imgDrag.posX = (S._imgEditState.posX || 0) * ratio
    imgDrag.posY = (S._imgEditState.posY || 0) * ratio
  } else if (S._imgEditState.zoom) {
    S._imgEditState.zoomPct = S._imgEditState.zoom; delete S._imgEditState.zoom
    zoomEl.value = S._imgEditState.zoomPct; zoomValEl.textContent = S._imgEditState.zoomPct + '%'
    const natW = S._imgEditState.natW || 1, natH = S._imgEditState.natH || 1
    const z = S._imgEditState.zoomPct / 100
    const previewPh = Math.round(320 * vh / vw)
    const pFill = Math.max(320 / natW, previewPh / natH)
    const previewImgW = natW * pFill * z
    const mainImgW = natW * Math.max(vw / natW, vh / natH) * z
    const ratio = previewImgW / mainImgW
    imgDrag.posX = (S._imgEditState.posX || 0) * ratio
    imgDrag.posY = (S._imgEditState.posY || 0) * ratio
  }
  applyImgTransform()
}

function autoFitImg() {
  const img = $('img-edit-img'), preview = $('img-editor-preview')
  const pw = preview.clientWidth, ph = preview.clientHeight
  if (!pw || !ph) return
  const iw = img.naturalWidth || img.width, ih = img.naturalHeight || img.height
  if (!iw || !ih) return
  $('img-zoom').value = 100; $('img-zoom-val').textContent = '100%'
  imgDrag.posX = 0; imgDrag.posY = 0
  applyImgTransform()
}

function applyImgTransform() {
  const img = $('img-edit-img'), preview = $('img-editor-preview')
  if (!img || !preview) return
  const pw = preview.clientWidth, ph = preview.clientHeight
  if (!pw || !ph) return
  const natW = img.naturalWidth || img.width, natH = img.naturalHeight || img.height
  if (!natW || !natH) return
  const vw = window.innerWidth, vh = window.innerHeight
  const zoomEl = $('img-zoom'), opacityEl = $('img-opacity'), blurEl = $('img-blur')
  const z = parseInt(zoomEl.value) / 100
  // Preview: scale is relative to filling the preview box, proportionally
  const previewFill = Math.max(pw / natW, ph / natH)
  const previewScale = previewFill * z
  const imgW = natW * previewScale, imgH = natH * previewScale
  const opacity = parseInt(opacityEl.value) / 100
  const blur = parseInt(blurEl.value)
  img.style.width = imgW + 'px'; img.style.height = imgH + 'px'
  img.style.left = ((pw - imgW) / 2 + imgDrag.posX) + 'px'
  img.style.top = ((ph - imgH) / 2 + imgDrag.posY) + 'px'
  img.style.opacity = opacity; img.style.filter = blur > 0 ? `blur(${blur}px)` : ''
  const oxPct = pw ? Math.round((-imgDrag.posX * 2 / pw) * 100) : 0
  const oyPct = ph ? Math.round((-imgDrag.posY * 2 / ph) * 100) : 0
  const offsetText = $('img-offset-text')
  if (offsetText) offsetText.textContent = `X: ${oxPct}% / Y: ${oyPct}%`
}

function commitImgChanges() {
  const opacityEl = $('img-opacity'), blurEl = $('img-blur'), zoomEl = $('img-zoom')
  S.ovl = parseInt(opacityEl.value)
  S.bgBlur = parseInt(blurEl.value)
  S._imgEditState = S._imgEditState || {}
  const img = $('img-edit-img')
  const natW = img.naturalWidth || 1, natH = img.naturalHeight || 1
  const vw = window.innerWidth, vh = window.innerHeight
  const z = parseInt(zoomEl.value) / 100
  const preview = $('img-editor-preview')
  const pw = preview.clientWidth, ph = preview.clientHeight
  const pFill = Math.max(pw / natW, ph / natH)
  const vFill = Math.max(vw / natW, vh / natH)
  const previewImgW = natW * pFill * z
  const mainImgW = natW * vFill * z
  const ratio = mainImgW / previewImgW
  S._imgEditState.zoomPct = parseInt(zoomEl.value)
  S._imgEditState.natW = natW
  S._imgEditState.natH = natH
  S._imgEditState.posX = imgDrag.posX * ratio
  S._imgEditState.posY = imgDrag.posY * ratio
  S._imgEditState.vw = vw; S._imgEditState.vh = vh
  schedSave(); apThBg()
}

function collectAllTracks(node) {
  let all = []
  for (const t of node.tracks) all.push(t)
  for (const c of node.children) { collectAllTracksInto(c, all) }
  return all
}
function collectAllTracksInto(node, out) {
  for (const t of node.tracks) out.push(t)
  for (const c of node.children) collectAllTracksInto(c, out)
}

function hasMusicRecursive(node) {
  if (!node) return false
  if (node.tracks && node.tracks.length > 0) return true
  for (const c of (node.children || [])) { if (hasMusicRecursive(c)) return true }
  return false
}

// Precompute folder metadata in a single tree traversal to avoid repeated recursive scans
function buildFolderMeta(tree) {
  const meta = {}
  function walk(node) {
    const p = node.path
    let trackCount = (node.tracks || []).length
    let hasMusic = trackCount > 0
    let coverData = node.coverData || null
    let validChildCount = 0
    for (const c of (node.children || [])) {
      walk(c)
      const childMeta = meta[c.path]
      if (childMeta && childMeta.hasMusic) {
        validChildCount++
        hasMusic = true
        trackCount += childMeta.trackCount
        if (!coverData && childMeta.coverData) coverData = childMeta.coverData
      }
    }
    if (!coverData && node.tracks && node.tracks.length) {
      for (const t of node.tracks) { if (t.coverData) { coverData = t.coverData; break } }
    }
    meta[p] = { trackCount, hasMusic, validChildCount, coverData }
  }
  for (const n of (tree || [])) walk(n)
  return meta
}

// Panel
$('btn-playlist-panel').addEventListener('click', () => {
  const panel = $('playlist-panel')
  const btn = $('btn-playlist-panel')
  if (btn) {
    const rect = btn.getBoundingClientRect()
    const playerBar = $('player-bar')
    const playerHeight = playerBar ? playerBar.offsetHeight : 80
    panel.style.left = (rect.right + 4) + 'px'
    panel.style.bottom = (playerHeight + 8) + 'px'
  }
  panel.classList.remove('hidden')
  renderPanel()
})
$('playlist-overlay').addEventListener('click', () => { $('playlist-panel').classList.add('hidden') })
$('playlist-panel').querySelector('.panel-close').addEventListener('click', () => { $('playlist-panel').classList.add('hidden') })
$('panel-body').addEventListener('click', e => { const t = e.target.closest('.panel-track'); if (t) { const idx = parseInt(t.dataset.pidx); if (!isNaN(idx)) playT(idx) } })

// Volume wheel
function adjVol(delta) {
  S.vol = Math.max(0, Math.min(100, S.vol + delta))
  S.pVol = S.vol; S.muted = false
  audio.volume = S.vol / 100
  $('volume-fill').style.width = S.vol + '%'; $('volume-text').textContent = S.vol
  schedSave()
}
$('volume-bar').addEventListener('wheel', e => { e.preventDefault(); adjVol(e.deltaY > 0 ? -3 : 3) })
$('btn-volume').addEventListener('wheel', e => { e.preventDefault(); adjVol(e.deltaY > 0 ? -3 : 3) })

// Speaker
let spShown = false
$('btn-volume').addEventListener('contextmenu', async e => {
  e.preventDefault()
  if (spShown) { $('speaker-popup').classList.add('hidden'); spShown = false; return }
  try {
    const devs = await navigator.mediaDevices.enumerateDevices(); const audioOut = devs.filter(d => d.kind === 'audiooutput')
    const list = $('speaker-device-list'); list.innerHTML = audioOut.map((d, i) => `<button class="speaker-device-item${S.devId === d.deviceId ? ' active' : ''}" data-did="${d.deviceId}"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" class="speaker-device-icon"><polygon points="11,5 6,9 2,9 2,15 6,15 11,19"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>${esc(d.label || '\u8bbe\u5907 ' + i)}</button>`).join('')
    $('speaker-popup').classList.remove('hidden'); spShown = true
    list.onclick = async ev => {
      const b = ev.target.closest('button'); if (b && b.dataset.did) {
        S.devId = b.dataset.did; if (audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
        schedSave(); $('speaker-popup').classList.add('hidden'); spShown = false
      }
    }
  } catch (e) { /* ignore */ }
})

document.addEventListener('click', e => {
  if (spShown && !e.target.closest('#speaker-popup') && !e.target.closest('#btn-volume')) { $('speaker-popup').classList.add('hidden'); spShown = false }
  if (!_ctxMenu.classList.contains('hidden') && !e.target.closest('#ctx-menu') && !e.target.closest('#ctx-playlist-sub')) hC()
})

// Titlebar
$('btn-import').addEventListener('click', importFolder)
$('btn-min').addEventListener('click', () => api.minimize())
$('btn-max').addEventListener('click', () => api.maximize())
$('btn-close').addEventListener('click', () => api.close())

// Sel mode toggle fix
const selToggle = document.querySelector('.sel-mode-toggle')
if (selToggle) selToggle.addEventListener('click', () => { S.selMode = !S.selMode; renderAll(); schedSave() })

// Window controls events from main
if (api.onMaximized) api.onMaximized(() => { const mi = $('max-icon'); if (mi) mi.innerHTML = '<rect x="5" y="5" width="14" height="14" rx="2"/>'; if (S.bgData) apThBg() })
if (api.onUnmaximized) api.onUnmaximized(() => { const mi = $('max-icon'); if (mi) mi.innerHTML = '<rect x="3" y="3" width="18" height="18" rx="2"/>'; if (S.bgData) apThBg() })

// === Init ===
async function init() {
  try {
    // Phase 1: Load settings (fast) and apply theme immediately
    await loadS(); apTh(); apThBg(); updSUI()
    // Phase 2: Load library/scan data (may be slower, but theme is already visible)
    await loadLibraryData()
    // Save state on exit (guard against double-save from both IPC and beforeunload)
    let _savingOnExit = false
    const saveOnExit = async () => { if (_savingOnExit) return; _savingOnExit = true; clearTimeout(saveTimer); await saveS() }
    api.onBeforeClose(saveOnExit)
    window.addEventListener('beforeunload', saveOnExit)
    let _resizeRAF = null
    let _resizeFlushTimer = null
    window.addEventListener('resize', () => {
      _startResizeThrottle()
      clearTimeout(_resizeFlushTimer)
      _resizeFlushTimer = setTimeout(() => {
        _resizeActive = false
        document.querySelectorAll('.vl-container').forEach(c => { if (c._vlRender) c._vlRender() })
      }, 300)
      if (!_resizeRAF) {
        _resizeRAF = requestAnimationFrame(() => {
          if (S.bgData) apThBg()
          _resizeRAF = null
        })
      }
    })
    if (S.all.length) pl = S.all
    syncPlayingState()
    if (S.tI >= 0 && S.tI < pl.length) {
      const t = pl[S.tI]
      if (t) {
        updPUI(t)
        try {
          if (isDsdTrack(t)) {
            stopDsdPlayback(false)
            await loadLrcForTrack(t)
            if (S.playing) {
              await playDsdTrack(t, S.cTime || 0)
            } else {
              S.dur = t.duration || 0
              _progressDuration.textContent = fmtTime(S.dur)
              S.cTime = S.cTime || 0
              _progressCurrent.textContent = fmtTime(S.cTime)
              dsdState.pausedAt = S.cTime || 0
            }
          } else {
            audio.src = 'file:///' + t.path.replace(/\\/g, '/')
            audio.currentTime = S.cTime || 0
            if (S.devId && audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
          }
        } catch (e) { /* ignore */ }
      }
    }
    apMode(); renderAll()
    audio.volume = S.muted ? 0 : S.vol / 100
    $('volume-fill').style.width = S.muted ? '0%' : S.vol + '%'; $('volume-text').textContent = S.muted ? '0' : S.vol
  } catch (e) {
    console.error('Init error:', e)
  }
}

init()

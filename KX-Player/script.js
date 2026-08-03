import { api } from './api.js'
import { $, esc, arrayMatchSorted, fmtTime, isVideoFile, nName, fuzzyMatchScore, collectAllTracks } from './utils.js'
import { virtualList, virtualFolderList, invalidateVL, _startResizeThrottle, _flushResizeThrottle } from './virtual-list.js'
import { _getCoverData, _loadCoversForTrackIds, _preloadVisibleCovers, _updateFolderTreeCovers, _clearCoverCache, _onCoversLoaded } from './cover.js'
import { createTrackIndex } from './track-index.js'
import { showConfirm, addT, updT, rmT } from './ui-feedback.js'
import { idbGet, idbSet } from './idb-store.js'
import { hex2rgb, h2hsl, hsvToRgb } from './color-utils.js'
import { findBestLyricsMatch } from './lyrics-matcher.js'
import { buildFolderMeta, findNodeByPath, hasMusicRecursive } from './folder-tree.js'

const S = {
  all: [], aI: -1, alI: -1, tI: -1,
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
  _searchFolderTotal: 0,
  _searchBack: null,
  _folderMeta: null, // precomputed folder metadata { path: { trackCount, validChildCount, hasMusic, coverData } }
}

let fp = [], audio = new Audio(), lrc = [], pl = [], nI = 0
const trackIndex = createTrackIndex()
function rebuildTrackIndex() { trackIndex.rebuild(S.all) }
function getTrackById(id) { return trackIndex.get(id) }
function tracksFromIds(ids) { return trackIndex.fromIds(ids) }
let _lastLrcActiveIdx = -1
let _idCounter = 0
let _scanRunning = false
let _pendingRescan = false
let _loadTGeneration = 0
let stopWatchingFs = null
let _watchPollTimer = null
let _watcherTriggered = false

// === Table column sort & resize state ===
// Resizable columns: artist, album, duration (indices into the non-check columns)
// Default widths in px for resizable columns
const _COL_DEFAULTS = { artist: 200, album: 200, duration: 60 }
const _COL_MIN = { artist: 80, album: 80, duration: 48, name: 120 }
let _colWidths = { ..._COL_DEFAULTS } // persisted via schedSave
let _sortCol = null // 'name' | 'artist' | 'album' | 'duration' | null
let _sortDir = 'asc' // 'asc' | 'desc'

function _buildColString() {
  const a = _colWidths.artist + 'px'
  const b = _colWidths.album + 'px'
  const d = _colWidths.duration + 'px'
  return S.selMode
    ? `32px 40px 1fr ${a} ${b} 40px ${d} 48px`
    : `40px 1fr ${a} ${b} 40px ${d} 48px`
}

function _sortArrow(col) {
  if (_sortCol !== col) return ''
  return _sortDir === 'asc'
    ? ' <span class="sort-arrow">\u25B2</span>'
    : ' <span class="sort-arrow">\u25BC</span>'
}

function _tableHeaderHTML() {
  const cols = _buildColString()
  const check = S.selMode ? '<div class="song-row-check"></div>' : ''
  return `<div class="song-row-header" style="grid-template-columns:${cols}">${check}<div class="col-hdr" data-sort-col="idx">#</div><div class="col-hdr col-hdr-sort" data-sort-col="name">\u6587\u4ef6\u540D${_sortArrow('name')}<div class="col-resize-handle" data-resize-col="name"></div></div><div class="col-hdr col-hdr-sort" data-sort-col="artist">\u827A\u672F\u5BB6${_sortArrow('artist')}<div class="col-resize-handle" data-resize-col="artist"></div></div><div class="col-hdr col-hdr-sort" data-sort-col="album">\u4E13\u8F91${_sortArrow('album')}<div class="col-resize-handle" data-resize-col="album"></div></div><div class="col-hdr"></div><div class="col-hdr col-hdr-sort" data-sort-col="duration">\u65F6\u957F${_sortArrow('duration')}<div class="col-resize-handle" data-resize-col="duration"></div></div><div class="col-hdr"></div></div>`
}

function _applySortToTracks(tracks) {
  if (!_sortCol) return tracks
  const dir = _sortDir === 'asc' ? 1 : -1
  const sorted = [...tracks]
  sorted.sort((a, b) => {
    let va, vb
    switch (_sortCol) {
      case 'name': va = nName(a).toLowerCase(); vb = nName(b).toLowerCase(); break
      case 'artist': va = (a.metaArtist || a.artist || '').toLowerCase(); vb = (b.metaArtist || b.artist || '').toLowerCase(); break
      case 'album': va = (a.album || '').toLowerCase(); vb = (b.album || '').toLowerCase(); break
      case 'duration': va = a.duration || 0; vb = b.duration || 0; return (va - vb) * dir
      default: return 0
    }
    if (va < vb) return -1 * dir
    if (va > vb) return 1 * dir
    return 0
  })
  return sorted
}

function _initColumnHandlers(rootEl) {
  if (!rootEl) return
  // Sort click handlers
  rootEl.querySelectorAll('.col-hdr-sort').forEach(el => {
    el.addEventListener('click', (e) => {
      if (e.target.classList.contains('col-resize-handle')) return
      const col = el.dataset.sortCol
      if (_sortCol === col) { _sortDir = _sortDir === 'asc' ? 'desc' : 'asc' }
      else { _sortCol = col; _sortDir = 'asc' }
      renderAll()
    })
  })
  // Resize drag handlers
  rootEl.querySelectorAll('.col-resize-handle').forEach(handle => {
    handle.addEventListener('mousedown', (e) => {
      e.preventDefault()
      e.stopPropagation()
      const col = handle.dataset.resizeCol
      const startX = e.clientX
      const startWidth = _colWidths[col]
      const minWidth = _COL_MIN[col] || 60
      const cols = _buildColString().split(' ')
      // Find which column index this is for adjusting the flex column
      const header = handle.closest('.song-row-header')
      const headerRect = header.getBoundingClientRect()
      const totalWidth = headerRect.width - 32 // account for padding
      function onMove(ev) {
        const delta = ev.clientX - startX
        const newWidth = Math.max(minWidth, startWidth + delta)
        _colWidths[col] = newWidth
        const newCols = _buildColString()
        // Update all grid rows in the table
        const table = rootEl.querySelector('.song-table')
        if (table) {
          table.querySelectorAll('.song-row-header, .song-row').forEach(row => {
            row.style.gridTemplateColumns = newCols
          })
        }
      }
      function onUp() {
        document.removeEventListener('mousemove', onMove)
        document.removeEventListener('mouseup', onUp)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
        schedSave()
      }
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
      document.addEventListener('mousemove', onMove)
      document.addEventListener('mouseup', onUp)
    })
  })
}

let lyricsManualScrollUntil = 0

// Cache frequently accessed DOM elements for performance
const _progressFill = $('progress-fill')
const _progressHandle = $('progress-handle')
const _progressCurrent = $('progress-current')
const _progressDuration = $('progress-duration')

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

function _setTrackDurationById(tid, seconds) {
  const duration = Math.round(Number(seconds) || 0)
  if (!tid || duration <= 0) return false
  let changed = false
  const update = (track) => {
    if (!track || track.id !== tid) return
    const oldDuration = Math.round(Number(track.duration) || 0)
    if (!oldDuration || Math.abs(oldDuration - duration) > 1) {
      track.duration = duration
      changed = true
    }
  }
  update(getTrackById(tid))
  for (const track of pl) update(track)
  const walk = (nodes) => {
    for (const node of (nodes || [])) {
      for (const track of (node.tracks || [])) update(track)
      walk(node.children)
    }
  }
  walk(S.folderTree)
  return changed
}

function _updateVisibleDurationCells(tid, duration) {
  document.querySelectorAll('.song-row').forEach(row => {
    if (row.dataset.tid !== tid) return
    const cell = row.querySelector('.song-row-duration')
    if (cell) cell.textContent = fmtTime(duration)
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

function applyScanResult(result) {
  S.folderTree = result?.folderTree || []
  S._folderMeta = buildFolderMeta(S.folderTree)
  const at = result?.allTracks ? result.allTracks.map(t => ({ ...t })) : []
  S.all = at
  rebuildTrackIndex()
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
    stopWatchingFs = await api.onFsChanged(() => {
      console.log('[watcher] fs changed, triggering rescan')
      _watcherTriggered = true
      rescan()
    })
    _watcherTriggered = false
    await api.startWatching(fp)
    console.log('[watcher] started for', fp.length, 'folders')
  } catch (e) { console.warn('[watcher] restart failed:', e) }
}

function stopWatching() {
  if (_watchPollTimer) { clearInterval(_watchPollTimer); _watchPollTimer = null }
  if (stopWatchingFs) { stopWatchingFs(); stopWatchingFs = null }
  api.stopWatching().catch(() => {})
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
      playingTid: S.playingTid, _colWidths, _sortCol, _sortDir
    }))
    // Sync-save to main process FIRST for immediate disk persistence
    api.syncSaveSettings(data)
    await api.saveSettings(data)
  } catch (e) { /* ignore */ }
}

async function loadS() {
  try {
    let s = await api.loadSettings()
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
    if (s._colWidths && typeof s._colWidths === 'object') {
      for (const k of Object.keys(_COL_DEFAULTS)) {
        if (typeof s._colWidths[k] === 'number' && s._colWidths[k] >= (_COL_MIN[k] || 40)) _colWidths[k] = s._colWidths[k]
      }
    }
    if (s._sortCol && ['name','artist','album','duration'].includes(s._sortCol)) _sortCol = s._sortCol
    if (s._sortDir && (s._sortDir === 'asc' || s._sortDir === 'desc')) _sortDir = s._sortDir

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
    // Use fast load (no cover data) to avoid loading 200MB+ of base64 at startup.
    // Covers are loaded on demand from SQLite (already compressed to 400px thumbnails).
    let library = await api.loadLibraryFast()
    let cacheUsed = false
    if (library && Array.isArray(library.folderPaths)) {
      const libraryPaths = library.folderPaths.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
      if (arrayMatchSorted(fp, libraryPaths)) {
        applyScanResult(library)
        cacheUsed = true
      } else {
        library = null
      }
    }
    if (!library) {
      const result = await api.scanFoldersWithProgress(fp)
      applyScanResult(result)
    }
    const allIds = new Set(S.all.map(t => t.id))
    cleanupStale(allIds)
    // Show the library immediately so the first paint isn't blocked by cover
    // loading or the offline-change sync below.
    pl = S.all
    renderAll()
    console.timeEnd('[startup] total')

    // If we used cached data, run an incremental scan to pick up any file
    // changes that happened while the app was not running (additions,
    // deletions, modifications). This is fast when nothing changed (only
    // file discovery, no re-parsing) and correctly syncs the library.
    if (cacheUsed) runStartupIncrementalSync()
  } catch (e) { /* ignore */ }
  // Start file watcher (separate from library loading so failures don't block each other)
  try { await restartWatching() } catch (e) { console.warn('[watcher] init failed:', e) }

  // Register callback to update UI when covers are loaded asynchronously
  _onCoversLoaded((loadedIds = []) => {
    const loadedSet = new Set(loadedIds)
    const currentTrack = S.tI >= 0 && S.tI < pl.length ? pl[S.tI] : null
    const currentCoverLoaded = currentTrack && (!loadedSet.size || loadedSet.has(currentTrack.id))
    if (!currentCoverLoaded) return

    const coverData = _getCoverData(currentTrack)
    if (!coverData) return

    const coverImgEl = $('player-cover-img')
    const coverEl = $('player-cover')
    if (coverImgEl && coverEl) {
      coverImgEl.src = coverData
      coverImgEl.style.display = ''
      coverEl.querySelector('.cover-placeholder').style.display = 'none'
    }
    if (S.view === 'lyrics') renderLrcContent()
  })
}

function runStartupIncrementalSync() {
  const syncPaths = fp.slice()
  Promise.resolve().then(async () => {
    try {
      console.log('[startup] running incremental scan to sync offline changes...')
      const r = await api.scanFoldersIncremental(syncPaths)
      if (r && r.allTracks) {
        const oldCount = S.all.length
        const newCount = r.allTracks.length
        console.log(`[startup] incremental scan: ${oldCount} -> ${newCount} tracks`)
        applyScanResult(r)
        _clearFolderCoverLazyState()
        const newIds = new Set(S.all.map(t => t.id))
        cleanupStale(newIds)
        renderAll()
      }
    } catch (e) {
      console.warn('[startup] incremental scan failed:', e)
    }
  })
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

    // 1. Try exact name match first (fast path)
    const lrcPath = dir + sep + nameWithoutExt + '.lrc'
    if (await api.fileExists(lrcPath)) {
      lyricsContent = await api.readTextFile(lrcPath)
    }

    // 2. If no exact match, use smart fuzzy matching by listing directory
    if (!lyricsContent) {
      const dirEntries = await api.listDir(dir)
      if (dirEntries && dirEntries.length > 0) {
        const match = findBestLyricsMatch(nameWithoutExt, ext, dirEntries)
        if (match) {
          const matchPath = dir + sep + match.filename
          const matchLower = match.filename.toLowerCase()
          const matchExt = matchLower.slice(matchLower.lastIndexOf('.'))
          if (matchExt === '.lrc') {
            lyricsContent = await api.readTextFile(matchPath)
          } else if (matchExt === '.vtt' || matchExt === '.srt') {
            lyricsContent = await api.readTextFile(matchPath)
            subtitleFormat = matchExt.slice(1)
          }
        }
      }
    }

    // 3. If still no LRC, try VTT and SRT with exact name
    if (!lyricsContent) {
      const subtitleExts = ['.vtt', '.srt']
      for (const subExt of subtitleExts) {
        const doublePath = dir + sep + baseName + subExt
        if (await api.fileExists(doublePath)) {
          lyricsContent = await api.readTextFile(doublePath)
          subtitleFormat = subExt.slice(1)
          break
        }
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
  const needLoad = (idx !== oldTI) || (audio.src === '' || !audio.src.includes(t.path.replace(/\\/g, '/')))
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
  let cd = _getCoverData(t)
  const titleEl = $('player-title'), artistEl = $('player-artist')
  const coverEl = $('player-cover'), coverImgEl = $('player-cover-img')
  titleEl.textContent = nName(t)
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
    // Lazy-load cover if not in memory
    if (t && t.id) _loadCoversForTrackIds([t.id])
  }
  if (!skipLrc && S.view === 'lyrics') renderLrcContent()
}

function hEnd() {
  S.cTime = audio.currentTime; S.dur = audio.duration
  if (isNaN(S.dur)) return
  if (S.mode === 2) {
    audio.currentTime = 0; audio.play().catch(() => { /* ignore */ })
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
  renderSB(); renderContent(); syncPlayingState()
  requestAnimationFrame(() => _initColumnHandlers($('content-area')))
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
      const tks = _setViewPlaylist(tracksFromIds(fav.trackIds))
      ca.innerHTML = renderFContent(fav, tks); return
    }
  }
  if (S.aPl) {
    const plObj = S.pls.find(p => p.id === S.aPl)
    if (plObj) {
      bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-pl-back">← 返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">${esc(plObj.name)}</button>`
      const tks = _setViewPlaylist(tracksFromIds(plObj.trackIds))
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

function _setViewPlaylist(tracks) {
  pl = _applySortToTracks(tracks || [])
  if (S.playingTid) syncPlayingState()
  return pl
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

const _folderCoverPending = []
const _folderCoverPendingSet = new Set()
const _folderCoverLoading = new Set()
const _folderCoverEmpty = new Set()
let _folderCoverTimer = null

function _queueFolderCovers(nodes) {
  if (!nodes || !nodes.length) return
  for (const node of nodes.flat ? nodes.flat() : nodes) {
    if (!node || !node.path) continue
    const meta = S._folderMeta || {}
    const existing = (meta[node.path] || {}).coverData || node.coverData
    if (existing || _folderCoverEmpty.has(node.path) || _folderCoverLoading.has(node.path) || _folderCoverPendingSet.has(node.path)) continue
    _folderCoverPending.push(node.path)
    _folderCoverPendingSet.add(node.path)
  }
  if (!_folderCoverTimer && _folderCoverPending.length) _folderCoverTimer = setTimeout(_flushFolderCovers, 40)
}

function _flushFolderCovers() {
  _folderCoverTimer = null
  const batch = _folderCoverPending.splice(0, 80)
  for (const p of batch) {
    _folderCoverPendingSet.delete(p)
    _folderCoverLoading.add(p)
  }
  if (!batch.length) return
  api.getFolderCovers(batch).then(covers => {
    let changed = false
    for (const p of batch) {
      const cd = covers && covers[p]
      if (cd && S._folderMeta && S._folderMeta[p] && S._folderMeta[p].coverData !== cd) {
        S._folderMeta[p].coverData = cd
        changed = true
      } else if (!cd) {
        _folderCoverEmpty.add(p)
      }
      _folderCoverLoading.delete(p)
    }
    if (changed) {
      _updateFolderTreeCovers(S.folderTree, covers)
      document.querySelectorAll('.virtual-vl').forEach(c => { if (c._vlRender) c._vlRender() })
    }
  }).catch(() => {
    for (const p of batch) _folderCoverLoading.delete(p)
  }).finally(() => {
    if (_folderCoverPending.length && !_folderCoverTimer) _folderCoverTimer = setTimeout(_flushFolderCovers, 40)
  })
}

function _clearFolderCoverLazyState() {
  _folderCoverPending.length = 0
  _folderCoverPendingSet.clear()
  _folderCoverLoading.clear()
  _folderCoverEmpty.clear()
  if (_folderCoverTimer) { clearTimeout(_folderCoverTimer); _folderCoverTimer = null }
}

function _renderVirtualFolders(containerId, folders) {
  if (!folders || !folders.length) return
  if (S.folderView === 'list') {
    virtualFolderList(containerId, folders, 78, folderListRowHTML, visible => _queueFolderCovers(visible))
    return
  }
  // Fixed card size: resizing the window only changes the column count, not
  // the card dimensions, so the grid reshapes quickly without re-measuring.
  const gap = 12
  const cardWidth = 176
  const vl = $(containerId)
  const width = (vl && vl.clientWidth) || 700
  const cols = Math.max(1, Math.floor((width + gap) / (cardWidth + gap)))
  const grid = $(containerId)
  // Column count unchanged: nothing to rebuild, just refresh visible rows.
  if (grid && grid._vlCols === cols && grid._vlRender) { grid._vlRender(); return }
  const rows = []
  for (let i = 0; i < folders.length; i += cols) rows.push(folders.slice(i, i + cols))
  const rowHeight = cardWidth + 68
  virtualFolderList(containerId, rows, rowHeight, (row) => `<div class="virtual-folder-grid-row" style="--fw:${cardWidth}px">${row.map(n => folderCardHTML(n)).join('')}</div>`, visibleRows => _queueFolderCovers(visibleRows.flat()))
  if (grid) { grid._vlCols = cols; grid._vlRebuild = () => _renderVirtualFolders(containerId, folders) }
}

function renderFolderAll() {
  const bc = $('breadcrumb'), ca = $('content-area')
  if (S.q) {
    bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-search-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">搜索结果</button>`
    if (!pl.length && !S._searchFolders.length) { ca.innerHTML = emptyS('未找到匹配的内容', '请尝试其他搜索关键词', false); return }
    let html = ''
    if (S._searchFolders.length) {
      const isList = S.folderView === 'list'
      const totalFolders = S._searchFolderTotal || S._searchFolders.length
      const countLabel = String(totalFolders) + '\u4e2a'
      html += `<div class="section-title">\u6587\u4ef6\u5939<span>${countLabel}</span></div><div class="virtual-vl" id="vt-search-folders"></div>`
    }
    if (pl.length) {
      if (html) html += '<div style="height:20px"></div>'
      const tks = _setViewPlaylist(pl)
      html += `<div class="section-title">音乐<span>${tks.length} 首</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="search"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>播放全部</button>${tableVT('vl-search', tks, (idx) => playT(idx, true))}`
    }
    ca.innerHTML = html
    if (S._searchFolders.length) _renderVirtualFolders('vt-search-folders', S._searchFolders)
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
    ca.innerHTML = `<div class="section-title">\u6587\u4ef6\u5939<span>${validRoots.length} \u4e2a\u6587\u4ef6\u5939</span></div>${folderSortBtnHTML()}<div class="virtual-vl" id="vt-root-folders"></div>`
    _renderVirtualFolders('vt-root-folders', validRoots)
    _restoreFolderScroll()
    return
  }
  const node = findNodeByPath(tree, S.folderStack[S.folderStack.length - 1])
  if (!node) { S.folderStack = []; renderFolderAll(); return }
  renderFolderNode(node)
  _restoreFolderScroll()
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


function _trackRowHTML(t, i, cols) {
  if (!cols) cols = _buildColString()
  const isPlaying = S.playingTid && t.id === S.playingTid
  const playState = isPlaying ? (S.playing ? 'is-playing-state' : 'is-paused-state') : ''
  const isVid = isVideoFile(t)
  const liked = isDefaultFavTrack(t.id)
  return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}">${S.selMode ? `<div class="song-row-check"><input type="checkbox" data-tid="${t.id}" /></div>` : ''}<div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(nName(t))}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${liked ? ' liked' : ''}" data-tid="${t.id}">${liked ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
}

function tableVT(containerId, tracks, onClick) {
  const cols = _buildColString()
  const header = _tableHeaderHTML()
  const html = `<div class="song-table">${header}<div class="vl-container" id="${containerId}"></div></div>`
  requestAnimationFrame(() => {
    if (!tracks.length) {
      const c = $(containerId)
      if (c) c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u6682\u65e0\u5185\u5bb9</h3></div>'
      return
    }
    virtualList(containerId, tracks, 46, (t, i) => _trackRowHTML(t, i, cols), (tid, keepView) => { if (onClick) { const idx = tracks.findIndex(tk => tk.id === tid); if (idx >= 0) onClick(idx, keepView) } }, visible => _preloadVisibleCovers(visible))
  })
  return html
}

function emptyS(title, desc, btn) {
  return `<div class="empty-state"><div class="empty-state-icon"><svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div><h3>${title}</h3><p>${desc}</p>${btn ? `<button class="btn-primary" id="empty-import"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>\u5bfc\u5165\u6587\u4ef6\u5939</button>` : ''}</div>`
}

// === Folder View ===
function renderFolderNode(node) {
  const bc = $('breadcrumb')
  let bcHtml = ''
  if (S.folderStack.length > 0) {
    bcHtml = `<button class="btn-breadcrumb-back" id="btn-folder-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span>`
  }
  bcHtml += `<button class="breadcrumb-item" data-fp-root="">\u5168\u90e8\u97f3\u4e50</button>`
  for (let i = 0; i < S.folderStack.length; i++) {
    const fp_i = S.folderStack[i]
    const fn = findNodeByPath(S.folderTree, fp_i)
    const nm = fn ? fn.name : fp_i.split(/[\\/]/).pop()
    bcHtml += `<span class="breadcrumb-sep">/</span><button class="breadcrumb-item${i === S.folderStack.length - 1 ? ' current' : ''}" data-fp="${esc(fp_i)}">${esc(nm)}</button>`
  }
  bc.innerHTML = bcHtml

  const meta = S._folderMeta || {}
  const validChildren = sortFolders(node.children.filter(c => meta[c.path]?.hasMusic))
  let html = ''

  if (validChildren.length > 0) {
    html += `<div class="section-title">\u5b50\u6587\u4ef6\u5939<span>${validChildren.length} \u4e2a</span></div>${folderSortBtnHTML()}<div class="virtual-vl" id="vt-child-folders"></div>`
  }

  if (node.tracks.length > 0) {
    const allTracks = _setViewPlaylist([...node.tracks])
    const fHeader = _tableHeaderHTML()
    html += `<div class="section-title" style="margin-top:${validChildren.length > 0 ? '24px' : '0'}">\u97f3\u4e50<span>${allTracks.length} \u9996</span></div><button class="btn-primary" style="margin-bottom:12px" data-pfolder="${esc(node.path)}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button><div class="song-table">${fHeader}<div class="vl-container" id="vl-songs"></div></div>`
  }

  $('content-area').innerHTML = html || '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u7a7a\u6587\u4ef6\u5939</h3></div>'
  if (validChildren.length > 0) _renderVirtualFolders('vt-child-folders', validChildren)
  if (node.tracks.length > 0) {
    const fCols = _buildColString()
    virtualList('vl-songs', pl, 46, (t, i) => _trackRowHTML(t, i, fCols), (tid, keepView) => {
      const idx = pl.findIndex(t => t.id === tid)
      if (idx >= 0) playT(idx, keepView)
    }, visible => _preloadVisibleCovers(visible))
  }
}


// Remember scroll position per folder level so going back restores it
const _folderScrollPos = new Map()
function _folderScrollKey() { return S.folderStack.join('\x00') }
function _saveFolderScroll() {
  const ca = $('content-area')
  if (ca) _folderScrollPos.set(_folderScrollKey(), ca.scrollTop)
}
function _restoreFolderScroll() {
  const ca = $('content-area')
  if (!ca) return
  const saved = _folderScrollPos.get(_folderScrollKey())
  if (saved !== undefined) { ca.scrollTop = saved }
}

// Drop the active search UI, remembering it so the back button can return to
// the results list if the user navigated here from a search result.
function exitSearchForNav() {
  if (!S.q) return
  if (!S._searchBack) {
    S._searchBack = { q: S.q, folders: S._searchFolders || [], folderTotal: S._searchFolderTotal || 0, pl: pl.slice() }
  }
  $('search-input').value = ''; S.q = ''; S._searchFolders = []; S._searchFolderTotal = 0
  const sc = $('search-clear'); if (sc) sc.classList.add('hidden')
  const sb = $('search-back'); if (sb) sb.classList.remove('visible')
  const si = $('search-input'); if (si) si.classList.remove('has-back')
}

function navigateFolder(path) {
  const node = findNodeByPath(S.folderTree, path)
  if (!node) return
  // Coming from a search result: remember the search so the back button can
  // return to the results list, then drop the active search UI for the folder view.
  exitSearchForNav()
  _saveFolderScroll()
  S.activeFp = node.path
  if (S.folderStack[S.folderStack.length - 1] !== node.path) S.folderStack.push(node.path)
  S.view = 'all'; renderAll(); schedSave()
}

// Restore a search-results context after backing out of a folder that was
// opened from a search result.
function restoreSearchOrigin() {
  const o = S._searchBack
  if (!o) return false
  S._searchBack = null
  S.q = o.q
  S._searchFolders = o.folders || []
  S._searchFolderTotal = o.folderTotal || S._searchFolders.length
  if (Array.isArray(o.pl) && o.pl.length) _setViewPlaylist(o.pl)
  const si = $('search-input'); if (si) { si.value = o.q; si.classList.add('has-back') }
  const sb = $('search-back'); if (sb) sb.classList.add('visible')
  const sc = $('search-clear'); if (sc) sc.classList.remove('hidden')
  return true
}

function navigateFolderUp() {
  _saveFolderScroll()
  if (S._searchBack && S.folderStack.length <= 1) {
    restoreSearchOrigin()
    S.folderStack = []; S.activeFp = null
    S.view = 'all'; renderAll(); schedSave(); return
  }
  if (S.folderStack.length <= 1) { S.folderStack = []; S.activeFp = null }
  else { S.folderStack.pop() }
  S.view = 'all'; renderAll(); schedSave()
}

function navigateFolderTo(path) {
  _saveFolderScroll()
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
  const tks = _setViewPlaylist(tracksFromIds(S.recents))
  if (!tks.length) { $('content-area').innerHTML = emptyS('\u8fd8\u6ca1\u6709\u64ad\u653e\u8bb0\u5f55', '\u5f00\u59cb\u64ad\u653e\u97f3\u4e50\u540e\u4f1a\u81ea\u52a8\u8bb0\u5f55', false); return }
  $('content-area').innerHTML = `<div class="section-title">\u6700\u8fd1\u64ad\u653e<span>${tks.length} \u9996</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="recent"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button>${tableVT('vl-recent', tks, (idx) => playT(idx, true))}`
}

// === Lyrics ===
let activeLrcTab = 'lyrics'
function renderLrcContent() {
  _lastLrcActiveIdx = -1
  const t = getTrackById(S.playingTid)
  const container = $('content-area')
  if (!container) return
  const cd = t ? _getCoverData(t) : null
  const lrcHtml = buildLrcLines(lrc)
  container.innerHTML = t ? `<div class="lyrics-page-actions"><button class="lyrics-action-btn" data-lact="folder">\u6240\u5728\u6587\u4ef6\u5939</button><button class="lyrics-action-btn" data-lact="copy">\u590d\u5236\u8def\u5f84</button></div><div class="lyrics-content-layout"><div class="lyrics-content-left"><div class="lyrics-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="56" height="56" fill="none" stroke="currentColor" stroke-width="1"><circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/></svg></div>'}</div></div><div class="lyrics-content-right"><div class="lyrics-content-info"><div class="lc-title">${esc(nName(t))}</div><div class="lc-artist">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}${isVideoFile(t) ? ' \u00b7 \u89c6\u9891-\u4ec5\u97f3\u9891\u6a21\u5f0f' : ''}</div></div><div class="lyrics-tab-btns"><button class="lyrics-tab-btn${activeLrcTab === 'lyrics' ? ' active' : ''}" data-ltab="lyrics">\u6b4c\u8bcd</button><button class="lyrics-tab-btn${activeLrcTab === 'meta' ? ' active' : ''}" data-ltab="meta">\u4fe1\u606f</button></div><div class="lyrics-container-wrapper"><div class="lyrics-lines-scroll${activeLrcTab !== 'lyrics' ? ' hidden' : ''}" id="lyrics-lines-scroll">${lrcHtml || '<div class="lc-empty">\u6682\u65e0\u6b4c\u8bcd</div>'}</div><div class="lyrics-meta-panel${activeLrcTab !== 'meta' ? ' hidden' : ''}" id="lyrics-meta-panel"><div class="meta-row"><span class="meta-label">\u6587\u4ef6\u540d</span><span class="meta-value">${esc(nName(t))}</span></div><div class="meta-row"><span class="meta-label">\u827a\u672f\u5bb6</span><span class="meta-value">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}</span></div><div class="meta-row"><span class="meta-label">\u4e13\u8f91</span><span class="meta-value">${esc(t.album || '')}</span></div><div class="meta-row"><span class="meta-label">\u683c\u5f0f</span><span class="meta-value">${t.format.toUpperCase()}${isVideoFile(t) ? ' (\u89c6\u9891)' : ''}</span></div><div class="meta-row"><span class="meta-label">\u65f6\u957f</span><span class="meta-value">${fmtTime(t.duration)}</span></div><div class="meta-row"><span class="meta-label">\u6587\u4ef6</span><span class="meta-value">${esc(t.path)}</span></div></div></div></div></div>` : '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u672a\u5728\u64ad\u653e</h3></div>'
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

function _scrollLyricToCenter(lineEl, behavior = 'smooth') {
  const scroll = $('lyrics-lines-scroll')
  if (!scroll || !lineEl) return
  const containerH = scroll.clientHeight
  const lineH = lineEl.clientHeight
  const layout = scroll.closest('.lyrics-content-layout')
  const layoutH = layout ? layout.clientHeight : containerH
  const layoutCY = layoutH / 2
  const scrollRect = scroll.getBoundingClientRect()
  const layoutRect = layout ? layout.getBoundingClientRect() : scrollRect
  const scrollTopOffset = scrollRect.top - layoutRect.top
  const scrollTarget = lineEl.offsetTop + (lineH / 2) + scrollTopOffset - layoutCY
  const maxScroll = scroll.scrollHeight - containerH
  const finalScroll = Math.max(0, Math.min(scrollTarget, maxScroll))
  scroll.scrollTo({ top: finalScroll, behavior })
}

function clearRecents() {
  S.recents = []
  if (S.view === 'recent') renderAll()
  else schedSave()
}

function goToTrackFolder(tid) {
  const track = getTrackById(tid)
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
  // Leave the search results view the same way folder navigation does, and
  // remember the search so the back button can return to the results list.
  exitSearchForNav()
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
function a2P(pid, tid) { const p = S.pls.find(x => x.id === pid); if (!p || p.trackIds.includes(tid)) return; p.trackIds.push(tid); if (!p.coverData) { const t = getTrackById(tid); if (t) p.coverData = _getCoverData(t) } schedSave(); renderAll() }
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
  const td = tks.find(t => _getCoverData(t)), cd = td ? _getCoverData(td) : null
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">\u64ad\u653e\u5217\u8868</div><div class="pl-content-name" data-plid="${plObj.id}" title="\u53cc\u51fb\u91cd\u547d\u540d">${esc(plObj.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-ppl="${plObj.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button><button class="btn-danger" data-delpl="${plObj.id}">\u5220\u9664</button></div></div></div>${tableVT('vl-pl-' + plObj.id, tks, (idx) => playT(idx, true))}`
}
function renderFContent(fav, tks) {
  const td = tks.find(t => _getCoverData(t)), cd = td ? _getCoverData(td) : null
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">收藏夹</div><div class="pl-content-name" data-fvid="${fav.id}" title="${fav.isDefault ? '默认收藏夹' : '双击重命名'}">${esc(fav.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-pfav="${fav.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>播放全部</button>${!fav.isDefault ? `<button class="btn-danger" data-delfv="${fav.id}">删除</button>` : ''}</div></div></div>${tableVT('vl-fv-' + fav.id, tks, (idx) => playT(idx, true))}`
}

// === Scan ===
async function importFolder() {
  if (_scanRunning) return
  _scanRunning = true
  try {
    const result = await api.openFolder(); if (!result || !result.length) return
    const normalized = result.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
    const newPaths = normalized.filter(p => !fp.includes(p))
    if (!newPaths.length) return // all folders already imported
    for (const p of newPaths) fp.push(p)
    const tid = addT('正在扫描音乐文件...')
    api.removeScanProgressListener()
    let removeProgress = null
    removeProgress = api.onScannerProgress((data) => { updT(tid, `${data.stage || '解析中...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
    // Only scan the new folders (incremental scan)
    const r = await api.scanFoldersIncremental(fp)
    const at = applyScanResult(r)
    _clearFolderCoverLazyState()
    const allIds = new Set(at.map(t => t.id))
    cleanupStale(allIds)
    S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []; S.activeFp = null
    _setViewPlaylist(at)
    if (S.playingTid && !allIds.has(S.playingTid)) { S.playingTid = null; S.playing = false; audio.pause(); lrc = [] }
    if (r.fileCount > 0) { updT(tid, '完成✔', 100, `共 ${r.fileCount || at.length} 首音乐`); rmT(tid) } else { updT(tid, '未找到音乐', 0, '请检查文件夹内容'); setTimeout(() => rmT(tid), 5000) }
    await restartWatching()
    renderAll()
  } catch (e) { alert('导入失败: ' + e.message) }
  finally { _scanRunning = false; removeProgress?.() }
}

let _scanGeneration = 0
async function rescan() {
  // If a scan is already running, bump generation to invalidate it and start fresh
  if (_scanRunning) { _scanGeneration++; _pendingRescan = true; return }
  if (!fp.length) {
    S.all = []; rebuildTrackIndex(); S.folderTree = []; S.folderStack = []; S._folderMeta = null
    pl = []; S.playingTid = null; S.tI = -1; audio.pause()
    cleanupStale(new Set())
    renderAll(); schedSave()
    return
  }
  const isWatchTriggered = _watcherTriggered
  const tid = addT(isWatchTriggered ? '\u6587\u4ef6\u53d8\u52a8\uff0c\u66f4\u65b0\u4e2d...' : '\u91cd\u65b0\u626b\u63cf...')
  _watcherTriggered = false
  _scanRunning = true
  const thisGen = ++_scanGeneration
  api.removeScanProgressListener()
  const removeProgress = api.onScannerProgress((data) => { if (_scanGeneration === thisGen) updT(tid, `${data.stage || '\u89e3\u6790\u4e2d...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
  const removeStage = api.onScannerStage((stage) => { if (_scanGeneration === thisGen) { const e = document.getElementById(tid + '-status'); if (e) e.textContent = stage } })
  try {
    const currentTrackId = pl.length > 0 && S.tI >= 0 ? pl[S.tI]?.id : null
    console.time('[total] scan->show')
    console.time('[scan] IPC wait')
    // Use incremental scan for watcher-triggered changes, full scan for manual rescan
    const r = isWatchTriggered ? await api.scanFoldersIncremental(fp) : await api.scanFoldersWithProgress(fp)
    console.timeEnd('[scan] IPC wait')
    // Check if this scan was superseded by a newer one
    if (_scanGeneration !== thisGen) {
      console.log('[scan] generation mismatch, discarding stale results')
      return
    }
    console.time('[scan] applyAndRender')
    const at = applyScanResult(r)
    const allIds = new Set(at.map(t => t.id))
    cleanupStale(allIds)
    if (S.aF) {
      const fav = S.favs.find(f => f.id === S.aF)
      _setViewPlaylist(fav ? tracksFromIds(fav.trackIds) : at)
    } else if (S.aPl) {
      const plObj = S.pls.find(p => p.id === S.aPl)
      _setViewPlaylist(plObj ? tracksFromIds(plObj.trackIds) : at)
    } else {
      _setViewPlaylist(at)
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
  } catch (e) { if (_scanGeneration === thisGen) updT(tid, '\u5931\u8d25', 0, e.message) }
  finally {
    _scanRunning = false; removeProgress?.(); removeStage?.()
    if (_pendingRescan && _scanGeneration === thisGen) { _pendingRescan = false; rescan() }
    else { _pendingRescan = false }
  }
}

// === Panel ===
function renderPanel() {
  const b = $('panel-body')
  if (!pl.length) { b.innerHTML = '<div class="panel-empty">\u64ad\u653e\u5217\u8868\u4e3a\u7a7a</div>'; $('panel-count').textContent = '0 \u9996'; return }
  $('panel-count').textContent = pl.length + ' \u9996'
  b.innerHTML = pl.map((t, i) => `<div class="panel-track${i === nI ? ' playing' : ''}" data-pidx="${i}"><span class="pt-idx">${i + 1}</span><div class="pt-info"><div class="pt-title">${esc(nName(t))}</div><div class="pt-artist">${esc(t.metaArtist || t.artist)}</div></div></div>`).join('')
}

function playAll(tracks) { if (!tracks || !tracks.length) return; pl = tracks; syncPlayingState(); if (S.view !== 'lyrics') S.prevView = S.view; S.view = 'lyrics'; activeLrcTab = 'lyrics'; playT(0); renderPanel() }

// === ctx ===
function showCtx(e, ci) {
  const m = $('ctx-menu'), ps = $('ctx-playlist-sub')
  const groups = []

  if (ci?.tid) {
    const track = getTrackById(ci.tid)
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
    if (b.dataset.a === 'copyname' && ci?.tid) { const track = getTrackById(ci.tid); if (track) copyText(track.name, '已复制歌名'); hC(); return }
    if (b.dataset.a === 'copypath' && ci?.tid) { const track = getTrackById(ci.tid); if (track) copyText(track.path, '已复制路径'); hC(); return }
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
    const currentTrack = getTrackById(S.playingTid)
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
  if (e.target.closest('[data-ppl]')) { const el = e.target.closest('[data-ppl]'); const pid = el.dataset.ppl; const p = S.pls.find(x => x.id === pid); if (p) { const tks = S.aPl === pid ? pl : _applySortToTracks(tracksFromIds(p.trackIds)); playAll(tks) } return }
  if (e.target.closest('[data-pfav]')) { const el = e.target.closest('[data-pfav]'); const fid = el.dataset.pfav; const f = S.favs.find(x => x.id === fid); if (f) { const tks = S.aF === fid ? pl : _applySortToTracks(tracksFromIds(f.trackIds)); playAll(tks) } return }
  if (e.target.closest('[data-pall]')) { if (pl.length) playAll(pl); return }
  if (e.target.closest('[data-pfolder]')) {
    const el = e.target.closest('[data-pfolder]'); const fpPath = el.dataset.pfolder; const n = findNodeByPath(S.folderTree, fpPath)
    if (n) { const all = _applySortToTracks(collectAllTracks(n)); if (all.length) playAll(all) }
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
      audio.currentTime = time.time
      // Immediately scroll clicked lyric line to center (no delay)
      lyricsManualScrollUntil = 0
      _scrollLyricToCenter(lcl, 'auto')
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
  const fpEl = e.target.closest('[data-fp]'); if (fpEl && fpEl.dataset.fp) { e.preventDefault(); showFolderCtx(e, fpEl.dataset.fp); return }
  if (S.view === 'lyrics' && S.playingTid) { e.preventDefault(); showCtx(e, { tid: S.playingTid, pid: S.aPl, fid: S.aF }); return }
  if (S.view === 'all' && S.folderStack && S.folderStack.length) { e.preventDefault(); showFolderCtx(e, S.folderStack[S.folderStack.length - 1]); return }
  if (S.view === 'recent') { e.preventDefault(); showRecentCtx(e); return }
  if (S.aPl) { e.preventDefault(); showPlaylistEmptyCtx(e, S.aPl); return }
  if (S.aF) { e.preventDefault(); showFavoriteEmptyCtx(e, S.aF); return }
})

function showFolderCtx(e, folderPath) {
  const m = $('ctx-menu')
  const folderName = folderPath.split(/[\\/]/).pop() || folderPath
  m.innerHTML = `<button data-a="fopen">打开文件夹</button><button data-a="fshow">在文件资源管理器中打开</button><button data-a="fcopy">复制文件夹路径</button><hr><button data-a="frescan">扫描此文件夹</button><button data-a="frem" class="danger">从扫描列表移除“${esc(folderName)}”</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 220) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 160) + 'px'
  m.onclick = async function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'fopen') { S.activeFp = folderPath; S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = [folderPath]; renderAll(); schedSave(); hC(); return }
    if (b.dataset.a === 'fshow') { await api.showItemInFolder(folderPath); hC(); return }
    if (b.dataset.a === 'fcopy') { copyText(folderPath, '已复制文件夹路径'); hC(); return }
    if (b.dataset.a === 'frescan') {
      hC()
      if (_scanRunning) return
      _scanRunning = true
      const tid = addT('扫描文件夹...')
      api.removeScanProgressListener()
      const removeProgress = api.onScannerProgress((data) => { updT(tid, `${data.stage || '解析中...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
      try {
        const r = await api.scanFoldersIncremental(fp)
        const at = applyScanResult(r)
        _clearFolderCoverLazyState()
        const allIds = new Set(at.map(t => t.id))
        cleanupStale(allIds)
        if (S.aF) {
          const fav = S.favs.find(f => f.id === S.aF)
          _setViewPlaylist(fav ? tracksFromIds(fav.trackIds) : at)
        } else if (S.aPl) {
          const plObj = S.pls.find(p => p.id === S.aPl)
          _setViewPlaylist(plObj ? tracksFromIds(plObj.trackIds) : at)
        } else { _setViewPlaylist(at) }
        if (S.playingTid && !allIds.has(S.playingTid)) { S.playingTid = null; S.playing = false; audio.pause(); lrc = [] }
        updT(tid, '完成✔', 100, `共 ${r.fileCount || at.length} 首`)
        rmT(tid)
        await restartWatching()
        renderAll()
      } catch (err) { updT(tid, '失败', 0, err.message) }
      finally { _scanRunning = false; removeProgress?.() }
      return
    }
    if (b.dataset.a === 'frem') {
      hC()
      const ok = await showConfirm('移除文件夹', `确定从扫描列表移除文件夹“${folderName}”吗？`)
      if (!ok) return
      fp = fp.filter(p => p !== folderPath)
      const r = await api.removeFolder(folderPath, fp)
      if (r && r.allTracks) {
        applyScanResult(r)
        _clearFolderCoverLazyState()
        const allIds = new Set(r.allTracks.map(t => t.id))
        cleanupStale(allIds)
        pl = pl.filter(t => allIds.has(t.id))
        if (S.playingTid && !allIds.has(S.playingTid)) { S.playingTid = null; S.playing = false; audio.pause(); lrc = [] }
      }
      await restartWatching()
      schedSave()
      renderAll()
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
      const tks = _applySortToTracks(tracksFromIds(S.recents))
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
    if (b.dataset.a === 'playall') { const plObj = S.pls.find(p => p.id === plid); const tks = plObj ? _applySortToTracks(tracksFromIds(plObj.trackIds)) : []; if (tks.length) playAll(tks); hC(); return }
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
    if (b.dataset.a === 'playall') { const tks = _applySortToTracks(tracksFromIds(fav.trackIds)); if (tks.length) playAll(tks); hC(); return }
    if (b.dataset.a === 'rename') { rnF(fvid); hC(); return }
    if (b.dataset.a === 'delete') { rmF(fvid); hC(); return }
    hC()
  }
}

$('#breadcrumb').addEventListener('click', e => {
  const b = e.target.closest('[data-bc="all"]'); if (b) { S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []; S.activeFp = null; S.prevView = null; S._searchBack = null; activeLrcTab = 'lyrics'; renderAll(); schedSave(); return }
  const a = e.target.closest('[data-bc="artist"]'); if (a) { S.alI = -1; renderAll(); schedSave(); return }
  const fpEl = e.target.closest('[data-fp]'); if (fpEl) { navigateFolderTo(fpEl.dataset.fp); return }
  if (e.target.closest('[data-fp-root]')) { S.activeFp = null; S.folderStack = []; S.view = 'all'; S._searchBack = null; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-folder-back')) { navigateFolderUp(); return }
  if (e.target.closest('#btn-lyrics-back')) {
    goBackFromLyrics(); return
  }
  if (e.target.closest('#btn-search-back')) { exitSearch(); return }
  if (e.target.closest('#btn-pl-back')) { S.aPl = null; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-fav-back')) { S.aF = null; renderAll(); schedSave(); return }
})

function exitSearch() {
  $('search-input').value = ''; S.q = ''; S._searchFolders = []; S._searchFolderTotal = 0; S._searchBack = null; $('search-clear').classList.add('hidden'); $('search-back').classList.remove('visible'); $('search-input').classList.remove('has-back')
  if (S.prevView) {
    S.view = S.prevView; S.aF = S._prevAF || null; S.aPl = S._prevAPl || null
    // Restore folder navigation state
    if (S._prevFolderStack) { S.folderStack = S._prevFolderStack; S._prevFolderStack = null }
    if (S._prevActiveFp !== undefined) { S.activeFp = S._prevActiveFp; S._prevActiveFp = undefined }
    // Restore the playlist/fav track list
    if (S.aF) { const fav = S.favs.find(f => f.id === S.aF); _setViewPlaylist(fav ? tracksFromIds(fav.trackIds) : S.all) }
    else if (S.aPl) { const plObj = S.pls.find(p => p.id === S.aPl); _setViewPlaylist(plObj ? tracksFromIds(plObj.trackIds) : S.all) }
    else { _setViewPlaylist(S.all) }
    activeLrcTab = 'lyrics'; S.prevView = null; S._prevAF = null; S._prevAPl = null
  }
  else { _setViewPlaylist(S.all); S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = [] }
  syncPlayingState()
  if (S.playingTid) { const idx = pl.findIndex(t => t.id === S.playingTid); S.tI = idx >= 0 ? idx : -1; nI = idx >= 0 ? idx : 0 }
  renderAll(); schedSave()
}

$('#sidebar-nav').addEventListener('click', async e => {
  if (S.q) exitSearch()
  S._searchBack = null
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
  // Open lyrics whenever a track exists, even if it is not in the currently
  // displayed list (e.g. search results or another folder view).
  const hasTrack = (S.tI >= 0 && S.tI < pl.length && pl[S.tI]) || S.playingTid
  if (!hasTrack) return
  if (S.view !== 'lyrics') S.prevView = S.view
  S.view = 'lyrics'
  activeLrcTab = 'lyrics'
  renderAll(); schedSave()
})

// Playback
$('btn-play').addEventListener('click', () => {
  if (S.playing) {
    audio.pause()
    S.playing = false
  } else {
    if (!pl.length && S.all.length) { pl = S.all; playT(0) }
    else if (pl.length) {
      audio.play().catch(() => { })
      S.playing = true
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
  $('volume-fill').style.width = S.muted ? '0%' : S.vol + '%'
  $('volume-text').textContent = S.muted ? '0' : S.vol; schedSave()
})

// Progress bar dragging state (shared with timeupdate handler)
let _progressDragging = false

// Playback duration for the seek bar: prefer the live audio duration, fall back to saved state.
function getPlaybackDuration() {
  const d = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : S.dur
  return d && d > 0 ? d : 0
}

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
    audio.currentTime = nextTime
  })
})()
$('search-input').addEventListener('keydown', e => {
  if (e.key === 'Enter') {
    const q = $('search-input').value
    S._searchBack = null
    S.q = q; $('search-back').classList.toggle('visible', !!S.q); $('search-clear').classList.toggle('hidden', !S.q); $('search-input').classList.toggle('has-back', !!S.q)
    if (!S.prevView) { S.prevView = S.view; S._prevAF = S.aF; S._prevAPl = S.aPl; S._prevFolderStack = [...S.folderStack]; S._prevActiveFp = S.activeFp }
    S.view = 'all'; S.aF = null; S.aPl = null; S.folderStack = []; S.activeFp = null
    if (S.tI >= 0 && pl[S.tI]) S.playingTid = pl[S.tI].id
    // Search by track metadata: score relevance across fields, rank by match degree
    const scored = []
    const fieldWeights = [['name', 1.0], ['artist', 0.95], ['metaArtist', 0.9], ['album', 0.8]]
    for (const t of S.all) {
      let best = -1
      for (const [f, w] of fieldWeights) {
        const s = fuzzyMatchScore(t[f], q)
        if (s >= 0 && s * w > best) best = s * w
      }
      if (best >= 0) scored.push({ t, s: best })
    }
    scored.sort((a, b) => b.s - a.s)
    _setViewPlaylist(scored.map(o => o.t))
    // Search by folder name: score then rank, keep top 400 but count all real matches
    function findFoldersByName(nodes) {
      const matched = []
      const walk = (ns) => {
        for (const n of ns) {
          const s = fuzzyMatchScore(n.name, q)
          if (s >= 0) matched.push({ n, s })
          if (n.children.length) walk(n.children)
        }
      }
      walk(nodes)
      matched.sort((a, b) => b.s - a.s)
      return { nodes: matched.map(o => o.n), total: matched.length }
    }
    const found = findFoldersByName(S.folderTree)
    S._searchFolders = found.nodes
    S._searchFolderTotal = found.total
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
        if (activeLine) _scrollLyricToCenter(activeLine, 'smooth')
      })
    }
  }
})
audio.addEventListener('loadedmetadata', () => {
  if (!Number.isFinite(audio.duration) || audio.duration <= 0) return
  S.dur = audio.duration
  _progressDuration.textContent = fmtTime(S.dur)
  const tid = S.playingTid || (S.tI >= 0 ? pl[S.tI]?.id : null)
  const duration = Math.round(audio.duration)
  if (tid && _setTrackDurationById(tid, duration)) {
    _updateVisibleDurationCells(tid, duration)
    if (S.view === 'lyrics') renderLrcContent()
  }
})
audio.addEventListener('ended', () => { hEnd() })
audio.addEventListener('error', () => { S.playing = false; updPlayBtn() })

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

// Panel
function _closePlaylistPanel() {
  const panel = $('playlist-panel')
  const content = panel.querySelector('.panel-content')
  if (!content || panel.classList.contains('hidden')) return
  content.classList.add('panel-closing')
  content.addEventListener('animationend', () => {
    content.classList.remove('panel-closing')
    panel.classList.add('hidden')
  }, { once: true })
}
$('btn-playlist-panel').addEventListener('click', () => {
  const panel = $('playlist-panel')
  panel.classList.remove('hidden')
  renderPanel()
})
$('playlist-overlay').addEventListener('click', _closePlaylistPanel)
$('playlist-panel').querySelector('.panel-close').addEventListener('click', _closePlaylistPanel)
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
    let _folderResizeRAF = null
    window.addEventListener('resize', () => {
      _startResizeThrottle()
      clearTimeout(_resizeFlushTimer)
      _resizeFlushTimer = setTimeout(() => {
        _flushResizeThrottle()
        document.querySelectorAll('.vl-container').forEach(c => { if (c._vlRender) c._vlRender() })
        document.querySelectorAll('.virtual-vl').forEach(c => { if (c._vlRebuild) c._vlRebuild(); else if (c._vlRender) c._vlRender() })
      }, 300)
      // Reflow folder grid columns in real time (per animation frame) so the
      // column count follows the window width immediately while dragging.
      if (!_folderResizeRAF) {
        _folderResizeRAF = requestAnimationFrame(() => {
          _folderResizeRAF = null
          document.querySelectorAll('.virtual-vl').forEach(c => { if (c._vlRebuild) c._vlRebuild(); else if (c._vlRender) c._vlRender() })
        })
      }
      if (!_resizeRAF) {
        _resizeRAF = requestAnimationFrame(() => {
          if (S.bgData) apThBg()
          _resizeRAF = null
        })
      }
    })
    // When window becomes visible again, force-update all UI components
    // so user doesn't see stale state (progress bar, lyrics, etc.)
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) return
      // Update progress bar from current audio state
      if (audio.duration) {
        S.cTime = audio.currentTime
        const p = S.dur ? (S.cTime / S.dur) * 100 : 0
        _progressFill.style.width = p + '%'
        _progressHandle.style.left = p + '%'
        _progressCurrent.textContent = fmtTime(S.cTime)
      }
      // Re-render lyrics if visible
      if (S.view === 'lyrics' && activeLrcTab === 'lyrics' && lrc.length) {
        const scroll = $('lyrics-lines-scroll')
        if (scroll) {
          const lines = scroll.querySelectorAll('.lc-line')
          let activeIdx = -1
          for (let i = lrc.length - 1; i >= 0; i--) {
            if (S.cTime >= lrc[i].time) { activeIdx = i; break }
          }
          // Update active class
          lines.forEach((l, i) => l.classList.toggle('active', i === activeIdx))
          _lastLrcActiveIdx = activeIdx
          // Scroll to center
          if (activeIdx >= 0) {
            const activeLine = scroll.querySelector(`.lc-line[data-lidx="${activeIdx}"]`)
            if (activeLine) _scrollLyricToCenter(activeLine, 'auto')
          }
        }
      }
      // Sync playing state visuals
      syncPlayingState()
    })
    if (S.all.length) pl = S.all
    syncPlayingState()
    if (S.tI >= 0 && S.tI < pl.length) {
      const t = pl[S.tI]
      if (t) {
        updPUI(t)
        try {
          audio.src = 'file:///' + t.path.replace(/\\/g, '/')
          audio.currentTime = S.cTime || 0
          if (S.devId && audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
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

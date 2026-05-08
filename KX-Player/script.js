import { api } from './api.js'

const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

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
  folderTree: [], folderStack: [], _syncingView: false
}

let fp = [], audio = new Audio(), lrc = [], pl = [], nI = 0

function $(sel) { return document.querySelector(/^[#.]/.test(sel) ? sel : '#' + sel) }
function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;') }
function fmtTime(t) { if (!t || !isFinite(t)) return '00:00'; const m = Math.floor(t / 60), s = Math.floor(t % 60); return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0') }
function isVideoFile(t) { return t && t.isVideo === true }

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

// === Virtual List ===
function virtualList(containerId, items, rowHeight, renderItem, onClick) {
  const c = $(containerId)
  if (!c) return
  if (c._vlRO) { c._vlRO.disconnect(); c._vlRO = null }
  c.innerHTML = ''
  if (!items.length) { c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u6682\u65e0\u5185\u5bb9</h3></div>'; return }
  const totalH = items.length * rowHeight
  const spacer = document.createElement('div'); spacer.style.height = totalH + 'px'; spacer.style.position = 'relative'
  const view = document.createElement('div'); view.style.position = 'absolute'; view.style.top = '0'; view.style.left = '0'; view.style.right = '0'
  spacer.appendChild(view); c.appendChild(spacer)
  const buffer = 10
  function render() {
    const scrollTop = c.scrollTop, clientH = c.clientHeight || 600
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - buffer)
    const end = Math.min(items.length, Math.ceil((scrollTop + clientH) / rowHeight) + buffer)
    view.style.top = (start * rowHeight) + 'px'
    let html = ''
    for (let i = start; i < end; i++) html += renderItem(items[i], i)
    view.innerHTML = html
  }
  c._vlRender = render; c._vlItems = items; render()
  c.addEventListener('scroll', render, { passive: true })
  let resizeTimer
  const ro = new ResizeObserver(() => { clearTimeout(resizeTimer); resizeTimer = setTimeout(render, 100) })
  ro.observe(c); c._vlRO = ro
  if (onClick) {
    c.addEventListener('click', e => {
      const playBtn = e.target.closest('.idx-play-btn')
      if (playBtn) { const row = e.target.closest('.song-row'); if (row && row.dataset.tid) onClick(row.dataset.tid) }
    })
    c.addEventListener('dblclick', e => {
      const row = e.target.closest('.song-row')
      if (row && row.dataset.tid) onClick(row.dataset.tid, true)
    })
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
    const cleanup = () => { $('confirm-modal').classList.add('hidden') }
    const okHandler = () => { cleanup(); $('confirm-ok-btn').removeEventListener('click', okHandler); $('confirm-cancel-btn').removeEventListener('click', cancelHandler); resolve(true) }
    const cancelHandler = () => { cleanup(); $('confirm-ok-btn').removeEventListener('click', okHandler); $('confirm-cancel-btn').removeEventListener('click', cancelHandler); resolve(false) }
    $('confirm-ok-btn').addEventListener('click', okHandler)
    $('confirm-cancel-btn').addEventListener('click', cancelHandler)
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

// === Settings ===
let saveTimer = null
function schedSave() { clearTimeout(saveTimer); saveTimer = setTimeout(saveS, 500) }
async function saveS() {
  try {
    const data = JSON.parse(JSON.stringify({
      folderPaths: fp, favs: S.favs, recents: S.recents, view: S.view, q: S.q, theme: S.theme, clr: S.clr, ovl: S.ovl, devId: S.devId,
      bgData: S.bgData, bgPath: S.bgPath, bgSize: S.bgSize, aI: S.aI, alI: S.alI, tI: S.tI, playing: S.playing, cTime: S.cTime,
      dur: S.dur, vol: S.vol, muted: S.muted, mode: S.mode, pls: S.pls, aPl: S.aPl, aF: S.aF,
      bgBlur: S.bgBlur, selMode: S.selMode, folderStack: S.folderStack, _imgEditState: S._imgEditState, listTextColor: S.listTextColor,
      titlebarOpacity: S.titlebarOpacity, playerOpacity: S.playerOpacity,
      playingTid: S.playingTid
    }))
    await api.saveSettings(data)
    if (S.bgData && S.bgData.length < 100000000) {
      await idbSet('cache', 'bgImage', S.bgData)
    }
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
    if ('bgData' in s) S.bgData = s.bgData; if ('bgPath' in s) S.bgPath = s.bgPath; if (s.bgSize) S.bgSize = s.bgSize
    if (s._imgEditState) S._imgEditState = s._imgEditState
    if (typeof s.aI === 'number') S.aI = s.aI; if (typeof s.alI === 'number') S.alI = s.alI; if (typeof s.tI === 'number') S.tI = s.tI
    if (typeof s.vol === 'number') S.vol = s.vol; if (typeof s.muted === 'boolean') S.muted = s.muted
    if (typeof s.bgBlur === 'number') S.bgBlur = s.bgBlur
    if (typeof s.sidebarOpacity === 'number') S.sidebarOpacity = s.sidebarOpacity; else S.sidebarOpacity = 100
    if (typeof s.titlebarOpacity === 'number') S.titlebarOpacity = s.titlebarOpacity; else S.titlebarOpacity = 100
    if (typeof s.playerOpacity === 'number') S.playerOpacity = s.playerOpacity; else S.playerOpacity = 100
    if (s.selMode) S.selMode = s.selMode; if (s.listTextColor) S.listTextColor = s.listTextColor
    if (s.playingTid) S.playingTid = s.playingTid
    if (Array.isArray(s.favs)) S.favs = s.favs; if (Array.isArray(s.pls)) S.pls = s.pls
    if (s.aPl) S.aPl = s.aPl; if (s.aF) S.aF = s.aF
    if (Array.isArray(s.folderStack)) S.folderStack = s.folderStack

    // Try to restore bg from IDB cache only if bgData was never set in settings
    if (S.bgData === undefined) {
      try { const cached = await idbGet('cache', 'bgImage'); if (cached) S.bgData = cached } catch (e) { /* ignore */ }
    }

    if (Array.isArray(s.folderPaths) && s.folderPaths.length > 0) {
      fp = s.folderPaths.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
      const result = await api.scanFoldersWithProgress(fp)
      const rs = result.artists || result; S.folderTree = result.folderTree || []
      const at = []
      for (const a of rs) for (const al of a.albums) { for (const t of al.tracks) { t.albumCoverData = al.coverData; at.push(t) } }
      S.af = rs; S.all = at
      const allIds = new Set(at.map(t => t.id))
      cleanupStale(allIds)
      if (fp.length > 0) try { await api.startWatching(fp, () => { rescan() }) } catch (e) { /* ignore */ }
    }
  } catch (e) { /* ignore */ }
}

// === Theme ===
function apTh() {
  const root = document.documentElement, isDark = S.theme === 'dark'
  root.style.setProperty('--accent', S.clr)
  const [r, g, b] = hex2rgb(S.clr)
  root.style.setProperty('--accent-light', `rgb(${Math.min(255, r + 30)},${Math.min(255, g + 10)},${Math.min(255, b + 20)})`)
  root.style.setProperty('--accent-bg', `rgba(${r},${g},${b},0.35)`)
  root.style.setProperty('--accent-r', r); root.style.setProperty('--accent-g', g); root.style.setProperty('--accent-b', b)
  if (isDark) {
    root.style.setProperty('--bg', '#0d0d12'); root.style.setProperty('--bg-card', 'rgba(22,22,30,0.92)')
    root.style.setProperty('--bg-sidebar', 'rgba(16,16,22,0.84)'); root.style.setProperty('--bg-player', 'rgba(20,20,28,0.35)')
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
    root.style.setProperty('--bg-sidebar', 'rgba(255,255,255,0.84)'); root.style.setProperty('--bg-player', 'rgba(255,255,255,0.05)')
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
  const sidebarAlpha = (S.sidebarOpacity ?? 100) / 100
  const sbBg = isDark ? `rgba(16,16,22,${sidebarAlpha})` : `rgba(245,245,247,${sidebarAlpha})`
  $('sidebar').style.background = sbBg
  $('sidebar').style.borderRight = `1.5px solid ${isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)'}`
  // Titlebar opacity
  const titlebarAlpha = (S.titlebarOpacity ?? 100) / 100
  const tbBg = isDark ? `rgba(16,16,22,${titlebarAlpha})` : `rgba(255,255,255,${titlebarAlpha})`
  $('titlebar').style.background = tbBg
  // Player bar opacity
  const playerAlpha = (S.playerOpacity ?? 100) / 100
  const pbBg = isDark ? `rgba(20,20,28,${playerAlpha})` : `rgba(255,255,255,${playerAlpha})`
  $('player-bar').style.background = pbBg
}

function apThBg() {
  if (!S.bgData) { $('bg-img').src = ''; $('bg-layer').style.opacity = 1; $('bg-layer').style.filter = ''; return }
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
  $('theme-dark').classList.toggle('active', S.theme === 'dark'); $('theme-light').classList.toggle('active', S.theme === 'light')
  $('sidebar-opacity').value = S.sidebarOpacity ?? 100; $('sidebar-opacity-val').textContent = (S.sidebarOpacity ?? 100) + '%'
  $('titlebar-opacity').value = S.titlebarOpacity ?? 100; $('titlebar-opacity-val').textContent = (S.titlebarOpacity ?? 100) + '%'
  $('player-opacity').value = S.playerOpacity ?? 100; $('player-opacity-val').textContent = (S.playerOpacity ?? 100) + '%'
  if (S.bgData) { $('bg-preview').style.backgroundImage = `url(${S.bgData})`; $('bg-preview-wrap').classList.remove('hidden'); $('btn-bg-upload').classList.add('hidden') } else { $('bg-preview-wrap').classList.add('hidden'); $('btn-bg-upload').classList.remove('hidden') }
  document.querySelectorAll('.cp-preset').forEach(b => b.classList.toggle('active', b.dataset.clr === S.clr))
}

// === Audio ===
async function loadT(idx) {
  if (idx < 0 || idx >= pl.length) return
  nI = idx
  const t = pl[idx]
  const ext = (t.format || '').toLowerCase()
  if (ext === 'dsf' || ext === 'dff' || ext === 'dsd') {
    try {
      const wavPath = await api.decodeDSD(t.path)
      if (!wavPath) throw new Error('\u89e3\u7801\u5931\u8d25')
      audio.src = 'file:///' + wavPath.replace(/\\/g, '/')
    } catch (e) { audio.src = 'file:///' + t.path.replace(/\\/g, '/') }
  } else {
    audio.src = 'file:///' + t.path.replace(/\\/g, '/')
  }
  await loadLrcForTrack(t)
  if (S.devId && audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
  await audio.play().catch(() => { /* ignore */ })
  S.tI = idx; S.playing = true
}

async function loadLrcForTrack(t) {
  lrc = []
  try {
    const sep = t.path.includes('\\') ? '\\' : '/'
    const lastSep = t.path.lastIndexOf(sep)
    const dir = lastSep >= 0 ? t.path.substring(0, lastSep) : ''
    const baseName = lastSep >= 0 ? t.path.substring(lastSep + 1) : t.path
    const lastDot = baseName.lastIndexOf('.')
    const nameWithoutExt = lastDot >= 0 ? baseName.substring(0, lastDot) : baseName
    const lrcPath = dir + sep + nameWithoutExt + '.lrc'
    const lrcExists = await api.fileExists(lrcPath)
    if (!lrcExists) return
    const lrcContent = await api.readTextFile(lrcPath)
    if (!lrcContent) return
    const lines = lrcContent.split(/\r?\n/)
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
    // Skip synchronous render with stale lrc — wait for loadT to load new lyrics
  } else {
    renderContent()
  }
  S.playing = true; updPlayBtn(); invalidateVL('vl-songs'); syncAllListsPlaying(); renderPanel(); schedSave()
}

function updPUI(t, skipLrc) {
  const cd = t.coverData || t.albumCoverData
  $('player-title').textContent = t.name
  const artist = t.metaArtist || t.artist || '\u4f5a\u540d'
  const isVid = isVideoFile(t)
  $('player-artist').textContent = artist + (isVid ? ' \u00b7 \u89c6\u9891-\u4ec5\u97f3\u9891\u6a21\u5f0f' : '')
  if (cd) {
    $('player-cover-img').src = cd; $('player-cover-img').style.display = ''
    $('player-cover').querySelector('.cover-placeholder').style.display = 'none'
  } else {
    $('player-cover-img').style.display = 'none'; $('player-cover').querySelector('.cover-placeholder').style.display = ''
    const ph = $('player-cover').querySelector('.cover-placeholder svg')
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
  S.cTime = audio.currentTime; S.dur = audio.duration
  if (isNaN(S.dur)) return
  if (S.mode === 2) { audio.currentTime = 0; audio.play().catch(() => { /* ignore */ }) }
  else if (S.mode === 3) { S.playing = false; updPlayBtn(); updPUI(pl[S.tI]); syncAllListsPlaying() }
  else nxt()
}

function nxt() {
  if (pl.length === 0) return
  if (S.mode === 1) { const r = Math.floor(Math.random() * pl.length); playT(r, true); return }
  playT((S.tI + 1) % pl.length, true)
}
function prv() { if (pl.length === 0) return; playT(S.tI <= 0 ? pl.length - 1 : S.tI - 1, true) }

// === Player UI ===
function updPlayBtn() { $('icon-play').style.display = S.playing ? 'none' : ''; $('icon-pause').style.display = S.playing ? '' : 'none' }

// === Render ===
function renderAll() {
  renderSB(); renderContent()
  renderPanel(); updPlayBtn(); syncAllListsPlaying(); schedSave()
}

function renderSB() {
  $('folder-list').innerHTML = ''
  if (fp.length && S.folderTree.length) {
    // Only show folders that exist in the scanned tree
    const treeRootPaths = new Set(S.folderTree.map(n => n.path.replace(/\\/g, '/')))
    for (const p of fp) {
      const np = p.replace(/\\/g, '/')
      if (!treeRootPaths.has(np)) continue
      const top = p.replace(/\\/g, '/').replace(/\/$/, '').split('/').pop() || p
      $('folder-list').insertAdjacentHTML('beforeend',
        `<button class="folder-item" data-fp="${esc(p)}"><svg class="folder-icon" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg><span class="folder-name">${esc(top)}</span></button>`)
    }
  }
  $('fav-list').innerHTML = S.favs.map(f => `<button class="fav-sidebar-item${S.aF === f.id ? ' active' : ''}" data-fvid="${f.id}" title="${f.isDefault ? '\u9ed8\u8ba4\u6536\u85cf\u5939' : '\u53cc\u51fb\u91cd\u547d\u540d'}"><span class="fav-sidebar-name">${esc(f.name)}</span><span class="fav-sidebar-count">${f.trackIds.length}</span></button>`).join('')
  $('playlist-list').innerHTML = S.pls.map(p => `<button class="playlist-sidebar-item${S.aPl === p.id ? ' active' : ''}" data-plid="${p.id}" title="\u53cc\u51fb\u91cd\u547d\u540d"><span class="pl-sidebar-name">${esc(p.name)}</span><span class="pl-sidebar-count">${p.trackIds.length}</span></button>`).join('')
  const favSection = $('fav-list').closest('.nav-section')
  if (favSection) favSection.classList.toggle('empty', S.favs.length === 0)
  const plSection = $('playlist-list').closest('.nav-section')
  if (plSection) plSection.classList.toggle('empty', S.pls.length === 0)
  const navs = document.querySelectorAll('#sidebar-nav .nav-item')
  navs.forEach(n => { n.classList.remove('active'); if (n.dataset.view === S.view) n.classList.add('active') })
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

function renderFolderAll() {
  const bc = $('breadcrumb'), ca = $('content-area')
  if (S.q) {
    bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-search-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span><button class="breadcrumb-item current">搜索结果</button>`
    if (!pl.length) { ca.innerHTML = emptyS('未找到匹配的音乐', '请尝试其他搜索关键词', false); return }
    ca.innerHTML = `<div class="section-title">搜索结果<span>${pl.length} 首</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="search"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>播放全部</button>${tableH(pl)}`
    return
  }
  const tree = S.folderTree || []
  if (!tree.length || !tree.some(n => hasMusicRecursive(n))) {
    ca.innerHTML = emptyS('\u8fd8\u6ca1\u6709\u97f3\u4e50\u6587\u4ef6\u5939', '\u70b9\u51fb\u5bfc\u5165\u6587\u4ef6\u5939\u5f00\u59cb', true)
    bc.innerHTML = `<button class="breadcrumb-item current">\u5168\u90e8\u97f3\u4e50</button>`
    return
  }
  if (S.folderStack.length === 0) {
    bc.innerHTML = `<button class="breadcrumb-item current">\u5168\u90e8\u97f3\u4e50</button>`
    const validRoots = tree.filter(n => hasMusicRecursive(n))
    const html = validRoots.map(n => folderCardHTML(n)).join('')
    ca.innerHTML = `<div class="section-title">\u6587\u4ef6\u5939<span>${validRoots.length} \u4e2a\u6587\u4ef6\u5939</span></div><div class="artist-grid">${html}</div>`
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
  const coverBg = findCoverInNode(n)
  const subtitle = n.trackCount ? `${n.trackCount} \u9996\u97f3\u4e50${n.children.length ? ` \u00b7 ${n.children.filter(c => hasMusicRecursive(c)).length} \u5b50\u6587\u4ef6\u5939` : ''}` : `${n.children.filter(c => hasMusicRecursive(c)).length} \u4e2a\u5b50\u6587\u4ef6\u5939`
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
    return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}">${S.selMode ? `<div class="song-row-check"><input type="checkbox" data-tid="${t.id}" /></div>` : ''}<div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(t.name)}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${S.favs.some(f => f.isDefault && f.trackIds.includes(t.id)) ? ' liked' : ''}" data-tid="${t.id}">${S.favs.some(f => f.isDefault && f.trackIds.includes(t.id)) ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
  }).join('')
  return `<div class="song-table">${checks}${items}</div>`
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
    const r = findNodeByPath(n.children, path)
    if (r) return r
  }
  return null
}

function renderFolderNode(node) {
  const bc = $('breadcrumb')
  bc.innerHTML = ''
  if (S.folderStack.length > 0) {
    bc.innerHTML = `<button class="btn-breadcrumb-back" id="btn-folder-back"><svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" style="vertical-align:middle;margin-right:3px"><polyline points="15,18 9,12 15,6"/></svg>返回</button><span class="breadcrumb-sep">|</span>`
  }
  bc.innerHTML += `<button class="breadcrumb-item" data-fp-root="">\u5168\u90e8\u97f3\u4e50</button>`
  for (let i = 0; i < S.folderStack.length; i++) {
    const fp_i = S.folderStack[i]; const fn = findNodeByPath(S.folderTree, fp_i); const nm = fn ? fn.name : fp_i.split(/[\\/]/).pop()
    bc.innerHTML += `<span class="breadcrumb-sep">/</span><button class="breadcrumb-item${i === S.folderStack.length - 1 ? ' current' : ''}" data-fp="${esc(fp_i)}">${esc(nm)}</button>`
  }
  const validChildren = node.children.filter(c => hasMusicRecursive(c))
  let html = ''

  // Subfolder cards only (click to navigate) \u2014 use shared folderCardHTML for consistency
  if (validChildren.length > 0) {
    html += `<div class="section-title">\u5b50\u6587\u4ef6\u5939<span>${validChildren.length} \u4e2a</span></div><div class="artist-grid">${validChildren.map(c => folderCardHTML(c)).join('')}</div>`
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
      return `<div class="song-row${isPlaying ? ' playing' : ''} ${playState}" data-tid="${t.id}" style="grid-template-columns:${cols}"><div class="song-row-idx"><span class="idx-num">${i + 1}</span><span class="idx-play-btn"><svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="5,3 19,12 5,21"/></svg></span><span class="idx-wave"><span class="wave-bar"></span><span class="wave-bar"></span><span class="wave-bar"></span></span></div><div class="song-row-title">${esc(t.name)}</div><div class="song-row-artist">${esc(t.metaArtist || t.artist)}</div><div class="song-row-album">${esc(t.album || '')}</div><div class="song-row-like${S.favs.some(f => f.isDefault && f.trackIds.includes(t.id)) ? ' liked' : ''}" data-tid="${t.id}">${S.favs.some(f => f.isDefault && f.trackIds.includes(t.id)) ? '\u2665' : '\u2661'}</div><div class="song-row-duration">${fmtTime(t.duration)}</div><div class="song-row-format"><span>${isVid ? '\uD83C\uDFAC' : ''}${(t.format || '').toUpperCase()}</span></div></div>`
    }, (tid, keepView) => { const idx = pl.findIndex(t => t.id === tid); if (idx >= 0) playT(idx, keepView) })
  }
}

function navigateFolder(path) {
  const node = findNodeByPath(S.folderTree, path)
  if (!node) return
  S.folderStack.push(node.path)
  S.view = 'all'; renderAll(); schedSave()
}

function navigateFolderUp() {
  if (S.folderStack.length <= 1) { S.folderStack = [] }
  else { S.folderStack.pop() }
  S.view = 'all'; renderAll(); schedSave()
}

function navigateFolderTo(path) {
  if (!path) { S.folderStack = [] }
  else {
    const idx = S.folderStack.indexOf(path)
    if (idx >= 0) { S.folderStack = S.folderStack.slice(0, idx + 1) }
    else { S.folderStack.push(path) }
  }
  S.view = 'all'; renderAll(); schedSave()
}

// === Recent View ===
function renderRecentView() {
  $('breadcrumb').innerHTML = `<button class="breadcrumb-item current">\u6700\u8fd1\u64ad\u653e</button>`
  const tks = S.recents.map(id => S.all.find(t => t.id === id)).filter(Boolean)
  if (!tks.length) { $('content-area').innerHTML = emptyS('\u8fd8\u6ca1\u6709\u64ad\u653e\u8bb0\u5f55', '\u5f00\u59cb\u64ad\u653e\u97f3\u4e50\u540e\u4f1a\u81ea\u52a8\u8bb0\u5f55', false); return }
  pl = tks
  $('content-area').innerHTML = `<div class="section-title">\u6700\u8fd1\u64ad\u653e<span>${tks.length} \u9996</span></div><button class="btn-primary" style="margin-bottom:12px" data-pall="recent"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button>${tableH(tks)}`
}

// === Lyrics ===
let activeLrcTab = 'lyrics'
function renderLrcContent() {
  const t = S.playingTid ? S.all.find(x => x.id === S.playingTid) : null
  const container = $('content-area')
  if (!container) return
  const cd = t ? (t.coverData || t.albumCoverData) : null
  const lrcHtml = buildLrcLines(lrc)
  container.innerHTML = t ? `<div class="lyrics-content-layout"><div class="lyrics-content-left"><div class="lyrics-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="56" height="56" fill="none" stroke="currentColor" stroke-width="1"><circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/></svg></div>'}</div></div><div class="lyrics-content-right"><div class="lyrics-content-info"><div class="lc-title">${esc(t.name)}</div><div class="lc-artist">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}${isVideoFile(t) ? ' \u00b7 \u89c6\u9891-\u4ec5\u97f3\u9891\u6a21\u5f0f' : ''}</div></div><div class="lyrics-tab-btns"><button class="lyrics-tab-btn${activeLrcTab === 'lyrics' ? ' active' : ''}" data-ltab="lyrics">\u6b4c\u8bcd</button><button class="lyrics-tab-btn${activeLrcTab === 'meta' ? ' active' : ''}" data-ltab="meta">\u4fe1\u606f</button></div><div class="lyrics-container-wrapper"><div class="lyrics-lines-scroll${activeLrcTab !== 'lyrics' ? ' hidden' : ''}" id="lyrics-lines-scroll">${lrcHtml || '<div class="lc-empty">\u6682\u65e0\u6b4c\u8bcd</div>'}</div><div class="lyrics-meta-panel${activeLrcTab !== 'meta' ? ' hidden' : ''}" id="lyrics-meta-panel"><div class="meta-row"><span class="meta-label">\u6807\u9898</span><span class="meta-value">${esc(t.name)}</span></div><div class="meta-row"><span class="meta-label">\u827a\u672f\u5bb6</span><span class="meta-value">${esc(t.metaArtist || t.artist || '\u4f5a\u540d')}</span></div><div class="meta-row"><span class="meta-label">\u4e13\u8f91</span><span class="meta-value">${esc(t.album || '')}</span></div><div class="meta-row"><span class="meta-label">\u683c\u5f0f</span><span class="meta-value">${t.format.toUpperCase()}${isVideoFile(t) ? ' (\u89c6\u9891)' : ''}</span></div><div class="meta-row"><span class="meta-label">\u65f6\u957f</span><span class="meta-value">${fmtTime(t.duration)}</span></div><div class="meta-row"><span class="meta-label">\u6587\u4ef6</span><span class="meta-value">${esc(t.path)}</span></div></div></div></div></div>` : '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u672a\u5728\u64ad\u653e</h3></div>'
  const lines = container.querySelectorAll('.lc-line')
  const scroll = $('lyrics-lines-scroll')
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

// === Favorites/Playlists ===
function mkP(n) { n = n || '\u65b0\u5217\u8868'; const id = 'pl-' + Date.now(); S.pls.push({ id, name: n, trackIds: [], coverData: null }); schedSave(); renderAll(); requestAnimationFrame(() => startRename('pl', id)) }
async function rmP(id) { const p = S.pls.find(x => x.id === id); if (!p) return; const ok = await showConfirm('删除播放列表', `确定删除播放列表"${p.name}"吗？`); if (!ok) return; S.pls = S.pls.filter(x => x.id !== id); if (S.aPl === id) { S.aPl = null; S.view = 'all' } schedSave(); renderAll() }
function rnP(id) { startRename('pl', id) }
function a2P(pid, tid) { const p = S.pls.find(x => x.id === pid); if (!p || p.trackIds.includes(tid)) return; p.trackIds.push(tid); if (!p.coverData) { const t = S.all.find(x => x.id === tid); if (t) p.coverData = t.coverData || t.albumCoverData } schedSave(); renderAll() }
function rFP(pid, tid) { const p = S.pls.find(x => x.id === pid); if (!p) return; p.trackIds = p.trackIds.filter(id => id !== tid); if (!p.trackIds.length) p.coverData = null; if (S.playingTid === tid) { S.playingTid = null; S.view = 'all'; S.aF = null; S.aPl = null; audio.pause(); S.playing = false; lrc = [] } schedSave(); renderAll() }
function mkF(n) { n = n || '\u65b0\u6536\u85cf\u5939'; const id = 'fav-' + Date.now(); S.favs.push({ id, name: n, trackIds: [], isDefault: false }); schedSave(); renderAll(); requestAnimationFrame(() => startRename('fav', id)) }
async function rmF(id) { const f = S.favs.find(x => x.id === id); if (!f) return; const ok = await showConfirm('删除收藏夹', `确定删除收藏夹"${f.name}"吗？`); if (!ok) return; S.favs = S.favs.filter(x => x.id !== id); if (S.aF === id) { S.aF = null; S.view = 'all' } schedSave(); renderAll() }
function rnF(id) { startRename('fav', id) }
function a2F(fid, tid) { const f = S.favs.find(x => x.id === fid); if (!f || f.trackIds.includes(tid)) return; f.trackIds.push(tid); schedSave(); renderAll() }
function rFF(fid, tid) { const f = S.favs.find(x => x.id === fid); if (!f) return; f.trackIds = f.trackIds.filter(id => id !== tid); if (S.playingTid === tid) { S.playingTid = null; S.view = 'all'; S.aF = null; S.aPl = null; audio.pause(); S.playing = false; lrc = [] } schedSave(); renderAll() }

function startRename(type, id) {
  const isFav = type === 'fav', list = isFav ? S.favs : S.pls, item = list.find(x => x.id === id)
  if (!item || (isFav && item.isDefault)) return
  let el = null, isSidebar = false
  if (isFav) { el = document.querySelector(`.fav-sidebar-item[data-fvid="${id}"] .fav-sidebar-name`); if (el) isSidebar = true; if (!el) el = document.querySelector(`.pl-content-name[data-fvid="${id}"]`) }
  else { el = document.querySelector(`.playlist-sidebar-item[data-plid="${id}"] .pl-sidebar-name`); if (el) isSidebar = true; if (!el) el = document.querySelector(`.pl-content-name[data-plid="${id}"]`) }
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
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">\u64ad\u653e\u5217\u8868</div><div class="pl-content-name" data-plid="${plObj.id}" title="\u53cc\u51fb\u91cd\u547d\u540d">${esc(plObj.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-ppl="${plObj.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button><button class="btn-danger" data-delpl="${plObj.id}">\u5220\u9664</button></div></div></div>${tableH(tks)}`
}
function renderFContent(fav, tks) {
  const td = tks.find(t => t.coverData || t.albumCoverData), cd = td ? td.coverData || td.albumCoverData : null
  return `<div class="pl-content-header"><div class="pl-content-cover">${cd ? `<img src="${cd}" alt="" />` : '<div class="cover-fallback"><svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg></div>'}</div><div class="pl-content-info"><div class="pl-content-label">\u6536\u85cf\u5939</div><div class="pl-content-name" data-fvid="${fav.id}" title="${fav.isDefault ? '\u9ed8\u8ba4\u6536\u85cf\u5939' : '\u53cc\u51fb\u91cd\u547d\u540d'}">${esc(fav.name)}</div><div class="pl-content-actions"><button class="btn-primary" data-pfav="${fav.id}"><svg viewBox="0 0 24 24" width="13" height="13" fill="white"><polygon points="5,3 19,12 5,21"/></svg>\u64ad\u653e\u5168\u90e8</button>${!fav.isDefault ? `<button class="btn-danger" data-delfv="${fav.id}">\u5220\u9664</button>` : ''}</div></div></div>${tableH(tks)}`
}

// === Scan ===
async function importFolder() {
  try {
    const result = await api.openFolder(); if (!result || !result.length) return
    const normalized = result.map(p => p.replace(/\\/g, '/').replace(/\/+$/, ''))
    for (const p of normalized) { if (!fp.includes(p)) fp.push(p) }
    const tid = addT('\u6b63\u5728\u626b\u63cf\u97f3\u4e50\u6587\u4ef6...')
    api.onScannerProgress((data) => { updT(tid, `${data.stage || '\u89e3\u6790\u4e2d...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
    const r = await api.scanFoldersWithProgress(fp)
    const rs = r.artists || r; S.folderTree = r.folderTree || []
    const at = []
    for (const a of rs) for (const al of a.albums) { for (const t of al.tracks) { t.albumCoverData = al.coverData; at.push(t) } }
    S.af = rs; S.all = at
    const allIds = new Set(at.map(t => t.id))
    cleanupStale(allIds)
    S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []
    pl = at
    if (r.fileCount > 0) { updT(tid, '\u5b8c\u6210\u2714', 100, `\u5171 ${r.fileCount || at.length} \u9996\u97f3\u4e50`); rmT(tid) } else { updT(tid, '\u672a\u627e\u5230\u97f3\u4e50', 0, '\u8bf7\u68c0\u67e5\u6587\u4ef6\u5939\u5185\u5bb9'); setTimeout(() => rmT(tid), 5000) }
    if (fp.length > 0) try { await api.startWatching(fp, () => { rescan() }) } catch (e) { /* ignore */ }
    renderAll()
  } catch (e) { alert('\u5bfc\u5165\u5931\u8d25: ' + e.message) }
}

async function rescan() {
  if (!fp.length) {
    S.af = []; S.all = []; S.folderTree = []; S.folderStack = []
    pl = []; S.playingTid = null; S.tI = -1; audio.pause()
    cleanupStale(new Set())
    renderAll(); schedSave()
    return
  }
  const tid = addT('\u91cd\u65b0\u626b\u63cf...')
  api.onScannerProgress((data) => { updT(tid, `${data.stage || '\u89e3\u6790\u4e2d...'}`, Math.round((data.completed / data.total) * 100), `${data.completed}/${data.total}`) })
  try {
    const currentTrackId = pl.length > 0 && S.tI >= 0 ? pl[S.tI]?.id : null
    const r = await api.scanFoldersWithProgress(fp)
    const rs = r.artists || r; S.folderTree = r.folderTree || []
    const at = []
    for (const a of rs) for (const al of a.albums) { for (const t of al.tracks) { t.albumCoverData = al.coverData; at.push(t) } }
    S.af = rs; S.all = at
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
    updT(tid, '\u5b8c\u6210\u2714', 100, `\u5171 ${r.fileCount || at.length} \u9996`)
    rmT(tid)
    renderAll()
  } catch (e) { updT(tid, '\u5931\u8d25', 0, e.message) }
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
    // Each option is its own group, separated by hr
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

function showFavPicker(tid, baseX, baseY, baseH) {
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

function showPlPicker(tid, baseX, baseY, baseH) {
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

function hC() { $('ctx-menu').classList.add('hidden'); $('ctx-playlist-sub').classList.add('hidden') }

// === Tools ===
const toolsState = { extractFiles: [], convertFiles: [], convertFmt: 'mp3', extractFmt: 'mp3' }

function renderToolsContent() {
  $('breadcrumb').innerHTML = `<button class="breadcrumb-item current">\u5de5\u5177</button>`
  $('content-area').innerHTML = `
    <div class="tools-container">
      <div class="tools-section" id="tools-extract">
        <h3>\uD83C\uDFAC \u89c6\u9891\u63d0\u53d6\u97f3\u9891</h3>
        <p>\u4ece\u89c6\u9891\u6587\u4ef6\u4e2d\u63d0\u53d6\u97f3\u9891\u8f68\uff0c\u652f\u6301 MP4/MKV/AVI/MOV \u7b49\u683c\u5f0f\u3002</p>
        <div class="tools-dropzone" id="tools-extract-dropzone">
          <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
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
          <button class="btn-primary" id="btn-extract-start">\u89c6\u9891\u8f6c\u97f3\u9891\u542f\u52a8</button>
          <button class="btn-secondary" id="btn-extract-clear">\u6e05\u7a7a\u5217\u8868</button>
        </div>
      </div>
      <div class="tools-section" id="tools-convert">
        <h3>\uD83C\uDFB5 \u97f3\u9891\u683c\u5f0f\u8f6c\u6362</h3>
        <p>\u5c06\u97f3\u9891\u6587\u4ef6\u6279\u91cf\u8f6c\u6362\u4e3a\u5176\u4ed6\u683c\u5f0f\u3002</p>
        <div class="tools-dropzone" id="tools-convert-dropzone">
          <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7,10 12,15 17,10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
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
          <button class="btn-primary" id="btn-convert-start">\u97f3\u9891\u683c\u5f0f\u8f6c\u6362\u542f\u52a8</button>
          <button class="btn-secondary" id="btn-convert-clear">\u6e05\u7a7a\u5217\u8868</button>
        </div>
      </div>
    </div>`
  setupToolsEvents()
}

function renderToolFileList(id, files, section) {
  const el = $(id); if (!el) return
  el.innerHTML = files.map((f, i) => {
    const fn = typeof f === 'string' ? (f.split(/[\\/]/).pop()) : (f.name || f)
    const status = f._status || ''
    const css = f._pct !== undefined ? `background:linear-gradient(to right,var(--accent) 0%,var(--accent) ${f._pct}%,transparent ${f._pct}%)` : ''
    return `<div class="tools-file-item${status ? ' ' + status : ''}" style="${css}" data-tfi="${i}" data-tf-section="${section}"><span class="file-name">${esc(fn)}</span><span class="file-status">${status === 'done' ? '\u2714' : status === 'error' ? '\u2718' : status === 'running' ? '\u8f6c\u6362\u4e2d...' : ''}</span>${(!status || status === 'error') ? `<button class="file-remove" data-idx="${i}">&times;</button>` : ''}</div>`
  }).join('')
}

function setupToolsEvents() {
  const extractDz = $('tools-extract-dropzone'), convertDz = $('tools-convert-dropzone')
  toolsState.extractFiles = []; toolsState.convertFiles = []

  function makeDropzone(dz, kind) {
    dz.addEventListener('click', async () => {
      const files = await api.openAudioFiles()
      if (!files || !files.length) return
      const arr = kind === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
      for (const f of files) { if (!arr.find(x => (typeof x === 'string' ? x : x.path) === f)) arr.push(f) }
      const id = kind === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
      const actionsId = kind === 'extract' ? 'extract-actions' : 'convert-actions'
      renderToolFileList(id, arr, kind)
      if (arr.length > 0) $(actionsId).classList.remove('hidden')
    })
    dz.addEventListener('dragover', e => { e.preventDefault(); dz.classList.add('drag-over') })
    dz.addEventListener('dragleave', () => { dz.classList.remove('drag-over') })
    dz.addEventListener('drop', async e => {
      e.preventDefault(); dz.classList.remove('drag-over')
      if (!e.dataTransfer.files) return
      const arr = kind === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
      for (const f of e.dataTransfer.files) {
        const fp = f.path || f.name; if (!arr.find(x => (typeof x === 'string' ? x : x.path) === fp)) arr.push(fp)
      }
      const id = kind === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
      const actionsId = kind === 'extract' ? 'extract-actions' : 'convert-actions'
      renderToolFileList(id, arr, kind)
      if (arr.length > 0) $(actionsId).classList.remove('hidden')
    })
  }

  makeDropzone(extractDz, 'extract')
  makeDropzone(convertDz, 'convert')

  // Extract format selector
  $('tools-extract-fmts').addEventListener('click', e => {
    const b = e.target.closest('.tools-format-opt'); if (!b) return
    document.querySelectorAll('#tools-extract-fmts .tools-format-opt').forEach(x => x.classList.remove('active'))
    b.classList.add('active')
    toolsState.extractFmt = b.dataset.fmt
  })

  // Convert format selector
  $('tools-convert-fmts').addEventListener('click', e => {
    const b = e.target.closest('.tools-format-opt'); if (!b) return
    document.querySelectorAll('#tools-convert-fmts .tools-format-opt').forEach(x => x.classList.remove('active'))
    b.classList.add('active')
    toolsState.convertFmt = b.dataset.fmt
  })

  // File list delegation for remove
  document.querySelectorAll('.tools-file-list').forEach(el => {
    el.addEventListener('click', e => {
      const btn = e.target.closest('.file-remove'); if (!btn) return
      const section = e.target.closest('[data-tf-section]')?.dataset.tfSection
      const arr = section === 'extract' ? toolsState.extractFiles : toolsState.convertFiles
      arr.splice(parseInt(btn.dataset.idx), 1)
      const id = section === 'extract' ? 'tools-extract-files' : 'tools-convert-files'
      const actionsId = section === 'extract' ? 'extract-actions' : 'convert-actions'
      renderToolFileList(id, arr, section)
      if (arr.length === 0) $(actionsId).classList.add('hidden')
    })
  })

  // Extract start button
  $('btn-extract-start').addEventListener('click', () => {
    const arr = toolsState.extractFiles
    if (!arr.length) return
    startExtractBatch([...arr])
  })

  // Convert start button
  $('btn-convert-start').addEventListener('click', () => {
    const arr = toolsState.convertFiles
    if (!arr.length) return
    startConvertBatch([...arr])
  })

  // Clear buttons
  $('btn-extract-clear').addEventListener('click', () => {
    toolsState.extractFiles = []
    renderToolFileList('tools-extract-files', [], 'extract')
    $('extract-actions').classList.add('hidden')
    $('extract-overall').classList.add('hidden')
  })
  $('btn-convert-clear').addEventListener('click', () => {
    toolsState.convertFiles = []
    renderToolFileList('tools-convert-files', [], 'convert')
    $('convert-actions').classList.add('hidden')
    $('convert-overall').classList.add('hidden')
  })
}

async function startExtractBatch(files) {
  const fmt = toolsState.extractFmt || 'mp3'
  const overall = $('extract-overall'); overall.classList.remove('hidden')
  const overallText = $('extract-overall-text'); const overallFill = $('extract-overall-fill')

  for (let i = 0; i < files.length; i++) {
    const f = files[i]
    f._status = 'running'; f._pct = 10
    renderToolFileList('tools-extract-files', files, 'extract')
    overallText.textContent = `\u603b\u8fdb\u5ea6: \u7b2c${i + 1}/\u5171${files.length} \u4e2a`
    overallFill.style.width = ((i / files.length) * 100) + '%'

    try {
      await api.extractAudio(typeof f === 'string' ? f : f.path, fmt)
      f._status = 'done'; f._pct = undefined
    } catch (e) {
      f._status = 'error'; f._pct = undefined
    }
    renderToolFileList('tools-extract-files', files, 'extract')
    overallFill.style.width = (((i + 1) / files.length) * 100) + '%'
  }
  overallText.textContent = `\u5b8c\u6210: ${files.filter(f => f._status === 'done').length}/${files.length} \u6210\u529f`
}

async function startConvertBatch(files) {
  const fmt = toolsState.convertFmt || 'mp3'
  const overall = $('convert-overall'); overall.classList.remove('hidden')
  const overallText = $('convert-overall-text'); const overallFill = $('convert-overall-fill')

  for (let i = 0; i < files.length; i++) {
    const f = files[i]
    f._status = 'running'; f._pct = 10
    renderToolFileList('tools-convert-files', files, 'convert')
    overallText.textContent = `\u603b\u8fdb\u5ea6: \u7b2c${i + 1}/\u5171${files.length} \u4e2a`
    overallFill.style.width = ((i / files.length) * 100) + '%'

    try {
      await api.convertAudio(typeof f === 'string' ? f : f.path, fmt)
      f._status = 'done'; f._pct = undefined
    } catch (e) {
      f._status = 'error'; f._pct = undefined
    }
    renderToolFileList('tools-convert-files', files, 'convert')
    overallFill.style.width = (((i + 1) / files.length) * 100) + '%'
  }
  overallText.textContent = `\u5b8c\u6210: ${files.filter(f => f._status === 'done').length}/${files.length} \u6210\u529f`
}

// === Events ===
$('content-area').addEventListener('click', e => {
  // Check like button FIRST before song-row (it's a child of song-row)
  const like = e.target.closest('.song-row-like'); if (like) {
    e.stopPropagation(); const tid = like.dataset.tid
    showFavAndPlPicker(e, tid); return
  }
  const playBtn = e.target.closest('.idx-play-btn')
  if (playBtn) {
    const row = e.target.closest('.song-row'); if (row && row.dataset.tid) { const idx = pl.findIndex(tk => tk.id === row.dataset.tid); if (idx >= 0) playT(idx); return }
  }
  const t = e.target.closest('.song-row'); if (t && t.dataset.tid) { /* single click does nothing, use dblclick or play button */ return }
  if (e.target.closest('[data-pa]')) {
    const [ai, al_] = e.target.closest('[data-pa]').dataset.pa.split(':').map(Number)
    if (!isNaN(ai)) { const a = S.af[ai]; if (al_ !== undefined && !isNaN(al_)) { if (a) playAll(a.albums[al_]?.tracks || []); return } const tks = a ? [].concat(...a.albums.map(al => al.tracks)) : []; playAll(tks) }
    return
  }
  if (e.target.closest('[data-ppl]')) { const pid = e.target.closest('[data-ppl]').dataset.ppl; const p = S.pls.find(x => x.id === pid); if (p) { const tks = p.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean); playAll(tks) } return }
  if (e.target.closest('[data-pfav]')) { const fid = e.target.closest('[data-pfav]').dataset.pfav; const f = S.favs.find(x => x.id === fid); if (f) { const tks = f.trackIds.map(id => S.all.find(t => t.id === id)).filter(Boolean); playAll(tks) } return }
  if (e.target.closest('[data-pall]')) { playAll(S.all); return }
  if (e.target.closest('[data-pfolder]')) {
    const fpPath = e.target.closest('[data-pfolder]').dataset.pfolder; const n = findNodeByPath(S.folderTree, fpPath)
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
    if (time && time.time !== undefined) { audio.currentTime = time.time }
    return
  }
  if (e.target.closest('[data-ltab]')) {
    const tab = e.target.closest('[data-ltab]').dataset.ltab
    activeLrcTab = tab
    document.querySelectorAll('.lyrics-tab-btn').forEach(b => b.classList.remove('active'))
    e.target.closest('.lyrics-tab-btn').classList.add('active')
    $('lyrics-lines-scroll').classList.toggle('hidden', tab !== 'lyrics')
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
})

function showFolderCtx(e, folderPath) {
  const m = $('ctx-menu')
  const folderName = folderPath.split(/[\\/]/).pop()
  m.innerHTML = `<button data-a="frem">从扫描列表移除 "${esc(folderName)}"</button>`
  m.classList.remove('hidden')
  const mx = Math.min(e.clientX, window.innerWidth - 200), my = Math.min(e.clientY, window.innerHeight - 100)
  m.style.left = mx + 'px'; m.style.top = my + 'px'
  m.onclick = async function (ev) {
    const b = ev.target.closest('button'); if (!b) return
    if (b.dataset.a === 'frem') {
      hC()
      const ok = await showConfirm('移除文件夹', `确定从扫描列表移除文件夹"${folderName}"吗？`)
      if (!ok) return
      fp = fp.filter(p => p !== folderPath)
      S.folderTree = S.folderTree.filter(n => n.path !== folderPath && !n.path.startsWith(folderPath + '/'))
      await rescan()
      schedSave()
    }
    hC()
  }
  $('ctx-playlist-sub').classList.add('hidden')
}

$('#breadcrumb').addEventListener('click', e => {
  const b = e.target.closest('[data-bc="all"]'); if (b) { S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; S.folderStack = []; S.prevView = null; activeLrcTab = 'lyrics'; renderAll(); schedSave(); return }
  const a = e.target.closest('[data-bc="artist"]'); if (a) { S.alI = -1; renderAll(); schedSave(); return }
  const fpEl = e.target.closest('[data-fp]'); if (fpEl) { navigateFolderTo(fpEl.dataset.fp); return }
  if (e.target.closest('[data-fp-root]')) { S.folderStack = []; S.view = 'all'; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-folder-back')) { navigateFolderUp(); return }
  if (e.target.closest('#btn-lyrics-back')) {
    if (S.prevView) { S.view = S.prevView; S.prevView = null }
    else { S.view = 'all' }
    activeLrcTab = 'lyrics'; renderAll(); schedSave(); return
  }
  if (e.target.closest('#btn-search-back')) { exitSearch(); return }
  if (e.target.closest('#btn-pl-back')) { S.aPl = null; renderAll(); schedSave(); return }
  if (e.target.closest('#btn-fav-back')) { S.aF = null; renderAll(); schedSave(); return }
})

function exitSearch() {
  $('search-input').value = ''; S.q = ''; $('search-clear').classList.add('hidden'); $('search-back').classList.remove('visible')
  if (S.prevView) {
    S.view = S.prevView; S.aF = S._prevAF || null; S.aPl = S._prevAPl || null
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
  const ni = e.target.closest('.nav-item'); if (ni) { if (ni.dataset.view) { S.prevView = null; S.view = ni.dataset.view; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null; if (ni.dataset.view === 'all') S.folderStack = []; activeLrcTab = 'lyrics'; renderAll(); schedSave(); return } if (ni.id === 'btn-add-folder') { await importFolder(); return } }
  const fa = e.target.closest('[data-fa]'); if (fa) { const k = fa.dataset.fa; const xf = S.xf || new Set(); xf.has(k) ? xf.delete(k) : xf.add(k); S.xf = xf; renderSB(); schedSave(); return }
  const fpEl = e.target.closest('.folder-item[data-fp]'); if (fpEl) {
    S.view = 'all'; S.aI = -1; S.alI = -1; S.aPl = null; S.aF = null
    const node = findNodeByPath(S.folderTree, fpEl.dataset.fp)
    S.folderStack = [node ? node.path : fpEl.dataset.fp]
    renderAll(); schedSave(); return
  }
  if (e.target.closest('[data-fvid]')) { S.view = 'all'; S.aF = e.target.closest('[data-fvid]').dataset.fvid; S.aPl = null; S.aI = -1; S.alI = -1; S.prevView = null; renderAll(); schedSave(); return }
  if (e.target.closest('[data-plid]')) { S.view = 'all'; S.aPl = e.target.closest('[data-plid]').dataset.plid; S.aF = null; S.aI = -1; S.alI = -1; S.prevView = null; renderAll(); schedSave(); return }
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
  const m = $('ctx-menu')
  const folderName = folderPath.split(/[\\/]/).pop()
  m.innerHTML = `<button data-a="sfrem">从扫描列表移除 "${esc(folderName)}"</button>`
  m.classList.remove('hidden')
  m.style.left = Math.min(e.clientX, window.innerWidth - 200) + 'px'
  m.style.top = Math.min(e.clientY, window.innerHeight - 100) + 'px'
  m.onclick = async () => {
    hC()
    const ok = await showConfirm('移除文件夹', `确定从扫描列表移除"${folderName}"吗？`)
    if (!ok) return
    fp = fp.filter(p => p !== folderPath)
    S.folderTree = S.folderTree.filter(n => n.path !== folderPath && !n.path.startsWith(folderPath + '/'))
    hC(); await rescan()
    schedSave()
  }
  $('ctx-playlist-sub').classList.add('hidden')
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
  if (S.playing) { audio.pause(); S.playing = false } else {
    if (!pl.length && S.all.length) { pl = S.all; playT(0) }
    else if (pl.length) { audio.play().catch(() => { }); S.playing = true }
  }
  updPlayBtn(); schedSave()
})
$('btn-prev').addEventListener('click', prv)
$('btn-next').addEventListener('click', nxt)
$('btn-mode').addEventListener('click', () => { S.mode = (S.mode + 1) % 4; apMode(); schedSave() })

$('volume-bar').addEventListener('click', e => {
  const r = e.target.getBoundingClientRect(); const p = (e.clientX - r.left) / r.width
  S.vol = Math.round(p * 100); S.pVol = S.vol; audio.volume = S.vol / 100
  $('volume-fill').style.width = S.vol + '%'; $('volume-text').textContent = S.vol; schedSave()
})
$('btn-volume').addEventListener('click', () => {
  S.muted = !S.muted
  audio.volume = S.muted ? 0 : S.vol / 100
  $('volume-fill').style.width = S.muted ? '0%' : S.vol + '%'
  $('volume-text').textContent = S.muted ? '0' : S.vol; schedSave()
})

;(function initProgressBar() {
  const bar = $('progress-bar')
  const fill = $('progress-fill')
  const handle = $('progress-handle')
  let dragging = false

  function getRatio(e) {
    const r = bar.getBoundingClientRect()
    return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width))
  }

  function showProgress(ratio) {
    fill.classList.add('no-transition')
    fill.style.width = (ratio * 100) + '%'
    handle.style.left = (ratio * 100) + '%'
    if (audio.duration) $('progress-current').textContent = fmtTime(ratio * audio.duration)
  }

  bar.addEventListener('mousedown', e => {
    if (!audio.duration) return
    dragging = true
    e.preventDefault()
    showProgress(getRatio(e))
  })

  window.addEventListener('mousemove', e => {
    if (!dragging) return
    showProgress(getRatio(e))
  })

  window.addEventListener('mouseup', e => {
    if (!dragging) return
    dragging = false
    fill.classList.remove('no-transition')
    audio.currentTime = getRatio(e) * audio.duration
  })
})()
$('search-input').addEventListener('keydown', e => {
  if (e.key === 'Enter') {
    const q = $('search-input').value
    S.q = q; $('search-back').classList.toggle('visible', !!S.q); $('search-clear').classList.toggle('hidden', !S.q); $('search-input').classList.toggle('has-back', !!S.q)
    if (!S.prevView) { S.prevView = S.view; S._prevAF = S.aF; S._prevAPl = S.aPl }
    S.view = 'all'; S.aF = null; S.aPl = null; S.folderStack = []
    if (S.tI >= 0 && pl[S.tI]) S.playingTid = pl[S.tI].id
    pl = S.all.filter(t => (t.name || '').toLowerCase().includes(S.q.toLowerCase()) || (t.artist || '').toLowerCase().includes(S.q.toLowerCase()))
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
  const p = S.dur ? (S.cTime / S.dur) * 100 : 0
  $('progress-fill').style.width = p + '%'; $('progress-handle').style.left = p + '%'
  $('progress-current').textContent = fmtTime(S.cTime)
  // Update lyrics highlight and auto-scroll
  if (lrc && lrc.length && S.view === 'lyrics' && activeLrcTab === 'lyrics') {
    const scroll = $('lyrics-lines-scroll')
    const lines = scroll ? scroll.querySelectorAll('.lc-line') : []
    let activeIdx = -1
    for (let i = lrc.length - 1; i >= 0; i--) {
      if (S.cTime >= lrc[i].time) { activeIdx = i; break }
    }
    lines.forEach((l, i) => l.classList.toggle('active', i === activeIdx))
    if (activeIdx >= 0 && scroll) {
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
audio.addEventListener('loadedmetadata', () => { S.dur = audio.duration; $('progress-duration').textContent = fmtTime(S.dur) })
audio.addEventListener('ended', hEnd)
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
let sidebarOpacityTimer
$('sidebar-opacity').addEventListener('input', e => {
  S.sidebarOpacity = parseInt(e.target.value)
  $('sidebar-opacity-val').textContent = S.sidebarOpacity + '%'
  const isDark = S.theme === 'dark'
  const sidebarAlpha = S.sidebarOpacity / 100
  const sbBg = isDark ? `rgba(16,16,22,${sidebarAlpha})` : `rgba(245,245,247,${sidebarAlpha})`
  $('sidebar').style.background = sbBg
  $('sidebar').style.borderRight = `1.5px solid ${isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)'}`
  clearTimeout(sidebarOpacityTimer)
  sidebarOpacityTimer = setTimeout(schedSave, 200)
})
$('titlebar-opacity').addEventListener('input', e => {
  S.titlebarOpacity = parseInt(e.target.value)
  $('titlebar-opacity-val').textContent = S.titlebarOpacity + '%'
  const isDark = S.theme === 'dark'
  const titlebarAlpha = S.titlebarOpacity / 100
  const tbBg = isDark ? `rgba(16,16,22,${titlebarAlpha})` : `rgba(255,255,255,${titlebarAlpha})`
  $('titlebar').style.background = tbBg
  clearTimeout(window._titlebarSaveTimer)
  window._titlebarSaveTimer = setTimeout(schedSave, 200)
})
$('player-opacity').addEventListener('input', e => {
  S.playerOpacity = parseInt(e.target.value)
  $('player-opacity-val').textContent = S.playerOpacity + '%'
  const isDark = S.theme === 'dark'
  const playerAlpha = S.playerOpacity / 100
  const pbBg = isDark ? `rgba(20,20,28,${playerAlpha})` : `rgba(255,255,255,${playerAlpha})`
  $('player-bar').style.background = pbBg
  clearTimeout(window._playerSaveTimer)
  window._playerSaveTimer = setTimeout(schedSave, 200)
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
    S.bgData = r.dataUrl; S.bgPath = r.path
    $('bg-preview').style.backgroundImage = `url(${r.dataUrl})`
    $('bg-preview-wrap').classList.remove('hidden')
    $('btn-bg-upload').classList.add('hidden')
    updSUI(); apTh(); apThBg(); schedSave()
  } catch (e) { alert('选择背景图片失败: ' + e.message) }
})
$('btn-bg-remove').addEventListener('click', () => { S.bgData = null; S.bgPath = null; S.bgBlur = 0; $('bg-preview').style.backgroundImage = ''; $('bg-preview-wrap').classList.add('hidden'); $('btn-bg-upload').classList.remove('hidden'); idbSet('cache', 'bgImage', null).catch(() => {}); apThBg(); apTh(); schedSave() })

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
  $('img-edit-img').src = S.bgData
  const vw = window.innerWidth, vh = window.innerHeight
  $('img-editor-preview').style.height = Math.round(320 * vh / vw) + 'px'
  S._imgEditState = S._imgEditState || {}
  $('img-opacity').value = S.ovl; $('img-opacity-val').textContent = S.ovl + '%'
  $('img-blur').value = S.bgBlur || 0; $('img-blur-val').textContent = (S.bgBlur || 0) + 'px'
  $('img-editor-modal').classList.remove('hidden')
  $('img-edit-img').onload = () => {
    if (!S._imgEditState.zoomPct) {
      autoFitImg()
    } else {
      applyImgTransform()
    }
  }
  if (S._imgEditState.zoomPct) {
    $('img-zoom').value = S._imgEditState.zoomPct; $('img-zoom-val').textContent = S._imgEditState.zoomPct + '%'
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
    $('img-zoom').value = S._imgEditState.zoomPct; $('img-zoom-val').textContent = S._imgEditState.zoomPct + '%'
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
  const pw = preview.clientWidth, ph = preview.clientHeight
  if (!pw || !ph) return
  const natW = img.naturalWidth || img.width, natH = img.naturalHeight || img.height
  if (!natW || !natH) return
  const vw = window.innerWidth, vh = window.innerHeight
  const z = parseInt($('img-zoom').value) / 100
  // Preview: scale is relative to filling the preview box, proportionally
  const previewFill = Math.max(pw / natW, ph / natH)
  const viewportFill = Math.max(vw / natW, vh / natH)
  const previewScale = previewFill * z
  const imgW = natW * previewScale, imgH = natH * previewScale
  const opacity = parseInt($('img-opacity').value) / 100
  const blur = parseInt($('img-blur').value)
  img.style.width = imgW + 'px'; img.style.height = imgH + 'px'
  img.style.left = ((pw - imgW) / 2 + imgDrag.posX) + 'px'
  img.style.top = ((ph - imgH) / 2 + imgDrag.posY) + 'px'
  img.style.opacity = opacity; img.style.filter = blur > 0 ? `blur(${blur}px)` : ''
  const oxPct = pw ? Math.round((-imgDrag.posX * 2 / pw) * 100) : 0
  const oyPct = ph ? Math.round((-imgDrag.posY * 2 / ph) * 100) : 0
  $('img-offset-text').textContent = `X: ${oxPct}% / Y: ${oyPct}%`
}

function commitImgChanges() {
  S.ovl = parseInt($('img-opacity').value)
  S.bgBlur = parseInt($('img-blur').value)
  S._imgEditState = S._imgEditState || {}
  const img = $('img-edit-img')
  const natW = img.naturalWidth || 1, natH = img.naturalHeight || 1
  const vw = window.innerWidth, vh = window.innerHeight
  const z = parseInt($('img-zoom').value) / 100
  const preview = $('img-editor-preview')
  const pw = preview.clientWidth, ph = preview.clientHeight
  const pFill = Math.max(pw / natW, ph / natH)
  const vFill = Math.max(vw / natW, vh / natH)
  const previewImgW = natW * pFill * z
  const mainImgW = natW * vFill * z
  const ratio = mainImgW / previewImgW
  S._imgEditState.zoomPct = parseInt($('img-zoom').value)
  S._imgEditState.natW = natW
  S._imgEditState.natH = natH
  S._imgEditState.posX = imgDrag.posX * ratio
  S._imgEditState.posY = imgDrag.posY * ratio
  S._imgEditState.vw = vw; S._imgEditState.vh = vh
  schedSave(); apThBg()
}

function collectAllTracks(node) {
  let all = []
  for (const c of node.children) { all = all.concat(collectAllTracks(c)) }
  return all.concat(node.tracks)
}

function hasMusicRecursive(node) {
  if (!node) return false
  if (node.tracks && node.tracks.length > 0) return true
  for (const c of (node.children || [])) { if (hasMusicRecursive(c)) return true }
  return false
}

// Panel
$('btn-playlist-panel').addEventListener('click', () => { $('playlist-panel').classList.remove('hidden'); renderPanel() })
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
    const list = $('speaker-device-list'); list.innerHTML = audioOut.map((d, i) => `<button class="speaker-device-item${S.devId === d.deviceId ? ' active' : ''}" data-did="${d.deviceId}"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" class="speaker-device-icon"><polygon points="11,5 6,9 2,9 2,15 6,15 11,19"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>${d.label || '\u8bbe\u5907 ' + i}</button>`).join('')
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
  if (!e.target.closest('#ctx-menu') && !e.target.closest('#ctx-playlist-sub')) hC()
})

// Titlebar
$('btn-import').addEventListener('click', importFolder)
$('btn-min').addEventListener('click', () => api.minimize())
$('btn-max').addEventListener('click', () => api.maximize())
$('btn-close').addEventListener('click', () => api.close())

// Sel mode toggle fix
try {
  const selToggle = document.querySelector('.sel-mode-toggle')
  if (selToggle) selToggle.addEventListener('click', () => { S.selMode = !S.selMode; renderAll(); schedSave() })
} catch (e) { /* ignore */ }

// Window controls events from main
if (api.onMaximized) api.onMaximized(() => { const mi = $('max-icon'); if (mi) mi.innerHTML = '<rect x="5" y="5" width="14" height="14" rx="2"/>'; if (S.bgData) apThBg() })
if (api.onUnmaximized) api.onUnmaximized(() => { const mi = $('max-icon'); if (mi) mi.innerHTML = '<rect x="3" y="3" width="18" height="18" rx="2"/>'; if (S.bgData) apThBg() })

// === Init ===
async function init() {
  try {
    await loadS(); apTh(); apThBg(); updSUI()
    // Save state on exit
    api.onBeforeClose(() => { clearTimeout(saveTimer); saveS() })
    window.addEventListener('beforeunload', () => { clearTimeout(saveTimer); saveS() })
    let _resizeRAF = null
    window.addEventListener('resize', () => {
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
          const ext = (t.format || '').toLowerCase()
          if (ext === 'dsf' || ext === 'dff' || ext === 'dsd') {
            try { const wavPath = await api.decodeDSD(t.path); if (wavPath) { audio.src = 'file:///' + wavPath.replace(/\\/g, '/') } } catch (e) { audio.src = 'file:///' + t.path.replace(/\\/g, '/') }
          } else { audio.src = 'file:///' + t.path.replace(/\\/g, '/') }
          audio.currentTime = S.cTime || 0
          if (S.devId && audio.setSinkId) try { await audio.setSinkId(S.devId) } catch (e) { /* ignore */ }
        } catch (e) { /* ignore */ }
      }
    }
    apMode(); renderAll()
    audio.volume = S.muted ? 0 : S.vol / 100
    $('volume-fill').style.width = S.muted ? '0%' : S.vol + '%'; $('volume-text').textContent = S.muted ? '0' : S.vol
  } catch (e) {
    /* silent init error */
  }
}

init()

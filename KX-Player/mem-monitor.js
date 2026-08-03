// === Renderer Memory Monitor ===
// Captures JS heap usage via performance.memory at key lifecycle checkpoints
// and pushes samples to the main process where the log file is owned.
//
// Logs go to KX-Player/memory.log (managed by electron/memMonitor.ts).

const SAMPLE_INTERVAL_MS = 10_000

let _started = false
let _interval = null
let _lastTick = 0
const _sentHistory = new Map() // event::extra -> ts, to suppress duplicate rapid samples

function _pickMem() {
  const out = {}
  // Preferred: Chromium's performance.memory (only meaningful with the
  // `--enable-precise-memory-info` switch in main.ts).
  if (typeof performance !== 'undefined' && performance && performance.memory) {
    const m = performance.memory
    out.jsHeapUsedMb = m.usedJSHeapSize / (1024 * 1024)
    out.jsHeapTotalMb = m.totalJSHeapSize / (1024 * 1024)
    out.heapUsedMb = m.usedJSHeapSize / (1024 * 1024)
    out.heapTotalMb = m.totalJSHeapSize / (1024 * 1024)
  }
  // Fallback / supplement: electronAPI.rendererMemoryUsage() pulled via main.
  // (Used by the init timer to short-circuit a noisy zero-only stream.)
  return out
}

// Pull a real reading via the IPC bridge, write it as a renderer sample, and
// return whether we got a non-zero result. Used by the periodic tick so we
// can correct any zeros from performance.memory.
async function _pullFromMain() {
  try {
    if (typeof window === 'undefined' || !window.electronAPI || !window.electronAPI.rendererMemoryUsage) return null
    const data = await window.electronAPI.rendererMemoryUsage()
    if (!data) return null
    return {
      jsHeapUsedMb: data.jsHeapUsed / (1024 * 1024),
      jsHeapTotalMb: data.jsHeapTotal / (1024 * 1024),
      rendererHeapMb: data.heapUsed / (1024 * 1024),
      rendererHeapTotalMb: data.heapTotal / (1024 * 1024),
      rendererExtMb: data.external ? (data.external / (1024 * 1024)) : undefined,
      rssMb: data.rss / (1024 * 1024),
    }
  } catch {
    return null
  }
}

function _send(event, extra) {
  const w = window
  if (!w || !w.electronAPI || typeof w.electronAPI.reportMemSample !== 'function') return
  const prev = _lastTick
  _lastTick = Date.now()
  const sample = {
    ts: new Date().toISOString(),
    source: 'renderer',
    event,
    pid: 'renderer',
    deltaMs: prev ? _lastTick - prev : undefined,
    extra,
    ..._pickMem(),
  }
  try { w.electronAPI.reportMemSample(sample) } catch { /* ignore */ }
}

export function startMemMonitor() {
  if (_started) return
  _started = true
  _send('renderer:start')
  // Schedule a pull on each tick so renderer samples reflect real numbers
  // (performance.memory can be zero in some Electron builds).
  _interval = setInterval(() => {
    if (typeof window !== 'undefined' && window.electronAPI && window.electronAPI.rendererMemoryUsage) {
      window.electronAPI.rendererMemoryUsage().then(data => {
        if (!data) { _send('tick'); return }
        _sendViaBridge('tick', data, '')
      }).catch(() => { _send('tick') })
    } else {
      _send('tick')
    }
  }, SAMPLE_INTERVAL_MS)
  // Pull a real measurement via IPC immediately so we can confirm the precise
  // memory flag took effect; falls back silently if unavailable.
  if (typeof window !== 'undefined' && window.electronAPI && window.electronAPI.rendererMemoryUsage) {
    window.electronAPI.rendererMemoryUsage().then(data => {
      if (!data) { _send('renderer:initialRead', 'no-data'); return }
      _sendViaBridge('initialRead', data, 'pulled-from-main')
    }).catch(() => { _send('renderer:initialRead', 'error') })
  }
  // Listen for memory pressure signals that Chromium may emit
  if (typeof performance !== 'undefined' && performance.addEventListener) {
    try {
      performance.addEventListener('memorypressure', () => _send('memorypressure'))
    } catch { /* not all versions support this */ }
  }
  // Listen for visibility changes to mark leak contexts
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') _send('visibility:hidden')
    else _send('visibility:visible')
  })
  // Detailed resource snapshot every 30s
  let _detailTimer = null
  if (_detailTimer) clearInterval(_detailTimer)
  _detailTimer = setInterval(() => {
    try {
      const detail = _collectDetailedResources()
      _send('resourceSnapshot', detail)
    } catch { /* ignore */ }
  }, 30000)
  // First snapshot immediately
  setTimeout(() => {
    try {
      const detail = _collectDetailedResources()
      _send('resourceSnapshot', detail)
    } catch { /* ignore */ }
  }, 5000)
  // Listen for audio events too
  document.querySelectorAll('audio').forEach(a => {
    if (!a._memMonitorBound) {
      a._memMonitorBound = true
      const srcNow = () => {
        try {
          const len = (a.currentSrc || a.src || '').length
          _send('audio:src', `len=${len} rs=${a.readyState} ns=${a.networkState}${a.paused ? ' paused' : ''} id=${a.id || ''}`)
        } catch {}
      }
      a.addEventListener('loadedmetadata', () => _send('audio:loadedmetadata', `dur=${a.duration}`))
      a.addEventListener('canplay',     () => srcNow())
      a.addEventListener('play',        () => _send('audio:play', 'play'))
      a.addEventListener('pause',       () => _send('audio:pause', 'pause'))
      a.addEventListener('ended',       () => _send('audio:ended', 'ended'))
      a.addEventListener('error',       () => _send('audio:error', `code=${a.error?.code || 0}`))
      a.addEventListener('seeked',      () => _send('audio:seeked', `t=${a.currentTime}`))
      srcNow()
    }
  })
}

// Send a sample whose memory numbers come exclusively from the IPC pull,
// bypassing the (possibly zero) performance.memory fallback in _pickMem.
function _sendViaBridge(event, data, extra) {
  const w = window
  if (!w || !w.electronAPI || typeof w.electronAPI.reportMemSample !== 'function') return
  const prev = _lastTick; _lastTick = Date.now()
  const sample = {
    ts: new Date().toISOString(),
    source: 'renderer',
    event,
    pid: 'renderer',
    jsHeapUsedMb: data.jsHeapUsed / 1048576,
    jsHeapTotalMb: data.jsHeapTotal / 1048576,
    rssMb: (data.rss || 0) / 1048576,
    deltaMs: prev ? _lastTick - prev : undefined,
    extra,
  }
  if (!Number.isFinite(sample.jsHeapUsedMb) || sample.jsHeapUsedMb === 0) {
    sample.jsHeapUsedMb = (data.heapUsed || 0) / 1048576
    sample.jsHeapTotalMb = (data.heapTotal / 1048576) || 0
    sample.rendererHeapMb = (data.heapUsed || 0) / 1048576
    sample.rendererHeapTotalMb = (data.heapTotal || 0) / 1048576
    sample.rendererExtMb = (data.external || 0) / 1048576
  }
  try { w.electronAPI.reportMemSample(sample) } catch { /* ignore */ }
}

// Detailed page-resource snapshot. Returns a string for log embedding.
// Captures things Chrome itself doesn't account for in JS heaps but does pin:
//   - decoded <img> ImageBitmap sizes
//   - <audio>/<video> state, currentSrc, readyState, networkState
//   - canvas / WebGL texture allocations
//   - AudioContext (web audio API) active node counts
//   - document.fonts loaded entries
//   - CSS url() occurrences
//   - MediaSource / Blob URL count
//   - <link rel=stylesheet>/<script> counts
function _collectDetailedResources() {
  const out = {}
  try {
    // Images
    const imgs = Array.from(document.images || [])
    let decodedMB = 0
    let dataUriCount = 0
    let loadedCount = 0
    let fileUriCount = 0
    for (const img of imgs) {
      if (img.src && img.src.startsWith('data:')) dataUriCount++
      else if (img.src && img.src.startsWith('file:')) fileUriCount++
      if (img.naturalWidth) {
        decodedMB += (img.naturalWidth * img.naturalHeight * 4) / (1024 * 1024)
        loadedCount++
      }
    }
    out.imgs = `${imgs.length}(file=${fileUriCount},data=${dataUriCount},~${decodedMB.toFixed(1)}MB)`

    // Audio / Video + buffer info
    const audios = Array.from(document.querySelectorAll('audio'))
    const videos = Array.from(document.querySelectorAll('video'))
    out.audio = audios.length
      ? audios.map(a => {
          const len = (a.currentSrc || a.src || '').length
          return `len=${len} rs=${a.readyState} ns=${a.networkState}${a.paused ? '/paused' : '/playing'}`
        }).join(';')
      : '0'
    out.video = videos.length

    // AudioBuffersource / OfflineAudioContext reachable via webkitAudioContext
    try {
      const wAny = window
      if (wAny.__audioContextCount !== undefined) {
        out.audioCtx = wAny.__audioContextCount
      }
    } catch {}

    // Canvas
    out.canvas = document.querySelectorAll('canvas').length

    // document.fonts
    try {
      if (document.fonts && typeof document.fonts.size === 'number') {
        out.fonts = `${document.fonts.size}/${document.fonts.status || ''}`
      }
    } catch {}

    // CSS url() occurrences in stylesheets
    try {
      let urlCount = 0
      const sheets = Array.from(document.styleSheets || [])
      for (const s of sheets) {
        try {
          const rules = s.cssRules || s.rules
          if (!rules) continue
          for (const r of rules) {
            const txt = r.cssText || ''
            const m = txt.match(/url\(([^)]+)\)/g)
            if (m) urlCount += m.length
          }
        } catch {}
      }
      out.cssUrls = urlCount
    } catch {}

    // Blob URLs in DOM
    try {
      const all = document.querySelectorAll('[src^="blob:"], [href^="blob:"]')
      out.blobs = all.length
    } catch {}

    // <link>/<script>/<style>
    out.link = document.querySelectorAll('link[rel="stylesheet"]').length
    out.scripts = document.querySelectorAll('script').length

    // Custom counters (cover cache)
    out.coverCache = (window.__coverCacheSize?.() || 0) + '@' +
                     Math.round((window.__coverCacheBytes?.() || 0) / 1024) + 'KB'

    // CSS background-image URLs
    try {
      let bgUrlCount = 0
      const all = document.querySelectorAll('*')
      for (const el of all) {
        try {
          const cs = getComputedStyle(el)
          if (cs && cs.backgroundImage && cs.backgroundImage.includes('url(')) bgUrlCount++
        } catch {}
      }
      out.bgUrls = bgUrlCount
    } catch {}

    // Performance: jsHeapSizeLimit
    try {
      const perf = performance
      if (perf && perf.memory) {
        const m = perf.memory
        out.perf = `used=${(m.usedJSHeapSize/1048576).toFixed(1)}MB total=${(m.totalJSHeapSize/1048576).toFixed(1)}MB limit=${(m.jsHeapSizeLimit/1048576).toFixed(0)}MB`
      }
    } catch {}

    // IPC channel: capture how many ipcRenderer listeners may be installed
    try {
      const lst = (window.electronAPI && window.__listenerProbe) || null
      if (lst) out.ipcL = lst
    } catch {}
  } catch (e) { /* ignore */ }

  const parts = []
  for (const k of Object.keys(out)) {
    parts.push(`${k}=${out[k]}`)
  }
  return parts.join(' ')
}

// Expose a global counter for IPC listener registration, populated by
// caller code when ipcRenderer.on/onOnce is used. This is opt-in to avoid
// intrusive instrumentation.
if (typeof window !== 'undefined' && !window.__listenerProbe) {
  window.__listenerProbe = () => {
    return Object.keys(window).filter(k => k.startsWith('__on_')).length
  }
}

// Explicit mark at a key checkpoint. Use a tag to dedupe within a short window.
export function mark(event, extra, opts = {}) {
  if (typeof window === 'undefined') return
  const dedupeMs = opts.dedupeMs ?? 0
  if (dedupeMs > 0) {
    const key = `${event}::${extra || ''}`
    const now = Date.now()
    const prev = _sentHistory.get(key) || 0
    if (now - prev < dedupeMs) return
    _sentHistory.set(key, now)
  }
  _send(event, extra)
}

export function stopMemMonitor() {
  if (_interval) { clearInterval(_interval); _interval = null }
  _started = false
}

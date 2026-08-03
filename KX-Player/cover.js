// === KX-Player Cover Loading & Caching ===
// On-demand cover system: covers are stored as compressed JPEG files in filesystem,
// NOT in SQLite. Loaded on demand via IPC, with deduplication and LRU cache.

import { api } from './api.js'
import { mark as markMem } from './mem-monitor.js'

const _coverCache = new Map()          // id -> dataUrl string (cover data)
const _coverLoading = new Set()        // Set of trackIds currently being loaded (prevents duplicate requests)
const _pendingBatch = []               // Batched requests (string[])
const _pendingSet = new Set()
let _batchTimer = null
const BATCH_DELAY = 50                 // ms to collect requests before sending
const MAX_COVER_CACHE = 300            // Capped; dataUrl strings ~30-80KB each → 9-24MB ceiling
const MAX_BATCH_SIZE = 100             // Smaller batches for better responsiveness
const _coverLoadedCallbacks = new Set() // Use Set (deduplicated) instead of array; entries are stable
let _coverCacheBytes = 0               // Track approximate bytes for size-based eviction

function _setCachedCover(id, data) {
  // `data` may be: empty string (no cover), or a file:// URL pointing to a
  // JPEG on disk. We never keep base64 strings here, so each cache entry is
  // tiny regardless of source image size.
  if (!data) {
    if (!_coverCache.has(id)) _coverCache.set(id, '')
    return
  }
  if (_coverCache.has(id)) {
    const old = _coverCache.get(id)
    _coverCacheBytes -= old ? old.length : 0
    _coverCache.delete(id)
  }
  _coverCacheBytes += data.length
  _coverCache.set(id, data)
}

function _getCachedCover(id) {
  if (!_coverCache.has(id)) return null
  const data = _coverCache.get(id)
  // LRU touch: re-insert with same key to update insertion order.
  if (data) {
    _coverCache.delete(id)
    _coverCache.set(id, data)
  }
  return data || null
}

function _evictCoverCache() {
  if (_coverCache.size <= MAX_COVER_CACHE) return
  const evictCount = _coverCache.size - MAX_COVER_CACHE + 50
  const it = _coverCache.keys()
  for (let i = 0; i < evictCount; i++) {
    const next = it.next()
    if (next.done) break
    const v = _coverCache.get(next.value)
    if (v) _coverCacheBytes -= v.length
    _coverCache.delete(next.value)
  }
}

export function _getCoverData(track) {
  if (!track) return null
  // Track's own data wins: covers inlined in scan result (rare path).
  if (track.coverData) return track.coverData
  if (track.albumCoverData) return track.albumCoverData
  // Cache lookup (filesystem lazy load path).
  const cached = _getCachedCover(track.id)
  return cached || null
}

// Register a callback to be called when covers are loaded
export function _onCoversLoaded(callback) {
  if (typeof callback === 'function') {
    _coverLoadedCallbacks.add(callback)
    return () => _coverLoadedCallbacks.delete(callback)
  }
}

function _flushBatch() {
  _batchTimer = null
  _lastFlushAt = Date.now()
  if (!_pendingBatch.length) return
  const batch = _pendingBatch.splice(0, MAX_BATCH_SIZE)
  for (const id of batch) _pendingSet.delete(id)
  // Filter out already loading or cached
  const toLoad = batch.filter(id => !_coverCache.has(id) && !_coverLoading.has(id))
  if (!toLoad.length) {
    if (_pendingBatch.length) _flushBatch()
    return
  }
  markMem('renderer:cover:request', `batch=${toLoad.length}`)
  for (const id of toLoad) _coverLoading.add(id)
  api.getTrackCovers(toLoad).then(covers => {
    let hasNewCovers = false
    const loadedIds = []
    let bytesIn = 0
    for (const [id, data] of Object.entries(covers)) {
      _setCachedCover(id, data || '')
      _coverLoading.delete(id)
      if (data) {
        hasNewCovers = true
        loadedIds.push(id)
        bytesIn += data.length
      }
    }
    // Clean up loading state for IDs that returned no cover
    for (const id of toLoad) {
      if (_coverLoading.has(id)) {
        if (!_coverCache.has(id)) _setCachedCover(id, '')
        _coverLoading.delete(id)
      }
    }
    _evictCoverCache()
    markMem('renderer:cover:response', `loaded=${loadedIds.length} bytes~=${(bytesIn / 1024).toFixed(0)}KB cacheSize=${_coverCache.size} cacheBytes~=${(_coverCacheBytes / 1024 / 1024).toFixed(1)}MB`)
    // Notify listeners that covers have been loaded
    if (hasNewCovers) {
      for (const callback of _coverLoadedCallbacks) {
        try { callback(loadedIds) } catch (e) { console.error('[cover] callback error:', e) }
      }
    }
  }).catch(() => {
    for (const id of toLoad) _coverLoading.delete(id)
  })
  // Process remaining
  if (_pendingBatch.length) {
    _batchTimer = setTimeout(_flushBatch, BATCH_DELAY)
  }
}

export function _loadCoversForTrackIds(trackIds) {
  if (!trackIds || !trackIds.length) return
  let added = 0
  for (const id of trackIds) {
    if (!_coverCache.has(id) && !_coverLoading.has(id) && !_pendingSet.has(id)) {
      _pendingBatch.push(id)
      _pendingSet.add(id)
      added++
    }
  }
  if (added && !_batchTimer && _pendingBatch.length) {
    // Throttle: never schedule a flush more often than every 80ms so quick
    // bursts of onVisibleItems calls (e.g. searching through a long list)
    // collapse into a single IPC batch.
    const wait = _lastFlushAt ? Math.max(0, 80 - (Date.now() - _lastFlushAt)) : 0
    _batchTimer = setTimeout(_flushBatch, wait)
  }
}

let _lastFlushAt = 0

export function _preloadVisibleCovers(tracks) {
  if (!tracks || !tracks.length) return
  // Cap to first 60; avoid copying entire visible window when it's huge.
  const slice = tracks.length > 60 ? tracks.slice(0, 60) : tracks
  const ids = slice.filter(t => t && t.id && !_coverCache.has(t.id) && !t.coverData && !t.albumCoverData).map(t => t.id)
  if (ids.length) _loadCoversForTrackIds(ids)
}

export function _updateFolderTreeCovers(nodes, covers) {
  if (!covers || !nodes) return
  for (const n of nodes) {
    if (covers[n.path]) n.coverData = covers[n.path]
    if (n.children && n.children.length) _updateFolderTreeCovers(n.children, covers)
  }
}

export function _clearCoverCache() {
  _coverCacheBytes = 0
  _coverCache.clear()
  _coverLoading.clear()
  _pendingBatch.length = 0
  _pendingSet.clear()
  if (_batchTimer) { clearTimeout(_batchTimer); _batchTimer = null }
}

// Expose cache stats for the memory monitor. Read-only; updates when the
// cache mutates. Returned as a small object literal to keep call cost low.
export function _getCoverCacheStats() {
  return { size: _coverCache.size, bytes: _coverCacheBytes }
}
if (typeof window !== 'undefined') {
  window.__coverCacheBytes = () => _coverCacheBytes
  window.__coverCacheSize = () => _coverCache.size
  window.__getCoverCacheStats = _getCoverCacheStats
}

// === KX-Player Cover Loading & Caching ===
// On-demand cover system: covers are stored as compressed JPEG files in filesystem,
// NOT in SQLite. Loaded on demand via IPC, with deduplication and LRU cache.

import { api } from './api.js'

const _coverCache = new Map()
const _coverLoading = new Set() // Set of trackIds currently being loaded (prevents duplicate requests)
const _pendingBatch = [] // Batched requests (string[])
const _pendingSet = new Set()
let _batchTimer = null
const BATCH_DELAY = 50 // ms to collect requests before sending
const MAX_COVER_CACHE = 500
const MAX_BATCH_SIZE = 100 // Smaller batches for better responsiveness
const _coverLoadedCallbacks = [] // Callbacks to fire when covers are loaded

function _setCachedCover(id, data) {
  if (_coverCache.has(id)) _coverCache.delete(id)
  _coverCache.set(id, { coverData: data, albumCoverData: null })
}

function _getCachedCover(id) {
  if (!_coverCache.has(id)) return null
  const cached = _coverCache.get(id)
  _coverCache.delete(id)
  _coverCache.set(id, cached)
  return cached
}

function _evictCoverCache() {
  if (_coverCache.size <= MAX_COVER_CACHE) return
  const evictCount = _coverCache.size - MAX_COVER_CACHE + 50
  const it = _coverCache.keys()
  for (let i = 0; i < evictCount; i++) {
    const next = it.next()
    if (next.done) break
    _coverCache.delete(next.value)
  }
}

export function _getCoverData(track) {
  if (!track) return null
  // Check track's own data first (always available from fresh scan result).
  // This prevents the cache from returning null when the track has albumCoverData
  // but no individual cover file saved yet.
  if (track.coverData) return track.coverData
  if (track.albumCoverData) return track.albumCoverData
  // Fall back to cache (from filesystem lazy load on restart)
  const cached = _getCachedCover(track.id)
  if (cached) return cached.coverData || cached.albumCoverData || null
  return null
}

// Register a callback to be called when covers are loaded
export function _onCoversLoaded(callback) {
  if (typeof callback === 'function') {
    _coverLoadedCallbacks.push(callback)
  }
}

function _flushBatch() {
  _batchTimer = null
  if (!_pendingBatch.length) return
  const batch = _pendingBatch.splice(0, MAX_BATCH_SIZE)
  for (const id of batch) _pendingSet.delete(id)
  // Filter out already loading or cached
  const toLoad = batch.filter(id => !_coverCache.has(id) && !_coverLoading.has(id))
  if (!toLoad.length) {
    if (_pendingBatch.length) _flushBatch()
    return
  }
  for (const id of toLoad) _coverLoading.add(id)
  api.getTrackCovers(toLoad).then(covers => {
    let hasNewCovers = false
    const loadedIds = []
    for (const [id, data] of Object.entries(covers)) {
      _setCachedCover(id, data)
      _coverLoading.delete(id)
      hasNewCovers = true
      loadedIds.push(id)
    }
    // Clean up loading state for IDs that returned no cover
    for (const id of toLoad) {
      if (_coverLoading.has(id) && !_coverCache.has(id)) {
        _setCachedCover(id, null)
      }
      _coverLoading.delete(id)
    }
    _evictCoverCache()
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
  for (const id of trackIds) {
    if (!_coverCache.has(id) && !_coverLoading.has(id) && !_pendingSet.has(id)) {
      _pendingBatch.push(id)
      _pendingSet.add(id)
    }
  }
  if (!_batchTimer && _pendingBatch.length) {
    _batchTimer = setTimeout(_flushBatch, BATCH_DELAY)
  }
}

export function _preloadVisibleCovers(tracks) {
  if (!tracks || !tracks.length) return
  const visible = tracks.slice(0, Math.min(80, tracks.length))
  const ids = visible.filter(t => t && t.id && !_coverCache.has(t.id) && !t.coverData && !t.albumCoverData).map(t => t.id)
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
  _coverCache.clear()
  _coverLoading.clear()
  _pendingBatch.length = 0
  _pendingSet.clear()
  if (_batchTimer) { clearTimeout(_batchTimer); _batchTimer = null }
}

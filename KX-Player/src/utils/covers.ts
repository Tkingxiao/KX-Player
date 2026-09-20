/**
 * 封面按需加载系统（移植自旧 cover.js + script.js 文件夹封面兜底）。
 * - LRU 缓存 300 条（file:// URL 字符串，非字节）
 * - 50ms 批量收集 + 80ms 节流，单批 ≤100
 * - 虚拟列表可见区预载 ≤60
 * - 文件夹封面独立队列（40ms 合批、批 ≤80）
 */
import { api } from '@/bridge/ipc'

const COVER_LRU_MAX = 300
const FLUSH_MS = 80
const COLLECT_MS = 50
const MAX_BATCH = 100
const PRELOAD_LIMIT = 60
const FOLDER_FLUSH_MS = 40
const FOLDER_BATCH = 80

const cache = new Map<string, string>()
const loading = new Set<string>()
const pending = new Map<string, Set<(url: string | null) => void>>()

const folderCache = new Map<string, string>()
const folderPending = new Map<string, Set<(url: string | null) => void>>()
let folderTimer: ReturnType<typeof setTimeout> | null = null

function cachePut(key: string, url: string | null): void {
  if (!url) return
  if (cache.has(key)) cache.delete(key)
  cache.set(key, url)
  if (cache.size > COVER_LRU_MAX) {
    const first = cache.keys().next().value
    if (first !== undefined) {
      cache.delete(first)
    }
  }
}

function request(ids: string[], cb: (url: string | null) => void): void {
  for (const id of ids) {
    if (!id) continue
    if (cache.has(id)) { cb(cache.get(id)!); continue }
    let set = pending.get(id)
    if (!set) { set = new Set(); pending.set(id, set) }
    set.add(cb)
  }
  scheduleFlush()
}

let flushTimer: ReturnType<typeof setTimeout> | null = null
let lastFlush = 0

function scheduleFlush(): void {
  if (flushTimer) return
  const wait = Math.max(0, lastFlush + FLUSH_MS - Date.now())
  flushTimer = setTimeout(flush, Math.max(wait, COLLECT_MS))
}

async function flush(): Promise<void> {
  flushTimer = null
  lastFlush = Date.now()
  if (pending.size === 0) return
  const batch: string[] = []
  for (const key of pending.keys()) {
    batch.push(key)
    if (batch.length >= MAX_BATCH) break
  }
  const todo = batch.filter((id) => !loading.has(id))
  // 取出回调表后再发请求（await 期间新到的请求会进入下一轮 flush）
  const todoCbs = new Map<string, Set<(url: string | null) => void>>()
  for (const id of todo) {
    todoCbs.set(id, pending.get(id)!)
    pending.delete(id)
    loading.add(id)
  }
  if (!todo.length) return
  try {
    const result = await api.getTrackCovers(todo)
    for (const id of todo) {
      const url = result[id] || null
      if (url) cachePut(id, url)
      loading.delete(id)
      const cbs = todoCbs.get(id)
      if (cbs) for (const cb of cbs) cb(url)
    }
  } catch {
    for (const id of todo) {
      loading.delete(id)
      const cbs = todoCbs.get(id)
      if (cbs) for (const cb of cbs) cb(null)
    }
  }
}

/** 请求曲目封面（含缓存命中回调）。 */
export function requestTrackCovers(ids: (string | undefined | null)[], cb: (url: string | null) => void): void {
  const clean = ids.filter((x): x is string => !!x)
  if (!clean.length) { cb(null); return }
  request([...new Set(clean)], cb)
}

function peekCache(ids: string[]): string | null {
  for (const id of ids) {
    if (id && cache.has(id)) return cache.get(id)!
  }
  return null
}

/** 同步查询已缓存的封面（首渲染用，未命中返回 null 并异步补载）。 */
export function cachedTrackCover(ids: (string | undefined | null)[]): string | null {
  return peekCache(ids.filter((x): x is string => !!x))
}

// ── 文件夹封面 ──────────────────────────────────────────────────
/** 无封面的目录记入负缓存，避免每次渲染都重新请求（否则整屏卡片持续发 IPC）。 */
const folderMiss = new Set<string>()

function queueFolderCovers(paths: string[], cb: (url: string | null) => void): void {
  const toRequest: string[] = []
  for (const p of paths) {
    if (!p) continue
    const hit = folderCache.get(p)
    if (hit) { cb(hit); continue }
    if (folderMiss.has(p)) { cb(null); continue }
    let set = folderPending.get(p)
    if (!set) { set = new Set(); folderPending.set(p, set) }
    set.add(cb)
    toRequest.push(p)
  }
  if (toRequest.length && !folderTimer) {
    folderTimer = setTimeout(flushFolderCovers, FOLDER_FLUSH_MS)
  }
}

async function flushFolderCovers(): Promise<void> {
  folderTimer = null
  if (!folderPending.size) return
  const batch = [...folderPending.keys()].slice(0, FOLDER_BATCH)
  const batchCbs = new Map<string, Set<(url: string | null) => void>>()
  for (const p of batch) {
    batchCbs.set(p, folderPending.get(p)!)
    folderPending.delete(p)
  }
  try {
    const result = await api.getFolderCovers(batch)
    for (const p of batch) {
      const url = result[p] || null
      if (url) folderCache.set(p, url)
      else folderMiss.add(p)
      const cbs = batchCbs.get(p)
      if (cbs) for (const cb of cbs) cb(url)
    }
  } catch {
    // 网络/后端异常不算「无封面」：不入负缓存，稍后可重试
    for (const p of batch) {
      const cbs = batchCbs.get(p)
      if (cbs) for (const cb of cbs) cb(null)
    }
  }
  if (folderPending.size && !folderTimer) {
    folderTimer = setTimeout(flushFolderCovers, FOLDER_FLUSH_MS)
  }
}

/** 曲库变更/重新扫描后清空负缓存，让新增封面能被重新发现 */
export function invalidateFolderCoverMisses(): void {
  folderMiss.clear()
}

export function cachedFolderCover(path: string | null | undefined): string | null {
  return path ? folderCache.get(path) ?? null : null
}

export function requestFolderCovers(paths: (string | null | undefined)[], cb: (url: string | null) => void): void {
  const clean = paths.filter((x): x is string => !!x)
  if (!clean.length) { cb(null); return }
  queueFolderCovers([...new Set(clean)], cb)
}

/** 虚拟列表可见区预载（上限 60）。 */
export function preloadVisibleCovers(ids: (string | undefined | null)[]): void {
  const clean = [...new Set(ids.filter((x): x is string => !!x))].slice(0, PRELOAD_LIMIT)
  const missing = clean.filter((id) => !cache.has(id) && !loading.has(id))
  if (missing.length) request(missing, () => { /* 预载无需回调 */ })
}

/** 曲目所属目录（归一化） */
export function trackDirPath(path: string): string | null {
  const p = String(path || '').replace(/\\/g, '/')
  const i = p.lastIndexOf('/')
  return i > 0 ? p.slice(0, i) : null
}

/**
 * 曲目展示封面：曲目封面 → 所在文件夹外部封面。
 * miss 时自动发起异步加载，命中回调 onChange。
 */
export function resolveTrackCover(t: { id: string; path: string }, onChange?: (url: string | null) => void): string | null {
  const hit = cachedTrackCover([t.id])
  if (hit) return hit
  const dir = trackDirPath(t.path)
  const folderHit = cachedFolderCover(dir)
  if (folderHit) return folderHit
  if (onChange) {
    requestTrackCovers([t.id], (url) => { if (url) onChange(url) })
    if (dir) requestFolderCovers([dir], (url) => { if (url) onChange(url) })
  }
  return null
}

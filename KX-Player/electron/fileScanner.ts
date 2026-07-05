import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { Worker } from 'node:worker_threads'
import chokidar from 'chokidar'
import * as musicMetadata from 'music-metadata'
import { getCoversDir } from './coverService'

const AUDIO_EXTS = new Set(['.mp3', '.flac', '.wav', '.ogg', '.m4a', '.aac', '.wma', '.opus', '.ape', '.wv', '.aiff', '.alac', '.dsf', '.dff', '.dsd'])
const VIDEO_EXTS = new Set(['.mp4', '.mkv', '.avi', '.mov', '.webm', '.flv', '.wmv'])
const ALL_EXTS = new Set([...AUDIO_EXTS, ...VIDEO_EXTS])

// On Windows, prepend \\?\ for paths > 240 chars to bypass MAX_PATH (260 char) limit.
function longPath(p: string): string {
  if (process.platform !== 'win32') return p
  // Only add prefix for absolute paths that might exceed the limit
  if (p.length > 240 && !p.startsWith('\\\\?\\')) {
    return '\\\\?\\' + p.replace(/\//g, '\\')
  }
  return p
}

// Robust recursive file discovery that handles Windows long paths.
async function discoverFiles(folderPaths: string[]): Promise<string[]> {
  const results: string[] = []
  const visited = new Set<string>()
  async function walk(dirPath: string): Promise<void> {
    const lp = longPath(dirPath)
    let entries: fs.Dirent[] = []
    try { entries = await fs.promises.readdir(lp, { withFileTypes: true }) } catch { return }
    for (const entry of entries) {
      const fullPath = path.join(dirPath, entry.name)
      if (entry.isDirectory()) {
        if (!visited.has(fullPath) && visited.size < 10000) {
          visited.add(fullPath)
          await walk(fullPath)
        }
      } else if (entry.isFile() || entry.isSymbolicLink()) {
        const ext = path.extname(entry.name).toLowerCase()
        if (ALL_EXTS.has(ext)) {
          results.push(fullPath)
        }
      }
    }
  }
  for (const fp of folderPaths) {
    visited.add(fp)
    await walk(fp)
  }
  return results
}

const SCAN_TIMEOUT_MS = 15000 // 15s per file (up from 5s — some files need longer parsing)
const LARGE_FILE_SCAN_TIMEOUT_MS = 30000 // 30 seconds for large files
const CHOKIDAR_DELAY = 1000
const YIELD_INTERVAL = 500 // yield to event loop every N iterations to keep main process responsive

const COVER_FILE_EXTS = new Set(['.jpg', '.jpeg', '.png', '.webp', '.bmp', '.gif'])
const MAX_COVER_BYTES = 15 * 1024 * 1024
const COVER_NAME_HINTS = ['cover', 'folder', 'front', 'albumart', 'album', 'art', 'jacket', 'ジャケット', '封面', '专辑封面', '专辑图']
const NON_COVER_HINTS = ['ui', '说明', 'screenshot', 'screen', 'manual', 'readme', 'player', 'capture', 'shot', 'ss', 'banner', 'icon', 'thumb', 'thumbnail', 'small', 'icon']

function normalizeImageMime(format: string): string {
  let f = format.toLowerCase().trim()
  if (f === 'jpg') f = 'jpeg'
  if (!f.startsWith('image/')) f = `image/${f}`
  return f
}

function coverToBase64(filePath: string): string | null {
  try {
    const data = fs.readFileSync(filePath)
    if (!data.length || data.length > MAX_COVER_BYTES) return null
    const ext = path.extname(filePath).slice(1).toLowerCase()
    return `data:image/${ext === 'jpg' ? 'jpeg' : ext};base64,${data.toString('base64')}`
  } catch {
    return null
  }
}

function getImageDimensions(filePath: string): { width: number; height: number } | null {
  try {
    const fd = fs.openSync(filePath, 'r')
    try {
      const head = Buffer.alloc(32)
      fs.readSync(fd, head, 0, 32, 0)
      const ext = path.extname(filePath).toLowerCase()
      if (ext === '.png') {
        if (head.toString('ascii', 1, 4) === 'PNG') {
          return { width: head.readUInt32BE(16), height: head.readUInt32BE(20) }
        }
      } else if (ext === '.jpg' || ext === '.jpeg') {
        let i = 2
        while (i < 65536) {
          const buf = Buffer.alloc(16)
          fs.readSync(fd, buf, 0, 16, i)
          if (buf[0] !== 0xFF) { i++; continue }
          const marker = buf[1]
          if (marker === 0xD9 || marker === 0xD8) { i += 2; continue }
          const len = buf.readUInt16BE(2)
          if (marker >= 0xC0 && marker <= 0xCF && marker !== 0xC4 && marker !== 0xC8 && marker !== 0xCC) {
            return { height: buf.readUInt16BE(5), width: buf.readUInt16BE(7) }
          }
          i += 2 + len
          if (len < 2) break
        }
      } else if (ext === '.webp') {
        if (head.toString('ascii', 0, 4) === 'RIFF' && head.toString('ascii', 8, 12) === 'WEBP') {
          const chunk = head.toString('ascii', 12, 16)
          if (chunk === 'VP8 ') {
            return { width: head.readUInt16LE(26) & 0x3fff, height: head.readUInt16LE(28) & 0x3fff }
          } else if (chunk === 'VP8L') {
            const bits = head.readUInt32LE(21)
            return { width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 }
          } else if (chunk === 'VP8X') {
            return { width: head.readUInt24BE(24) + 1, height: head.readUInt24BE(27) + 1 }
          }
        }
      } else if (ext === '.bmp') {
        return { width: head.readUInt32LE(18), height: Math.abs(head.readInt32LE(22)) }
      } else if (ext === '.gif') {
        return { width: head.readUInt16LE(6), height: head.readUInt16LE(8) }
      }
    } finally {
      fs.closeSync(fd)
    }
  } catch { /* ignore */ }
  return null
}

function scoreCoverCandidate(filePath: string, depth: number): number {
  const name = path.basename(filePath).toLowerCase()
  const ext = path.extname(filePath).toLowerCase()
  const stat = fs.statSync(filePath)
  const sizeKB = stat.size / 1024

  // Strongly exclude known non-cover images
  for (const hint of NON_COVER_HINTS) {
    if (name.includes(hint)) return -1000
  }

  let score = 0

  // Prefer standard cover file names
  for (const hint of COVER_NAME_HINTS) {
    if (name.includes(hint)) score += 100
  }

  // Prefer common image formats
  if (ext === '.jpg' || ext === '.jpeg') score += 10
  if (ext === '.png') score += 5

  // Prefer files in the same directory (less depth)
  score -= depth * 30

  // Prefer typical cover dimensions (DLsite cover is 560x420 ~ 4:3)
  const dims = getImageDimensions(filePath)
  if (dims) {
    const ratio = dims.width / dims.height
    // DLsite / DLsite-like cover ratio ~ 1.33
    if (ratio >= 1.2 && ratio <= 1.5) score += 60
    // Square-ish covers
    else if (ratio >= 0.9 && ratio <= 1.1) score += 40
    // Penalize very wide or very tall images (screenshots, banners)
    else if (ratio > 2.5 || ratio < 0.4) score -= 50

    // Prefer moderate dimensions (covers are usually 400-1200 px on the long side)
    const longSide = Math.max(dims.width, dims.height)
    if (longSide >= 400 && longSide <= 1200) score += 20
    else if (longSide > 1600) score -= 20
  } else {
    // Fallback size heuristic
    if (sizeKB >= 30 && sizeKB <= 600) score += 10
    else if (sizeKB > 1000) score -= 10
  }

  return score
}

function findExternalCover(dirPath: string, maxDepth = 1): string | null {
  let bestPath: string | null = null
  let bestScore = -Infinity

  function scan(currentDir: string, depth: number) {
    if (depth > maxDepth) return
    let entries: fs.Dirent[] = []
    try { entries = fs.readdirSync(currentDir, { withFileTypes: true }) } catch { return }
    for (const entry of entries) {
      if (entry.isDirectory()) {
        scan(path.join(currentDir, entry.name), depth + 1)
        continue
      }
      const ext = path.extname(entry.name).toLowerCase()
      if (!COVER_FILE_EXTS.has(ext)) continue
      const filePath = path.join(currentDir, entry.name)
      const score = scoreCoverCandidate(filePath, depth)
      if (score > bestScore) {
        bestScore = score
        bestPath = filePath
      }
    }
  }

  scan(dirPath, 0)
  if (!bestPath) return null
  // If the best score is negative (non-cover match), still use it as a fallback
  if (bestScore < 0) return null
  return coverToBase64(bestPath)
}

// Broader cover search: find any image in the album directory, with fallback for
// directories containing images but no standard cover-named file.
function findAnyImage(dirPath: string): string | null {
  let entries: fs.Dirent[] = []
  try { entries = fs.readdirSync(dirPath, { withFileTypes: true }) } catch { return null }
  let bestPath: string | null = null
  let bestSize = 0
  for (const entry of entries) {
    if (entry.isDirectory()) continue
    const ext = path.extname(entry.name).toLowerCase()
    if (!COVER_FILE_EXTS.has(ext)) continue
    const filePath = path.join(dirPath, entry.name)
    const score = scoreCoverCandidate(filePath, 0)
    if (score > -100 && score > bestSize) { // not excluded, prefer larger scores
      bestSize = score
      bestPath = filePath
    }
  }
  if (!bestPath) return null
  return coverToBase64(bestPath)
}

interface ScannedTrack {
  id: string
  name: string
  path: string
  duration: number
  artist: string
  album: string
  format: string
  isVideo: boolean
  coverPath: string | null
  coverData: string | null
  lyricsPath: string | null
  fileMtime: number
  fileSize: number
  metaTitle: string | null
  metaArtist: string | null
  genre: string | null
  bitrate: number | null
  sampleRate: number | null
  albumCoverData?: string | null
}

interface ScannedAlbum {
  name: string
  artist: string
  dirPath: string | null
  coverPath: string | null
  coverData: string | null
  tracks: ScannedTrack[]
}

interface ScannedArtist {
  name: string
  path: string
  albums: ScannedAlbum[]
}

interface FolderNode {
  name: string
  path: string
  children: FolderNode[]
  tracks: ScannedTrack[]
  trackCount: number
  coverData: string | null
}

let watchers: chokidar.FSWatcher[] = []
let onChangeCallback: (() => void) | null = null

function hashPath(p: string): string {
  return crypto.createHash('md5').update(p).digest('hex').slice(0, 12)
}

// Yield to the event loop periodically so the main process can handle other IPC messages
// and send progress updates during long synchronous scanning loops.
function yieldToEventLoop(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve))
}

function normalizeName(filename: string): string {
  let name = path.basename(filename, path.extname(filename))
  name = name.replace(/^[\d]+[\s.\-_]+/, '').replace(/[_\-]/g, ' ').trim()
  return name || path.basename(filename)
}

function throttleProgress(callback: (completed: number, total: number) => void, total: number) {
  let lastReported = -1
  let timer: NodeJS.Timeout | null = null
  return (completed: number) => {
    if (completed === total || completed - lastReported >= Math.max(1, Math.floor(total * 0.02)) || lastReported < 0) {
      if (timer) { clearTimeout(timer); timer = null }
      lastReported = completed
      callback(completed, total)
    } else if (!timer) {
      timer = setTimeout(() => {
        timer = null
        lastReported = completed
        callback(completed, total)
      }, 120)
    }
  }
}

// Worker pool for metadata parsing. Workers are terminated and recreated
// before each scan to avoid stale/dead workers from previous runs.
let _workerPool: Worker[] = []
let _workerPath: string | null = null

function _getWorkerPath(): string | null {
  if (_workerPath) return _workerPath
  const candidates = [
    path.join(__dirname, 'workers', 'metadata-worker.js'),
    path.join(__dirname, '..', 'dist-electron', 'workers', 'metadata-worker.js'),
    path.join(process.cwd(), 'dist-electron', 'workers', 'metadata-worker.js'),
  ]
  for (const p of candidates) {
    if (fs.existsSync(p)) { _workerPath = p; return p }
  }
  return null
}

function _terminateWorkerPool() {
  if (_workerPool.length === 0) return
  for (const w of _workerPool) {
    try { w.terminate() } catch { /* ignore */ }
  }
  _workerPool = []
}

/**
 * LPT (Longest Processing Time first) file distribution.
 * Sorts files by size descending, then assigns each file to the worker
 * with the smallest total size. This ensures balanced load across workers
 * since larger files take proportionally longer to parse.
 * 
 * For large file counts (>5000), falls back to round-robin to avoid
 * the O(n log n) sort + O(n) stat overhead.
 */
function distributeFilesBySize(files: string[], workerCount: number): string[][] {
  if (files.length === 0) return []
  const effectiveWorkers = Math.min(workerCount, files.length)
  const chunks: string[][] = Array.from({ length: effectiveWorkers }, () => [])

  // For large file counts, use round-robin to avoid stat overhead
  if (files.length > 5000) {
    for (let i = 0; i < files.length; i++) {
      chunks[i % effectiveWorkers].push(files[i])
    }
    return chunks.filter(c => c.length > 0)
  }

  const sizes = new Float64Array(effectiveWorkers) // total size per worker

  // Get file sizes and sort descending (largest first)
  const fileSizes: Array<{ path: string; size: number }> = []
  for (const f of files) {
    try { fileSizes.push({ path: f, size: fs.statSync(f).size }) } catch { fileSizes.push({ path: f, size: 0 }) }
  }
  fileSizes.sort((a, b) => b.size - a.size)

  // Assign each file to the worker with the least total size
  for (const { path: filePath, size } of fileSizes) {
    let minIdx = 0
    for (let i = 1; i < effectiveWorkers; i++) {
      if (sizes[i] < sizes[minIdx]) minIdx = i
    }
    chunks[minIdx].push(filePath)
    sizes[minIdx] += size
  }

  // Remove empty chunks (if fewer files than workers)
  return chunks.filter(c => c.length > 0)
}

async function enrichWithWorkers(
  files: string[],
  existingMeta: Map<string, { duration: number; hasCover: boolean; title: string | null; artist: string | null; fileMtime: number; fileSize: number; genre: string | null; bitrate: number | null; sampleRate: number | null }> = new Map(),
  onProgress?: (completed: number, total: number) => void
): Promise<Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>> {
  const results = new Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>()
  const normalFiles: string[] = []
  const total = files.length
  let completed = 0
  const reportProgress = onProgress ? throttleProgress(onProgress, total) : () => {}

  console.time('[scan] cacheCheck')
  for (const [i, filePath] of files.entries()) {
    if (i > 0 && i % YIELD_INTERVAL === 0) await yieldToEventLoop()
    const stat = getFileStat(filePath)
    const cached = existingMeta.get(filePath.replace(/\\/g, '/'))
    if (stat && cached && cached.fileMtime === stat.mtime && cached.fileSize === stat.size) {
      results.set(filePath, {
        duration: cached.duration,
        coverData: null, // Cover preserved by saveLibrarySnapshot's DB fallback
        title: cached.title,
        artist: cached.artist,
        genre: cached.genre,
        bitrate: cached.bitrate,
        sampleRate: cached.sampleRate,
      })
      completed += 1
      reportProgress(completed)
    } else {
      normalFiles.push(filePath)
    }
  }
  console.timeEnd('[scan] cacheCheck')

  if (normalFiles.length === 0) {
    reportProgress(total)
    return results
  }

  // Audio metadata parsing is I/O-bound (disk reads) + CPU-bound (tag parsing).
  // Over-subscribing workers beyond core count helps keep CPU busy during I/O waits.
  // Formula: cores * 1.5, clamped to [2, 16] for reasonable resource usage.
  const physicalCores = Math.max(1, os.cpus().length - 1)
  const cpuCount = Math.min(16, Math.max(2, Math.ceil(physicalCores * 1.5)))
  // LPT (Longest Processing Time first) algorithm: distribute files by size
  // so each worker gets roughly equal total file size for balanced load
  const chunks = distributeFilesBySize(normalFiles, cpuCount)

  // Use larger timeout if any chunk contains large files (>1GB)
  const LARGE_FILE_SIZE = 100 * 1024 * 1024 // 100MB threshold for extended timeout
  function hasLargeFiles(chunk: string[]): boolean {
    return chunk.some(f => {
      try { return fs.statSync(f).size > LARGE_FILE_SIZE } catch { return false }
    })
  }

  const workerPath = _getWorkerPath()
  if (!workerPath) {
    for (const f of normalFiles) {
      results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
    }
    completed += normalFiles.length
    reportProgress(total)
    return results
  }

  // Always terminate stale workers and create fresh ones for each scan.
  _terminateWorkerPool()

  console.time('[scan] workerCreate')
  for (let i = 0; i < chunks.length; i++) {
    try {
      const perFileTimeout = hasLargeFiles(chunks[i]) ? LARGE_FILE_SCAN_TIMEOUT_MS : SCAN_TIMEOUT_MS
      const w = new Worker(workerPath, { workerData: { files: chunks[i], timeoutMs: perFileTimeout } })
      _workerPool.push(w)
    } catch (e) {
      console.error('[scan] worker creation failed:', e)
      break
    }
  }
  console.timeEnd('[scan] workerCreate')

  console.time('[scan] workerBatch')
  const workerPromises = chunks.map((chunk, idx) => {
    return new Promise<void>((resolve) => {
      const worker = _workerPool[idx]
      if (!worker) {
        for (const f of chunk) {
          results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
        }
        completed += chunk.length
        reportProgress(completed)
        resolve()
        return
      }

      // Scale timeout per file count: each file gets SCAN_TIMEOUT_MS (5s), min 10s, max 10min
      const chunkTimeout = Math.max(10000, Math.min(600000, chunk.length * SCAN_TIMEOUT_MS))
      let hasResponded = false
      let chunkTimer: NodeJS.Timeout | null = null
      const clearChunkTimer = () => { if (chunkTimer) { clearTimeout(chunkTimer); chunkTimer = null } }

      const messageHandler = (msg: any) => {
        if (msg.type === 'progress') {
          // Reset timer on progress to keep long-running chunks alive
          clearChunkTimer()
          chunkTimer = setTimeout(handleTimeout, chunkTimeout)
          return
        }
        if (hasResponded) return
        hasResponded = true
        clearChunkTimer()
        worker.removeListener('message', messageHandler)
        worker.removeListener('error', errorHandler)
        if (msg.type === 'result') {
          for (const r of msg.results) {
            results.set(r.path, {
              duration: r.duration || 0,
              coverData: r.coverB64 || null,
              title: r.title || null,
              artist: r.artist || null,
              genre: r.genre || null,
              bitrate: r.bitrate || null,
              sampleRate: r.sampleRate || null,
            })
          }
          completed += chunk.length
          reportProgress(completed)
          resolve()
        } else if (msg.type === 'error') {
          console.error('[scan] worker reported error:', msg.message)
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
          }
          completed += chunk.length
          reportProgress(completed)
          resolve()
        }
      }
      worker.on('message', messageHandler)

      const errorHandler = (err: Error) => {
        console.error('[scan] worker error:', err?.message || err)
        clearChunkTimer()
        worker.removeListener('message', messageHandler)
        worker.removeListener('error', errorHandler)
        if (!hasResponded) {
          hasResponded = true
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
          }
          completed += chunk.length
          reportProgress(completed)
        }
        resolve()
      }
      worker.on('error', errorHandler)

      const handleTimeout = () => {
        if (!hasResponded) {
          console.warn('[scan] worker timeout for chunk', idx, 'size', chunk.length)
          hasResponded = true
          worker.removeListener('message', messageHandler)
          worker.removeListener('error', errorHandler)
          try { worker.terminate() } catch { /* ignore */ }
          _workerPool = _workerPool.filter(w => w !== worker)
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
          }
          completed += chunk.length
          reportProgress(completed)
          resolve()
        }
      }
      chunkTimer = setTimeout(handleTimeout, chunkTimeout)

      // Workers are created fresh with workerData for each scan.
      // No need to send postMessage — workerData is available at worker startup.
      // The worker will process it and respond via parentPort.postMessage.
      })
  })

  await Promise.all(workerPromises)
  console.timeEnd('[scan] workerBatch')
  reportProgress(total)
  _terminateWorkerPool() // Clean up workers after scan
  return results
}

function getFileStat(filePath: string): { mtime: number; size: number } | null {
  try {
    const st = fs.statSync(filePath)
    return { mtime: st.mtimeMs, size: st.size }
  } catch {
    return null
  }
}

async function groupTracksByFolder(
  files: string[],
  metaResults: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>,
  rootPaths: string[]
): Promise<ScannedArtist[]> {
  const artistMap = new Map<string, { path: string; albums: Map<string, ScannedAlbum> }>()

  // Pre-normalize root paths to avoid repeated replace in the inner loop
  const normalizedRoots = rootPaths.map(rp => rp.replace(/\\/g, '/').replace(/\/+$/, ''))

  for (const [fi, fp] of files.entries()) {
    if (fi > 0 && fi % YIELD_INTERVAL === 0) await yieldToEventLoop()
    const meta = metaResults.get(fp)
    if (!meta) continue
    const st = getFileStat(fp)
    if (!st) continue

    const nfp = fp.replace(/\\/g, '/')
    let matchedRoot: string | null = null

    for (let ri = 0; ri < normalizedRoots.length; ri++) {
      const nrp = normalizedRoots[ri]
      if (nfp === nrp || nfp.startsWith(nrp + '/')) {
        matchedRoot = nrp
        break
      }
    }

    if (!matchedRoot) continue

    const rel = path.relative(matchedRoot, fp)
    const parts = rel.split(path.sep)
    let artistName = path.basename(matchedRoot)
    let albumName: string

    if (parts.length >= 2) {
      albumName = parts[0]
    } else {
      albumName = artistName
    }

    if (meta.artist && meta.artist.trim()) {
      artistName = meta.artist.trim()
    }

    if (!artistMap.has(artistName)) {
      artistMap.set(artistName, { path: matchedRoot, albums: new Map() })
    }

    const artist = artistMap.get(artistName)!
    if (!artist.albums.has(albumName)) {
      const albumDirPath = parts.length >= 1
        ? path.join(matchedRoot, parts[0])
        : matchedRoot
      artist.albums.set(albumName, {
        name: albumName,
        artist: artistName,
        dirPath: albumDirPath,
        coverPath: null,
        coverData: null,
        tracks: [],
      })
    }

    const album = artist.albums.get(albumName)!
    const trackExt = path.extname(fp).toLowerCase()
    album.tracks.push({
      id: hashPath(fp),
      name: (meta.title && meta.title.trim()) ? meta.title.trim() : normalizeName(fp),
      path: nfp,
      duration: meta.duration,
      artist: meta.artist && meta.artist.trim() ? meta.artist.trim() : '佚名',
      album: albumName,
      format: trackExt.replace('.', ''),
      isVideo: VIDEO_EXTS.has(trackExt),
      coverPath: null,
      coverData: meta.coverData,
      lyricsPath: null,
      fileMtime: st.mtime,
      fileSize: st.size,
      metaTitle: meta.title,
      metaArtist: meta.artist,
      genre: meta.genre || null,
      bitrate: meta.bitrate || null,
      sampleRate: meta.sampleRate || null,
    })
  }

  for (const [, artist] of artistMap) {
    for (const [, album] of artist.albums) {
      // Prefer embedded cover from any track in the album
      for (const track of album.tracks) {
        const metaResult = metaResults.get(track.path)
        if (metaResult?.coverData) {
          album.coverData = metaResult.coverData
          break
        }
      }
      // Fallback to external cover files in album directory
      if (!album.coverData && album.dirPath) {
        album.coverData = findExternalCover(album.dirPath)
      }
    }
  }

  return [...artistMap.entries()].map(([name, a]) => ({
    name,
    path: a.path,
    albums: [...a.albums.values()],
  }))
}

async function buildFolderTree(
  files: string[],
  metaResults: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>,
  rootPaths: string[]
): Promise<FolderNode[]> {
  const nodeMap = new Map<string, FolderNode>()
  const cleanRoots = rootPaths.map(rp => rp.replace(/\\/g, '/').replace(/\/+$/, ''))
  const roots: FolderNode[] = []

  function getOrCreateNode(dirPath: string, dirName: string): FolderNode {
    if (nodeMap.has(dirPath)) return nodeMap.get(dirPath)!
    const node: FolderNode = { name: dirName, path: dirPath, children: [], tracks: [], trackCount: 0, coverData: null }
    nodeMap.set(dirPath, node)
    return node
  }

  for (const [fi, fp] of files.entries()) {
    if (fi > 0 && fi % YIELD_INTERVAL === 0) await yieldToEventLoop()
    const meta = metaResults.get(fp)
    if (!meta) continue
    const st = getFileStat(fp)
    if (!st) continue

    const nfp = fp.replace(/\\/g, '/')
    let matchedRoot: string | null = null

    for (let ri = 0; ri < cleanRoots.length; ri++) {
      if (nfp === cleanRoots[ri] || nfp.startsWith(cleanRoots[ri] + '/')) {
        matchedRoot = cleanRoots[ri]
        break
      }
    }
    if (!matchedRoot) continue

    const dir = path.dirname(fp).replace(/\\/g, '/')
    const dirName = path.basename(dir)
    const parentDir = path.dirname(dir).replace(/\\/g, '/')

    const node = getOrCreateNode(dir, dirName)
    const trackExt = path.extname(fp).toLowerCase()

    node.tracks.push({
      id: hashPath(fp),
      name: (meta.title && meta.title.trim()) ? meta.title.trim() : normalizeName(fp),
      path: nfp,
      duration: meta.duration,
      artist: meta.artist && meta.artist.trim() ? meta.artist.trim() : '佚名',
      album: dirName,
      format: trackExt.replace('.', ''),
      isVideo: VIDEO_EXTS.has(trackExt),
      coverPath: null,
      coverData: meta.coverData,
      lyricsPath: null,
      fileMtime: st.mtime,
      fileSize: st.size,
      metaTitle: meta.title,
      metaArtist: meta.artist,
      genre: meta.genre || null,
      bitrate: meta.bitrate || null,
      sampleRate: meta.sampleRate || null,
    })

    let isRoot = true
    for (let ri = 0; ri < cleanRoots.length; ri++) {
      if (parentDir === cleanRoots[ri] || parentDir.startsWith(cleanRoots[ri] + '/')) {
        isRoot = false
        const pName = path.basename(parentDir)
        const parentNode = getOrCreateNode(parentDir, pName)
        if (!parentNode.children.some(c => c.path === dir)) {
          parentNode.children.push(node)
        }
        break
      }
    }
    if (isRoot) {
      if (!roots.some(r => r.path === dir)) {
        roots.push(node)
      }
    }
  }

  // Find cover for a folder: external cover files first, then embedded track covers, then children.
  function findFolderCover(rootNode: FolderNode): string | null {
    const external = findExternalCover(rootNode.path.replace(/\//g, path.sep))
    if (external) return external
    for (const t of rootNode.tracks) {
      const meta = metaResults.get(t.path)
      if (meta?.coverData) return meta.coverData
    }
    for (const c of rootNode.children) {
      const r = findFolderCover(c)
      if (r) return r
    }
    return null
  }
  function computeNodeStats(node: FolderNode): void {
    node.children.sort((a, b) => a.name.localeCompare(b.name))
    node.tracks.sort((a, b) => a.name.localeCompare(b.name))
    let trackCount = node.tracks.length
    for (const child of node.children) {
      computeNodeStats(child)
      trackCount += child.trackCount
    }
    node.trackCount = trackCount
    node.coverData = findFolderCover(node)
  }

  for (const [, node] of nodeMap) {
    node.children.sort((a, b) => a.name.localeCompare(b.name))
    node.tracks.sort((a, b) => a.name.localeCompare(b.name))
    node.trackCount = node.tracks.length
  }

  roots.sort((a, b) => a.name.localeCompare(b.name))

  // Ensure all imported root folders are in roots.
  // Parent nodes created during child processing are never added to roots.
  for (let ri = 0; ri < cleanRoots.length; ri++) {
    const nrp = cleanRoots[ri]
    let hasContent = false
    for (const [, node] of nodeMap) {
      const nn = node.path.replace(/\\/g, '/')
      if (nn === nrp || nn.startsWith(nrp + '/')) { hasContent = true; break }
    }
    if (!hasContent) continue
    const rn = getOrCreateNode(nrp, path.basename(nrp))
    // Promote any existing root that is under this imported folder to a child
    for (let i = roots.length - 1; i >= 0; i--) {
      const r = roots[i]
      const nr = r.path.replace(/\\/g, '/')
      if (nr.startsWith(nrp + '/')) {
        const nr2 = r.path.replace(/\\/g, '/')
        if (!rn.children.some(c => c.path.replace(/\\/g, '/') === nr2)) rn.children.push(r)
        roots.splice(i, 1)
      }
    }
    const nrn = rn.path.replace(/\\/g, '/')
    if (!roots.some(r => r.path.replace(/\\/g, '/') === nrn)) roots.push(rn)
  }

  // Assign covers to roots as well
  for (const r of roots) computeNodeStats(r)
  return roots
}

// For albums that still have no cover after scanning, find covers using a
// three-tier strategy ordered by speed:
//   1. Filesystem cache check — O(1) per track, reuses covers from previous scans
//   2. External cover files  — reads small image files from disk
//   3. Embedded extraction   — last resort, parses audio metadata (slowest)
const COVER_EXTRACT_TIMEOUT = 3000 // 3s per album cover extraction
async function fillAlbumCovers(artists: ScannedArtist[]): Promise<void> {
  // Build set of track IDs that already have cover files on disk (fast, one readdir)
  let existingCovers = new Set<string>()
  try {
    const coversDir = getCoversDir()
    if (fs.existsSync(coversDir)) {
      const files = fs.readdirSync(coversDir)
      for (const f of files) {
        if (f.startsWith('folder_') || !f.endsWith('.jpg')) continue
        existingCovers.add(f.slice(0, -4)) // strip .jpg → trackId
      }
    }
  } catch { /* ignore */ }

  for (const artist of artists) {
    for (const album of artist.albums) {
      if (album.coverData) continue

      // --- Tier 1: Filesystem cache (fastest — no audio parsing) ---
      // Check if any track in this album already has a saved cover file
      let foundCached = false
      for (const track of album.tracks) {
        const trackId = hashPath(track.path)
        if (existingCovers.has(trackId)) {
          try {
            const coversDir = getCoversDir()
            const coverPath = path.join(coversDir, `${trackId}.jpg`)
            const buffer = fs.readFileSync(coverPath)
            if (buffer.length > 0) {
              album.coverData = `data:image/jpeg;base64,${buffer.toString('base64')}`
              track.coverData = album.coverData
              foundCached = true
              break
            }
          } catch { /* skip */ }
        }
      }
      if (foundCached) continue

      // --- Tier 2: External cover files (fast — reads small image) ---
      if (album.dirPath) {
        const externalCover = findExternalCover(album.dirPath, 1)
        if (externalCover) {
          album.coverData = externalCover
          continue
        }
        const anyImage = findAnyImage(album.dirPath)
        if (anyImage) {
          album.coverData = anyImage
          continue
        }
      }

      // --- Tier 3: Embedded cover extraction (slowest — parses audio file) ---
      const targetTrack = album.tracks[0]
      if (targetTrack) {
        try {
          const meta = await Promise.race([
            musicMetadata.parseFile(targetTrack.path, {
              duration: false,
              skipCovers: false,
            }),
            new Promise<never>((_, reject) => setTimeout(() => reject(new Error('timeout')), COVER_EXTRACT_TIMEOUT))
          ])
          if (meta.common.picture && meta.common.picture.length > 0) {
            const pic = meta.common.picture[0]
            const data = Buffer.isBuffer(pic.data) ? pic.data : Buffer.from(pic.data)
            if (data.length > 0 && data.length <= 15 * 1024 * 1024) {
              let fmt = pic.format || 'image/jpeg'
              if (!fmt.startsWith('image/')) fmt = `image/${fmt}`
              const b64 = `data:${fmt};base64,${data.toString('base64')}`
              album.coverData = b64
              targetTrack.coverData = b64
            }
          }
        } catch { /* embedded extraction failed */ }
      }
    }
  }
}

export async function scanFoldersWithProgress(
  folderPaths: string[],
  existingMeta: Map<string, { duration: number; hasCover: boolean; title: string | null; artist: string | null; fileMtime: number; fileSize: number; genre: string | null; bitrate: number | null; sampleRate: number | null }> = new Map(),
  onProgress?: (completed: number, total: number) => void,
  onStage?: (stage: string) => void
): Promise<{ artists: ScannedArtist[]; folderTree: FolderNode[]; allTracks: ScannedTrack[]; fileCount: number }> {
  console.time('[scan] total')
  onStage?.('发现文件...')
  console.time('[scan] discoverFiles')
  const files = await discoverFiles(folderPaths)
  console.timeEnd('[scan] discoverFiles')
  const totalFiles = files.length
  onProgress?.(0, totalFiles)
  onStage?.(`解析元数据... (${totalFiles} 个文件)`)

  console.time('[scan] enrichWithWorkers')
  const metaResults = await enrichWithWorkers(files, existingMeta, onProgress)
  console.timeEnd('[scan] enrichWithWorkers')

  onStage?.('整理结构...')
  console.time('[scan] groupTracksByFolder')
  const artists = await groupTracksByFolder(files, metaResults, folderPaths)
  console.timeEnd('[scan] groupTracksByFolder')
  console.time('[scan] buildFolderTree')
  const folderTree = await buildFolderTree(files, metaResults, folderPaths)
  console.timeEnd('[scan] buildFolderTree')

  // Extract covers for albums that have none (from external or embedded sources)
  console.time('[scan] fillAlbumCovers')
  await fillAlbumCovers(artists)
  console.timeEnd('[scan] fillAlbumCovers')

  const allTracks: ScannedTrack[] = []
  let ti = 0
  console.time('[scan] assembleAllTracks')
  for (const artist of artists) {
    for (const album of artist.albums) {
      for (const track of album.tracks) {
        track.albumCoverData = album.coverData || track.coverData || null
        allTracks.push(track)
        ti++
        if (ti % YIELD_INTERVAL === 0) await yieldToEventLoop()
      }
    }
  }
  console.timeEnd('[scan] assembleAllTracks')

  // Propagate covers from allTracks back to folder tree nodes
  // This is needed because fillAlbumCovers runs after buildFolderTree,
  // so folder nodes don't get covers from album/track data on cache re-scan
  console.time('[scan] propagateCoversToFolderTree')
  const pathToCover = new Map<string, string>()
  for (const t of allTracks) {
    const cover = t.coverData || t.albumCoverData
    if (cover) {
      const trackDir = path.dirname(t.path).replace(/\\/g, '/')
      if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover)
    }
  }
  function propagateCoversToNode(node: FolderNode): void {
    // Check if this node has a cover from track data
    const coverFromTrack = pathToCover.get(node.path.replace(/\\/g, '/'))
    if (coverFromTrack && !node.coverData) {
      node.coverData = coverFromTrack
    }
    // Recurse to children
    for (const child of node.children) {
      propagateCoversToNode(child)
      // If this node still has no cover, try to inherit from children
      if (!node.coverData && child.coverData) {
        node.coverData = child.coverData
      }
    }
  }
  for (const rootNode of folderTree) {
    propagateCoversToNode(rootNode)
  }
  console.timeEnd('[scan] propagateCoversToFolderTree')

  // Verify cover coverage
  const tracksWithCover = allTracks.filter(t => t.coverData || t.albumCoverData).length
  const albumsWithCover = artists.reduce((a, ar) => a + ar.albums.filter(al => al.coverData).length, 0)
  const totalAlbums = artists.reduce((a, ar) => a + ar.albums.length, 0)
  console.log(`[scan] cover coverage: ${tracksWithCover}/${allTracks.length} tracks, ${albumsWithCover}/${totalAlbums} albums`)

  console.timeEnd('[scan] total')
  return { artists, folderTree, allTracks, fileCount: totalFiles }
}

/**
 * Incremental scan: only parse new/changed files via workers.
 * Uses existing metadata from DB for unchanged files, making it much faster
 * than a full scan when only a few folders are added or changed.
 */
export async function scanFoldersIncremental(
  allFolderPaths: string[],
  existingMeta: Map<string, {
    duration: number; coverData: string | null; title: string | null; artist: string | null;
    genre: string | null; bitrate: number | null; sampleRate: number | null;
    fileMtime: number; fileSize: number
  }>,
  onProgress?: (completed: number, total: number) => void,
  onStage?: (stage: string) => void
): Promise<{ artists: ScannedArtist[]; folderTree: FolderNode[]; allTracks: ScannedTrack[]; fileCount: number; changedPaths: Set<string> }> {
  console.time('[scan-incr] total')

  // Load cover data from filesystem for existing cached files
  // Build reverse map: trackId → filePath for O(1) lookup instead of O(n*m)
  console.time('[scan-incr] loadCovers')
  try {
    const coversDir = getCoversDir()
    if (fs.existsSync(coversDir)) {
      const idToPath = new Map<string, string>()
      for (const [filePath] of existingMeta) {
        idToPath.set(hashPath(filePath), filePath)
      }
      const coverFiles = fs.readdirSync(coversDir)
      for (const cf of coverFiles) {
        if (cf.startsWith('folder_') || !cf.endsWith('.jpg')) continue
        const trackId = cf.slice(0, -4)
        const filePath = idToPath.get(trackId)
        if (filePath) {
          const meta = existingMeta.get(filePath)
          if (meta) {
            try {
              const buffer = fs.readFileSync(path.join(coversDir, cf))
              if (buffer.length > 0) {
                meta.coverData = `data:image/jpeg;base64,${buffer.toString('base64')}`
              }
            } catch { /* skip */ }
          }
        }
      }
    }
  } catch { /* ignore */ }
  console.timeEnd('[scan-incr] loadCovers')

  // Discover files in ALL folders (to get complete file list)
  console.time('[scan-incr] discoverFiles')
  const files = await discoverFiles(allFolderPaths)
  console.timeEnd('[scan-incr] discoverFiles')
  const totalFiles = files.length

  // Find new/changed files
  const changedFiles: string[] = []
  for (const filePath of files) {
    const normalizedPath = filePath.replace(/\\/g, '/')
    const cached = existingMeta.get(normalizedPath)
    if (!cached) { changedFiles.push(filePath); continue }
    const stat = getFileStat(filePath)
    if (!stat || cached.fileMtime !== stat.mtime || cached.fileSize !== stat.size) {
      changedFiles.push(filePath)
    }
  }

  console.log(`[scan-incr] ${changedFiles.length} new/changed out of ${totalFiles} files`)

  // Build metaResults from existing data
  const metaResults = new Map<string, {
    duration: number; coverData: string | null; title: string | null; artist: string | null;
    genre: string | null; bitrate: number | null; sampleRate: number | null
  }>()
  for (const filePath of files) {
    const normalizedPath = filePath.replace(/\\/g, '/')
    const cached = existingMeta.get(normalizedPath)
    if (cached) {
      metaResults.set(filePath, {
        duration: cached.duration,
        coverData: cached.coverData,
        title: cached.title,
        artist: cached.artist,
        genre: cached.genre,
        bitrate: cached.bitrate,
        sampleRate: cached.sampleRate,
      })
    }
  }

  // Process only new/changed files via workers
  if (changedFiles.length > 0) {
    onStage?.('解析新文件元数据...')
    const workerResults = await enrichWithWorkers(changedFiles, existingMeta, (completed, total) => {
      onProgress?.(completed, total)
    })
    for (const [filePath, meta] of workerResults) {
      metaResults.set(filePath, meta)
    }
  } else {
    onProgress?.(0, 0)
  }

  // Build structures (same as full scan)
  onStage?.('构建音乐库...')

  console.time('[scan-incr] groupTracksByFolder')
  const artists = await groupTracksByFolder(files, metaResults, allFolderPaths)
  console.timeEnd('[scan-incr] groupTracksByFolder')

  console.time('[scan-incr] buildFolderTree')
  const folderTree = await buildFolderTree(files, metaResults, allFolderPaths)
  console.timeEnd('[scan-incr] buildFolderTree')

  await fillAlbumCovers(artists)

  console.time('[scan-incr] assembleAllTracks')
  const allTracks: ScannedTrack[] = []
  let ti = 0
  for (const artist of artists) {
    for (const album of artist.albums) {
      for (const track of album.tracks) {
        track.albumCoverData = album.coverData || track.coverData || null
        allTracks.push(track)
        ti++
        if (ti % YIELD_INTERVAL === 0) await yieldToEventLoop()
      }
    }
  }
  console.timeEnd('[scan-incr] assembleAllTracks')

  // Propagate covers to folder tree
  const pathToCover = new Map<string, string>()
  for (const t of allTracks) {
    const cover = t.coverData || t.albumCoverData
    if (cover) {
      const trackDir = path.dirname(t.path).replace(/\\/g, '/')
      if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover)
    }
  }
  function propagateCovers(node: FolderNode): void {
    const c = pathToCover.get(node.path.replace(/\\/g, '/'))
    if (c && !node.coverData) node.coverData = c
    for (const child of node.children) {
      propagateCovers(child)
      if (!node.coverData && child.coverData) node.coverData = child.coverData
    }
  }
  for (const rootNode of folderTree) propagateCovers(rootNode)

  const tracksWithCover = allTracks.filter(t => t.coverData || t.albumCoverData).length
  console.log(`[scan-incr] cover coverage: ${tracksWithCover}/${allTracks.length} tracks`)
  console.timeEnd('[scan-incr] total')

  // Build set of normalized changed paths for incremental cover saving
  const changedPaths = new Set(changedFiles.map(f => f.replace(/\\/g, '/')))

  return { artists, folderTree, allTracks, fileCount: totalFiles, changedPaths }
}

export async function startWatching(
  folderPaths: string[],
  onChange: () => void
): Promise<void> {
  stopWatching()
  onChangeCallback = onChange

  for (const fp of folderPaths) {
    try {
      const watcher = chokidar.watch(fp, {
        ignored: /(^|[\/\\])\../,
        persistent: true,
        ignoreInitial: true,
        depth: 99,
        awaitWriteFinish: { stabilityThreshold: 500, pollInterval: 100 },
      })

      let timer: NodeJS.Timeout | null = null

      const scheduleChange = () => {
        if (!onChangeCallback) return
        if (timer) clearTimeout(timer)
        timer = setTimeout(() => {
          timer = null
          onChangeCallback?.()
        }, CHOKIDAR_DELAY)
      }

      watcher.on('add', scheduleChange)
      watcher.on('change', scheduleChange)
      watcher.on('unlink', scheduleChange)
      watcher.on('addDir', scheduleChange)
      watcher.on('unlinkDir', scheduleChange)

      watchers.push(watcher)
    } catch (e) {
      console.error(`[watcher] Failed to watch ${fp}:`, e)
    }
  }
}

export function stopWatching(): void {
  for (const w of watchers) {
    try { w.close() } catch { }
  }
  watchers = []
  onChangeCallback = null
}

export function terminateWorkerPool(): void {
  _terminateWorkerPool()
}

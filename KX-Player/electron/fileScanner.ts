import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { Worker } from 'node:worker_threads'
import glob from 'fast-glob'
import chokidar from 'chokidar'

const AUDIO_EXTS = new Set(['.mp3', '.flac', '.wav', '.ogg', '.m4a', '.aac', '.wma', '.opus', '.ape', '.wv', '.aiff', '.alac', '.dsf', '.dff', '.dsd'])
const VIDEO_EXTS = new Set(['.mp4', '.mkv', '.avi', '.mov', '.webm', '.flv', '.wmv'])
const ALL_EXTS = new Set([...AUDIO_EXTS, ...VIDEO_EXTS])

const SCAN_TIMEOUT_MS = 30000
const LARGE_FILE_SCAN_TIMEOUT_MS = 120000 // 2 minutes for large files
const CHOKIDAR_DELAY = 1000
const YIELD_INTERVAL = 500 // yield to event loop every N iterations to keep main process responsive

const COVER_FILE_EXTS = new Set(['.jpg', '.jpeg', '.png', '.webp', '.bmp', '.gif'])
const MAX_COVER_BYTES = 15 * 1024 * 1024
const COVER_NAME_HINTS = ['cover', 'folder', 'front', 'albumart', 'album', 'art', 'jacket', 'ジャケット']
const NON_COVER_HINTS = ['ui', '说明', 'screenshot', 'screen', 'manual', 'readme', 'player', 'capture', 'shot', 'ss', 'banner', 'icon']

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

async function discoverFiles(folderPaths: string[]): Promise<string[]> {
  const extPattern = [...ALL_EXTS].map(e => e.replace('.', '')).join(',')
  const patterns = folderPaths.map(fp => {
    const normalized = fp.replace(/\\/g, '/').replace(/\/+$/, '')
    return `${normalized}/**/*.{${extPattern}}`
  })
  return await glob(patterns, {
    onlyFiles: true,
    caseSensitiveMatch: false,
    ignore: ['**/node_modules/**', '**/.git/**'],
    absolute: true,
  })
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

async function enrichWithWorkers(
  files: string[],
  existingMeta: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; fileMtime: number; fileSize: number; genre: string | null; bitrate: number | null; sampleRate: number | null }> = new Map(),
  onProgress?: (completed: number, total: number) => void
): Promise<Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>> {
  const results = new Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; genre: string | null; bitrate: number | null; sampleRate: number | null }>()
  const normalFiles: string[] = []
  const total = files.length
  let completed = 0
  const reportProgress = onProgress ? throttleProgress(onProgress, total) : () => {}

  for (const [i, filePath] of files.entries()) {
    if (i > 0 && i % YIELD_INTERVAL === 0) await yieldToEventLoop()
    const stat = getFileStat(filePath)
    const cached = existingMeta.get(filePath.replace(/\\/g, '/'))
    if (stat && cached && cached.fileMtime === stat.mtime && cached.fileSize === stat.size) {
      results.set(filePath, {
        duration: cached.duration,
        coverData: cached.coverData,
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

  if (normalFiles.length === 0) {
    reportProgress(total)
    return results
  }

  const MAX_WORKERS = 4
  const cpuCount = Math.min(MAX_WORKERS, Math.max(1, os.cpus().length - 1))
  const chunkSize = Math.ceil(normalFiles.length / cpuCount)
  const chunks: string[][] = []
  for (let i = 0; i < normalFiles.length; i += chunkSize) {
    chunks.push(normalFiles.slice(i, i + chunkSize))
  }

  // Use larger timeout if any chunk contains large files (>1GB)
  const LARGE_FILE_SIZE = 1024 * 1024 * 1024
  function hasLargeFiles(chunk: string[]): boolean {
    return chunk.some(f => {
      try { return fs.statSync(f).size > LARGE_FILE_SIZE } catch { return false }
    })
  }

  const workerPromises = chunks.map((chunk) => {
    return new Promise<void>((resolve) => {
      try {
        // Try multiple possible worker paths
        let workerPath = path.join(__dirname, 'workers', 'metadata-worker.js')
        if (!fs.existsSync(workerPath)) {
          workerPath = path.join(__dirname, '..', 'dist-electron', 'workers', 'metadata-worker.js')
        }
        if (!fs.existsSync(workerPath)) {
          workerPath = path.join(process.cwd(), 'dist-electron', 'workers', 'metadata-worker.js')
        }
        if (!fs.existsSync(workerPath)) {
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null })
          }
          completed += chunk.length
          onProgress?.(completed, total)
          resolve()
          return
        }

        const chunkTimeout = hasLargeFiles(chunk) ? LARGE_FILE_SCAN_TIMEOUT_MS : SCAN_TIMEOUT_MS
        const worker = new Worker(workerPath, {
          workerData: { files: chunk, timeoutMs: chunkTimeout },
        })

        let hasResponded = false

        worker.on('message', (msg: any) => {
          hasResponded = true
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
          } else if (msg.type === 'progress') {
            completed += 1
            reportProgress(completed)
          }
        })

        worker.on('error', () => {
          if (!hasResponded) {
            for (const f of chunk) {
              results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
            }
            completed += chunk.length
            reportProgress(completed)
          }
          resolve()
        })

      worker.on('exit', (code) => {
        if (!hasResponded && code !== 0) {
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
          }
          completed += chunk.length
          reportProgress(completed)
        }
        resolve()
      })

      setTimeout(() => {
        if (!hasResponded) {
          worker.terminate()
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
          }
          completed += chunk.length
          reportProgress(completed)
          resolve()
        }
      }, chunkTimeout)
    } catch {
      for (const f of chunk) {
        results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null })
      }
      completed += chunk.length
      reportProgress(completed)
      resolve()
    }
  })
})

  await Promise.all(workerPromises)
  reportProgress(total)
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

  for (const [fi, fp] of files.entries()) {
    if (fi > 0 && fi % YIELD_INTERVAL === 0) await yieldToEventLoop()
    const meta = metaResults.get(fp)
    if (!meta) continue
    const st = getFileStat(fp)
    if (!st) continue

    const nfp = fp.replace(/\\/g, '/')
    let matchedRoot: string | null = null

    for (let ri = 0; ri < rootPaths.length; ri++) {
      const nrp = rootPaths[ri].replace(/\\/g, '/').replace(/\/+$/, '')
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

export async function scanFoldersWithProgress(
  folderPaths: string[],
  existingMeta: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null; fileMtime: number; fileSize: number; genre: string | null; bitrate: number | null; sampleRate: number | null }> = new Map(),
  onProgress?: (completed: number, total: number) => void,
  onStage?: (stage: string) => void
): Promise<{ artists: ScannedArtist[]; folderTree: FolderNode[]; allTracks: ScannedTrack[]; fileCount: number }> {
  onStage?.('发现文件...')
  const files = await discoverFiles(folderPaths)
  const totalFiles = files.length
  onProgress?.(0, totalFiles)
  onStage?.(`解析元数据... (${totalFiles} 个文件)`)

  const metaResults = await enrichWithWorkers(files, existingMeta, onProgress)

  onStage?.('整理结构...')
  const artists = await groupTracksByFolder(files, metaResults, folderPaths)
  const folderTree = await buildFolderTree(files, metaResults, folderPaths)
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

  return { artists, folderTree, allTracks, fileCount: totalFiles }
}

export async function startWatching(
  folderPaths: string[],
  onChange: () => void
): Promise<void> {
  stopWatching()
  onChangeCallback = onChange

  for (const fp of folderPaths) {
    const watcher = chokidar.watch(fp, {
      ignored: /(^|[\/\\])\../,
      persistent: true,
      ignoreInitial: true,
      depth: 99,
      awaitWriteFinish: { stabilityThreshold: 500, pollInterval: 100 },
    })

    let timer: NodeJS.Timeout | null = null

    const scheduleChange = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => {
        onChangeCallback?.()
      }, CHOKIDAR_DELAY)
    }

    watcher.on('add', scheduleChange)
    watcher.on('change', scheduleChange)
    watcher.on('unlink', scheduleChange)

    watchers.push(watcher)
  }
}

export function stopWatching(): void {
  for (const w of watchers) {
    try { w.close() } catch { }
  }
  watchers = []
  onChangeCallback = null
}

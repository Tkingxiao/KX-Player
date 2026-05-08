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
const CHOKIDAR_DELAY = 1000

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
}

interface ScannedAlbum {
  name: string
  artist: string
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

async function enrichWithWorkers(
  files: string[],
  onProgress?: (completed: number, total: number) => void
): Promise<Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null }>> {
  const results = new Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null }>()
  const normalFiles = [...files]
  const total = files.length
  let completed = 0

  if (normalFiles.length === 0) {
    return results
  }

  const cpuCount = Math.max(1, os.cpus().length - 1)
  const chunkSize = Math.ceil(normalFiles.length / cpuCount)
  const chunks: string[][] = []
  for (let i = 0; i < normalFiles.length; i += chunkSize) {
    chunks.push(normalFiles.slice(i, i + chunkSize))
  }

  const workerPromises = chunks.map((chunk) => {
    return new Promise<void>((resolve) => {
      try {
        const workerPath = path.join(__dirname, 'workers', 'metadata-worker.js')
        if (!fs.existsSync(workerPath)) {
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null })
          }
          completed += chunk.length
          onProgress?.(completed, total)
          resolve()
          return
        }

        const worker = new Worker(workerPath, {
          workerData: { files: chunk, timeoutMs: SCAN_TIMEOUT_MS },
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
              })
            }
            completed += chunk.length
            onProgress?.(completed, total)
            resolve()
          } else if (msg.type === 'progress') {
            completed += 1
            onProgress?.(completed, total)
          }
        })

        worker.on('error', () => {
          if (!hasResponded) {
            for (const f of chunk) {
              results.set(f, { duration: 0, coverData: null, title: null, artist: null })
            }
            completed += chunk.length
            onProgress?.(completed, total)
          }
          resolve()
        })

        worker.on('exit', (code) => {
          if (!hasResponded && code !== 0) {
            for (const f of chunk) {
              results.set(f, { duration: 0, coverData: null, title: null, artist: null })
            }
            completed += chunk.length
            onProgress?.(completed, total)
          }
          resolve()
        })

        setTimeout(() => {
          if (!hasResponded) {
            worker.terminate()
            for (const f of chunk) {
              results.set(f, { duration: 0, coverData: null, title: null, artist: null })
            }
            completed += chunk.length
            onProgress?.(completed, total)
            resolve()
          }
        }, SCAN_TIMEOUT_MS)
      } catch {
        for (const f of chunk) {
          results.set(f, { duration: 0, coverData: null, title: null, artist: null })
        }
        completed += chunk.length
        onProgress?.(completed, total)
        resolve()
      }
    })
  })

  await Promise.all(workerPromises)
  onProgress?.(total, total)
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

function groupTracksByFolder(
  files: string[],
  metaResults: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null }>,
  rootPaths: string[]
): ScannedArtist[] {
  const artistMap = new Map<string, { path: string; albums: Map<string, ScannedAlbum> }>()

  for (const fp of files) {
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
      artist.albums.set(albumName, {
        name: albumName,
        artist: artistName,
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
    })
  }

  for (const [, artist] of artistMap) {
    for (const [, album] of artist.albums) {
      const firstTrack = album.tracks[0]
      if (firstTrack) {
        const metaResult = metaResults.get(firstTrack.path)
        if (metaResult?.coverData) {
          album.coverData = metaResult.coverData
        }
      }
    }
  }

  return [...artistMap.entries()].map(([name, a]) => ({
    name,
    path: a.path,
    albums: [...a.albums.values()],
  }))
}

function buildFolderTree(
  files: string[],
  metaResults: Map<string, { duration: number; coverData: string | null; title: string | null; artist: string | null }>,
  rootPaths: string[]
): FolderNode[] {
  const nodeMap = new Map<string, FolderNode>()
  const cleanRoots = rootPaths.map(rp => rp.replace(/\\/g, '/').replace(/\/+$/, ''))
  const roots: FolderNode[] = []

  function getOrCreateNode(dirPath: string, dirName: string): FolderNode {
    if (nodeMap.has(dirPath)) return nodeMap.get(dirPath)!
    const node: FolderNode = { name: dirName, path: dirPath, children: [], tracks: [], trackCount: 0, coverData: null }
    nodeMap.set(dirPath, node)
    return node
  }

  for (const fp of files) {
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

  // Find the first track with cover using sorted order (recursive).
  function findFirstCoverSorted(rootNode: FolderNode): string | null {
    for (const t of rootNode.tracks) {
      const meta = metaResults.get(t.path)
      if (meta?.coverData) return meta.coverData
    }
    for (const c of rootNode.children) {
      const r = findFirstCoverSorted(c)
      if (r) return r
    }
    return null
  }
  for (const [, node] of nodeMap) {
    node.children.sort((a, b) => a.name.localeCompare(b.name))
    node.tracks.sort((a, b) => a.name.localeCompare(b.name))
    node.trackCount = node.tracks.length
    for (const child of node.children) {
      node.trackCount += child.trackCount
    }
    node.coverData = findFirstCoverSorted(node)
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
  for (const r of roots) {
    r.coverData = findFirstCoverSorted(r)
  }
  return roots
}

export async function scanFoldersWithProgress(
  folderPaths: string[],
  onProgress?: (completed: number, total: number) => void,
  onStage?: (stage: string) => void
): Promise<{ artists: ScannedArtist[]; folderTree: FolderNode[]; fileCount: number }> {
  onStage?.('发现文件...')
  const files = await discoverFiles(folderPaths)
  const totalFiles = files.length
  onProgress?.(0, totalFiles)
  onStage?.(`解析元数据... (${totalFiles} 文件)`)

  const metaResults = await enrichWithWorkers(files, onProgress)

  onStage?.('整理结构...')
  const artists = groupTracksByFolder(files, metaResults, folderPaths)
  const folderTree = buildFolderTree(files, metaResults, folderPaths)

  return { artists, folderTree, fileCount: totalFiles }
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

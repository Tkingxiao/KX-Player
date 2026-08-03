// === Cover File Service ===
// Covers are stored as compressed JPEG files in the filesystem, NOT in SQLite.
// - Track covers:  covers/{trackId}.jpg
// - Folder covers: covers/folder_{md5(path)}.jpg
// - External cover files (cover.jpg/png/webp) are copied and used as folder covers.

import fs from 'node:fs'
import fsp from 'node:fs/promises'
import path from 'node:path'
import crypto from 'node:crypto'
import sharp from 'sharp'

const IMG_MIME: Record<string, string> = { jpg: 'jpeg', jpeg: 'jpeg', png: 'png', webp: 'webp', bmp: 'bmp', gif: 'gif' }
const COVER_EXTS = new Set(['.jpg', '.jpeg', '.png', '.webp', '.bmp', '.gif'])
const COVER_NAMES = ['cover', 'folder', 'front', 'albumart', 'album', 'art', 'jacket', '封面', '专辑封面', '专辑图', 'ジャケット']
const NON_COVER_HINTS = ['ui', '说明', 'screenshot', 'screen', 'manual', 'readme', 'player', 'capture', 'shot', 'ss', 'banner', 'icon', 'thumb', 'thumbnail', 'small', 'icon']

let _coversDir = ''

export function initCoverDir(userDataDir: string) {
  _coversDir = path.join(userDataDir, 'covers')
  if (!fs.existsSync(_coversDir)) fs.mkdirSync(_coversDir, { recursive: true })
}

function coversDir(): string {
  if (!_coversDir) throw new Error('coverService not initialized')
  return _coversDir
}

export function getCoversDir(): string { return coversDir() }

// --- Compression ---
async function compressToJpeg(input: Buffer): Promise<Buffer> {
  if (input.length < 10 * 1024) return input // skip tiny images
  try {
    return await sharp(input)
      .resize(400, 400, { fit: 'inside', withoutEnlargement: true })
      .jpeg({ quality: 70 })
      .toBuffer()
  } catch {
    return input
  }
}

// --- Save ---
export async function saveTrackCover(trackId: string, dataUrl: string | null): Promise<boolean> {
  if (!dataUrl) return false
  try {
    const match = dataUrl.match(/^data:image\/\w+;base64,(.+)$/)
    if (!match) {
      console.warn('[cover] saveTrackCover regex mismatch for', trackId, dataUrl.slice(0, 60))
      return false
    }
    const buffer = Buffer.from(match[1], 'base64')
    const compressed = await compressToJpeg(buffer)
    const filePath = path.join(coversDir(), `${trackId}.jpg`)
    fs.writeFileSync(filePath, compressed)
    return true
  } catch (e: any) {
    console.error('[cover] saveTrackCover failed for', trackId, ':', e?.message || e)
    return false
  }
}

export async function saveFolderCover(folderPath: string, dataUrl: string | null): Promise<boolean> {
  if (!dataUrl) return false
  try {
    const match = dataUrl.match(/^data:image\/\w+;base64,(.+)$/)
    if (!match) return false
    const buffer = Buffer.from(match[1], 'base64')
    const compressed = await compressToJpeg(buffer)
    const hash = crypto.createHash('md5').update(folderPath).digest('hex')
    const filePath = path.join(coversDir(), `folder_${hash}.jpg`)
    fs.writeFileSync(filePath, compressed)
    return true
  } catch {
    return false
  }
}

/** Copy an external cover file into the covers directory as a folder cover */
export async function saveExternalCover(folderPath: string, extFilePath: string): Promise<boolean> {
  try {
    const buffer = fs.readFileSync(extFilePath)
    const compressed = await compressToJpeg(buffer)
    const hash = crypto.createHash('md5').update(folderPath).digest('hex')
    const filePath = path.join(coversDir(), `folder_${hash}.jpg`)
    fs.writeFileSync(filePath, compressed)
    return true
  } catch {
    return false
  }
}

// --- Load ---
export function getTrackCoverPath(trackId: string): string | null {
  const p = path.join(coversDir(), `${trackId}.jpg`)
  return fs.existsSync(p) ? p : null
}

export function getTrackCoverDataUrl(trackId: string): string | null {
  const p = getTrackCoverPath(trackId)
  if (!p) return null
  try {
    const buffer = fs.readFileSync(p)
    return `data:image/jpeg;base64,${buffer.toString('base64')}`
  } catch {
    return null
  }
}

export function getFolderCoverPath(folderPath: string): string | null {
  const hash = crypto.createHash('md5').update(folderPath).digest('hex')
  const p = path.join(coversDir(), `folder_${hash}.jpg`)
  return fs.existsSync(p) ? p : null
}

export function getFolderCoverDataUrl(folderPath: string): string | null {
  const p = getFolderCoverPath(folderPath)
  if (!p) return null
  try {
    const buffer = fs.readFileSync(p)
    return `data:image/jpeg;base64,${buffer.toString('base64')}`
  } catch {
    return null
  }
}

/** Load covers for multiple track IDs at once */
export function getTrackCoversBatch(trackIds: string[]): Record<string, string> {
  const result: Record<string, string> = {}
  for (const id of trackIds) {
    const dataUrl = getTrackCoverDataUrl(id)
    if (dataUrl) result[id] = dataUrl
  }
  return result
}

/** Load all folder covers */
export function getAllFolderCovers(): Record<string, string> {
  const result: Record<string, string> = {}
  try {
    const files = fs.readdirSync(coversDir())
    for (const f of files) {
      if (!f.startsWith('folder_') || !f.endsWith('.jpg')) continue
      try {
        const buffer = fs.readFileSync(path.join(coversDir(), f))
        const md5 = f.slice(7, -4) // remove 'folder_' prefix and '.jpg' suffix
        // We can't reverse MD5 to get path, so we store the mapping in a separate file
        // For now, skip - we'll use folderCoverMap instead
      } catch { /* skip */ }
    }
  } catch { /* ignore */ }
  return result
}

// --- External Cover Detection ---
export function findExternalCoverInDir(dirPath: string): string | null {
  try {
    const entries = fs.readdirSync(dirPath)
    // Priority 1: exact match cover.* (prefer standard names)
    for (const name of COVER_NAMES) {
      for (const ext of ['jpg', 'jpeg', 'png', 'webp', 'bmp']) {
        const p = path.join(dirPath, `${name}.${ext}`)
        if (fs.existsSync(p)) {
          try {
            const stat = fs.statSync(p)
            if (stat.size > 0 && stat.size < 15 * 1024 * 1024) return p
          } catch { /* continue */ }
        }
      }
    }
    // Priority 2: any image file, excluding known non-cover patterns
    for (const entry of entries) {
      const ext = path.extname(entry).toLowerCase()
      if (!COVER_EXTS.has(ext)) continue
      const name = entry.toLowerCase()
      // Skip known non-cover files
      const isNonCover = NON_COVER_HINTS.some(h => name.includes(h))
      if (isNonCover) continue
      const p = path.join(dirPath, entry)
      try {
        const stat = fs.statSync(p)
        if (stat.size > 0 && stat.size < 15 * 1024 * 1024) return p
      } catch { /* continue */ }
    }
  } catch { /* ignore */ }
  return null
}

// --- Folder cover path mapping (for reverse lookup) ---
let _folderCoverMap: Record<string, string> = {}
let _folderCoverMapPath = ''

export function loadFolderCoverMap(userDataDir: string) {
  _folderCoverMapPath = path.join(userDataDir, 'folder-cover-map.json')
  try {
    if (fs.existsSync(_folderCoverMapPath)) {
      _folderCoverMap = JSON.parse(fs.readFileSync(_folderCoverMapPath, 'utf-8'))
    }
  } catch {
    _folderCoverMap = {}
  }
}

export function saveFolderCoverMap() {
  try {
    fs.writeFileSync(_folderCoverMapPath, JSON.stringify(_folderCoverMap), 'utf-8')
  } catch { /* ignore */ }
}

export function setFolderCoverMapping(folderPath: string) {
  _folderCoverMap[folderPath] = crypto.createHash('md5').update(folderPath).digest('hex')
  saveFolderCoverMap()
}

/** Return the local absolute path of a folder cover (no base64 read). */
export function getFolderCoverPathByMapping(folderPath: string): string | null {
  const hash = _folderCoverMap[folderPath]
  if (!hash) return null
  const p = path.join(coversDir(), `folder_${hash}.jpg`)
  if (!fs.existsSync(p)) return null
  return p
}

export function getFolderCoverByMapping(folderPath: string): string | null {
  const hash = _folderCoverMap[folderPath]
  if (!hash) return null
  const p = path.join(coversDir(), `folder_${hash}.jpg`)
  if (!fs.existsSync(p)) return null
  try {
    const buffer = fs.readFileSync(p)
    return `data:image/jpeg;base64,${buffer.toString('base64')}`
  } catch {
    return null
  }
}

/** Load covers for many track IDs in parallel (async). */
export async function getTrackCoversBatchAsync(trackIds: string[]): Promise<Record<string, string>> {
  const result: Record<string, string> = {}
  await Promise.all(trackIds.map(async (id) => {
    const p = getTrackCoverPath(id)
    if (!p) return
    try {
      const buffer = await fsp.readFile(p)
      result[id] = `data:image/jpeg;base64,${buffer.toString('base64')}`
    } catch { /* skip */ }
  }))
  return result
}

/** Load covers for many folder paths in parallel (async). */
export async function getFolderCoversBatchAsync(folderPaths: string[]): Promise<Record<string, string>> {
  const result: Record<string, string> = {}
  await Promise.all(folderPaths.map(async (folderPath) => {
    const hash = _folderCoverMap[folderPath]
    if (!hash) return
    const p = path.join(coversDir(), `folder_${hash}.jpg`)
    try {
      const buffer = await fsp.readFile(p)
      result[folderPath] = `data:image/jpeg;base64,${buffer.toString('base64')}`
    } catch { /* skip */ }
  }))
  return result
}

/** Load all folder covers in parallel (async). */
export async function getAllFolderCoversFromMapAsync(): Promise<Record<string, string>> {
  const entries = Object.entries(_folderCoverMap)
  const result: Record<string, string> = {}
  await Promise.all(entries.map(async ([folderPath, hash]) => {
    const p = path.join(coversDir(), `folder_${hash}.jpg`)
    try {
      const buffer = await fsp.readFile(p)
      result[folderPath] = `data:image/jpeg;base64,${buffer.toString('base64')}`
    } catch { /* skip */ }
  }))
  return result
}

export function getAllFolderCoversFromMap(): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [folderPath, hash] of Object.entries(_folderCoverMap)) {
    const p = path.join(coversDir(), `folder_${hash}.jpg`)
    if (fs.existsSync(p)) {
      try {
        const buffer = fs.readFileSync(p)
        result[folderPath] = `data:image/jpeg;base64,${buffer.toString('base64')}`
      } catch { /* skip */ }
    }
  }
  return result
}

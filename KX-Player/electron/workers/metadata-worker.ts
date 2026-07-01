import { parentPort, workerData } from 'node:worker_threads'
import path from 'node:path'
import fs from 'node:fs'
import IconvLite from 'iconv-lite'
import * as musicMetadata from 'music-metadata'

const DSD_EXTS = new Set(['.dsf', '.dff', '.dsd'])
const VIDEO_EXTS = new Set(['.mp4', '.mkv', '.avi', '.mov', '.webm', '.flv', '.wmv'])

// Skip full metadata parsing for files larger than this threshold (bytes).
// For ASMR/audio files >10MB, music-metadata is very slow due to cover
// extraction and full file scanning. Use filename-based info instead.
const LARGE_FILE_SKIP_PARSE = 10 * 1024 * 1024

// Detect garbled text from Shift-JIS misread as ISO-8859-1
// Characters 0x80-0xFF in Latin-1 are common indicators
function isLikelyGarbled(text: string): boolean {
  if (!text) return false
  let garbledCount = 0
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i)
    // Latin-1 Supplement range (0x80-0xFF): not typical in normal text
    if ((code >= 0x80 && code <= 0x9F) || (code >= 0xA1 && code <= 0xFF)) {
      garbledCount++
    }
  }
  // If more than 20% of chars are in garbled range, likely mis-encoded
  return garbledCount / text.length > 0.2
}

// Try to fix garbled text by re-interpreting as Shift-JIS
function tryFixEncoding(text: string): string {
  if (!text || !isLikelyGarbled(text)) return text
  try {
    // Convert ISO-8859-1 chars back to bytes
    const bytes = new Uint8Array(text.length)
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i)
    // Re-interpret as Shift-JIS
    const fixed = IconvLite.decode(bytes, 'Shift_JIS')
    // Verify it looks better (contains Japanese chars)
    if (/[぀-ヿ＀-￯]/.test(fixed)) return fixed
  } catch { /* ignore */ }
  // Fallback: try Windows-31J (superset of Shift-JIS)
  try {
    const bytes = new Uint8Array(text.length)
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i)
    const fixed = IconvLite.decode(bytes, 'Windows-31J')
    if (/[぀-ヿ＀-￯]/.test(fixed)) return fixed
  } catch { /* ignore */ }
  return text
}

interface WorkerResultItem {
  path: string; duration: number; coverB64: string | null; title: string | null; artist: string | null;
  genre: string | null; bitrate: number | null; sampleRate: number | null;
}

function extractBasicInfo(filePath: string): WorkerResultItem {
  const base = path.basename(filePath, path.extname(filePath))
  return {
    path: filePath,
    duration: 0,
    coverB64: null,
    title: base,
    artist: null,
    genre: null,
    bitrate: null,
    sampleRate: null,
  }
}

async function parseFile(filePath: string): Promise<WorkerResultItem | null> {
  try {
    if (!fs.existsSync(filePath)) {
      return extractBasicInfo(filePath)
    }

    const ext = path.extname(filePath).toLowerCase()
    const stat = fs.statSync(filePath)

    // Skip music-metadata for video files; use filename only
    if (VIDEO_EXTS.has(ext)) {
      return extractBasicInfo(filePath)
    }

    // Skip full metadata parse for very large files; use filename only
    if (stat.size > LARGE_FILE_SKIP_PARSE) {
      return extractBasicInfo(filePath)
    }

    // Parse metadata (with covers for small files; large files use filename only)
    // skipCovers=false for ≤10MB files so they retain embedded cover art
    const meta = await musicMetadata.parseFile(filePath, {
      duration: true,
      skipCovers: false,
    })
    
    let coverB64: string | null = null
    if (meta.common.picture && meta.common.picture.length > 0) {
      const pic = meta.common.picture[0]
      let data: Buffer
      if (Buffer.isBuffer(pic.data)) {
        data = pic.data
      } else if (pic.data instanceof Uint8Array) {
        data = Buffer.from(pic.data)
      } else {
        data = Buffer.alloc(0)
      }
      const maxSize = 15 * 1024 * 1024
      if (data.length > 0 && data.length <= maxSize) {
        const b64 = data.toString('base64')
        let format = pic.format || 'image/jpeg'
        if (!format.startsWith('image/')) format = `image/${format}`
        coverB64 = `data:${format};base64,${b64}`
      } else if (data.length > maxSize) {
        console.warn(`Cover too large for ${filePath}: ${data.length} bytes`)
      }
    }

    const rawTitle = meta.common.title || path.basename(filePath, path.extname(filePath))
    const rawArtist = meta.common.artist || null
    const genre = meta.common.genre && meta.common.genre.length > 0 ? meta.common.genre.join(', ') : null
    const bitrate = meta.format.bitrate ? Math.round(meta.format.bitrate) : null
    const sampleRate = meta.format.sampleRate || null
    return {
      path: filePath,
      duration: meta.format.duration ? Math.round(meta.format.duration) : 0,
      coverB64,
      title: tryFixEncoding(rawTitle),
      artist: tryFixEncoding(rawArtist),
      genre,
      bitrate,
      sampleRate,
    }
  } catch (err) {
    return extractBasicInfo(filePath)
  }
}

async function processBatch(files: string[], timeoutMs: number): Promise<WorkerResultItem[]> {
  const results: WorkerResultItem[] = []
  let lastReported = 0
  let reportTimer: NodeJS.Timeout | null = null
  function reportProgress(completed: number, immediate = false) {
    if (!parentPort) return
    const shouldReport = immediate || completed === files.length || completed - lastReported >= Math.max(1, Math.floor(files.length * 0.05))
    if (shouldReport) {
      if (reportTimer) { clearTimeout(reportTimer); reportTimer = null }
      lastReported = completed
      parentPort.postMessage({ type: 'progress', completed, total: files.length })
    } else if (!reportTimer) {
      reportTimer = setTimeout(() => {
        reportTimer = null
        lastReported = completed
        parentPort.postMessage({ type: 'progress', completed, total: files.length })
      }, 80)
    }
  }
  for (let i = 0; i < files.length; i++) {
    const filePath = files[i]
    try {
      const result = await Promise.race([
        parseFile(filePath),
        new Promise<never>((_, reject) => setTimeout(() => reject(new Error('timeout')), timeoutMs))
      ])
      results.push(result)
    } catch (err) {
      console.error(`Worker error on ${filePath}:`, err)
      results.push({
        path: filePath,
        duration: 0,
        coverB64: null,
        title: path.basename(filePath, path.extname(filePath)),
        artist: null,
        genre: null,
        bitrate: null,
        sampleRate: null,
      })
    }
    reportProgress(i + 1)
  }
  reportProgress(files.length, true)
  return results
}

if (parentPort) {
  const { files, timeoutMs } = workerData
  processBatch(files, timeoutMs).then(results => {
    if (parentPort) {
      parentPort.postMessage({ type: 'result', results })
    }
  }).catch(err => {
    console.error('Worker batch error:', err)
    if (parentPort) {
      parentPort.postMessage({ type: 'error', message: err.message })
    }
  })
}

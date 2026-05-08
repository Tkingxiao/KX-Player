import { parentPort, workerData } from 'node:worker_threads'
import path from 'node:path'
import fs from 'node:fs'

let musicMetadata: any = null

const DSD_EXTS = new Set(['.dsf', '.dff', '.dsd'])

async function ensureMM() {
  if (!musicMetadata) {
    try {
      musicMetadata = await import('music-metadata')
    } catch (mmErr) {
      console.error('Failed to load music-metadata:', mmErr)
      musicMetadata = null
    }
  }
}

function extractBasicInfo(filePath: string): { path: string; duration: number; coverB64: null; title: string | null; artist: string | null } {
  const base = path.basename(filePath, path.extname(filePath))
  return {
    path: filePath,
    duration: 0,
    coverB64: null,
    title: base,
    artist: null,
  }
}

async function parseFile(filePath: string): Promise<{
  path: string; duration: number; coverB64: string | null; title: string | null; artist: string | null
} | null> {
  try {
    if (!fs.existsSync(filePath)) {
      return extractBasicInfo(filePath)
    }
    
    await ensureMM()
    
    if (!musicMetadata) {
      return extractBasicInfo(filePath)
    }
    
    const ext = path.extname(filePath).toLowerCase()
    const isDSD = DSD_EXTS.has(ext)
    
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
      const maxSize = 5 * 1024 * 1024
      if (data.length > 0 && data.length <= maxSize) {
        const b64 = data.toString('base64')
        coverB64 = `data:${pic.format || 'image/jpeg'};base64,${b64}`
      } else if (data.length > maxSize) {
        console.warn(`Cover too large for ${filePath}: ${data.length} bytes`)
      }
    }

    return {
      path: filePath,
      duration: meta.format.duration ? Math.round(meta.format.duration) : 0,
      coverB64,
      title: meta.common.title || path.basename(filePath, path.extname(filePath)),
      artist: meta.common.artist || null,
    }
  } catch (err) {
    console.error(`Error parsing ${filePath}:`, err)
    return extractBasicInfo(filePath)
  }
}

async function processBatch(files: string[], timeoutMs: number) {
  const results: any[] = []
  for (let i = 0; i < files.length; i++) {
    const filePath = files[i]
    try {
      const result = await Promise.race([
        parseFile(filePath),
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error('TIMEOUT')), timeoutMs)
        ),
      ])
      if (result) results.push(result)
    } catch (err) {
      console.error(`Worker error on ${filePath}:`, err)
      results.push({
        path: filePath,
        duration: 0,
        coverB64: null,
        title: path.basename(filePath, path.extname(filePath)),
        artist: null,
      })
    }
    if (parentPort) {
      parentPort.postMessage({ type: 'progress', completed: i + 1, total: files.length })
    }
  }
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

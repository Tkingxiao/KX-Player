// === Memory Monitor ===
// Tracks per-process memory usage at key lifecycle checkpoints.
// Writes samples both on a fixed 10s interval and at explicit marks.
// Renderer reports its own snapshots to main via the `mem:report` IPC,
// so this module is the single writer for `memory.log`.

import fs from 'node:fs'
import path from 'node:path'
import { app } from 'electron'

const LOG_FILE_NAME = 'memory.log'
const MAX_LOG_BYTES = 2 * 1024 * 1024 // rotate when file exceeds 2MB
const KEEP_BYTES = 1024 * 1024          // after rotation keep last 1MB
const SAMPLE_INTERVAL_MS = 10_000       // periodic sample every 10s

export interface MemorySample {
  ts: string                // ISO timestamp
  source: 'main' | 'renderer'
  event: string             // free-form label, e.g. "tick", "startup:loadLibraryData:start"
  pid?: number|string       // process id (mainly for distinguishing renderer webContents)
  rssMb?: number
  heapUsedMb?: number
  heapTotalMb?: number
  externalMb?: number        // main: process.memoryUsage().external
  jsHeapUsedMb?: number      // renderer: performance.memory.usedJSHeapSize
  jsHeapTotalMb?: number
  rendererHeapMb?: number    // renderer: process.memoryUsage.heapUsed fallback
  rendererHeapTotalMb?: number
  rendererExtMb?: number
  arraysMb?: number          // renderer: ArrayBuffer-backed memory estimate
  deltaMs?: number           // ms since last sample in the same source
  extra?: string             // optional structured note (e.g. duration, fileCount)
}

export interface RendererMemoryReading {
  rss: number
  heapUsed: number
  heapTotal: number
  external: number
  jsHeapUsed?: number
  jsHeapTotal?: number
  privateBytes?: number
  sharedBytes?: number
  blinkAllocated?: number
  blinkTotal?: number
}

let logPath: string | null = null
let lastSampleAt: Record<string, number> = { main: 0 }
let intervalHandle: NodeJS.Timeout | null = null

function formatLine(s: MemorySample): string {
  const parts: string[] = [s.ts, s.source, s.event]
  if (s.pid !== undefined) parts.push(`pid=${s.pid}`)
  if (s.rssMb !== undefined) parts.push(`rss=${s.rssMb.toFixed(1)}MB`)
  if (s.heapUsedMb !== undefined) parts.push(`heap=${s.heapUsedMb.toFixed(1)}/${s.heapTotalMb?.toFixed(1) ?? '?'}MB`)
  if (s.jsHeapUsedMb !== undefined && s.jsHeapUsedMb > 0) parts.push(`jsHeap=${s.jsHeapUsedMb.toFixed(1)}/${s.jsHeapTotalMb?.toFixed(1) ?? '?'}MB`)
  if (s.externalMb !== undefined) parts.push(`ext=${s.externalMb.toFixed(1)}MB`)
  if (s.rendererHeapMb !== undefined) parts.push(`rheap=${s.rendererHeapMb.toFixed(1)}/${s.rendererHeapTotalMb?.toFixed(1) ?? '?'}MB`)
  if (s.rendererExtMb !== undefined) parts.push(`rext=${s.rendererExtMb.toFixed(1)}MB`)
  if (s.arraysMb !== undefined) parts.push(`buffers=${s.arraysMb.toFixed(1)}MB`)
  if (s.deltaMs !== undefined) parts.push(`dt=${s.deltaMs}ms`)
  if (s.extra) parts.push(s.extra)
  return parts.join(' | ') + '\n'
}

function writeLog(line: string) {
  if (!logPath) return
  try {
    if (fs.existsSync(logPath)) {
      const stat = fs.statSync(logPath)
      if (stat.size > MAX_LOG_BYTES) {
        const content = fs.readFileSync(logPath, 'utf-8')
        const keepFrom = Math.max(0, content.length - KEEP_BYTES)
        fs.writeFileSync(logPath, '// --- rotated ---\n' + content.slice(keepFrom) + line, 'utf-8')
        return
      }
    }
    fs.appendFileSync(logPath, line, 'utf-8')
  } catch {
    /* never throw from monitor */
  }
}

function nowIso(): string {
  return new Date().toISOString()
}

function mb(bytes: number | null | undefined): number | undefined {
  if (bytes == null || !Number.isFinite(bytes)) return undefined
  return bytes / (1024 * 1024)
}

function sampleMb(value: number | null | undefined): number | undefined {
  if (value == null || !Number.isFinite(value)) return undefined
  return value
}

function hasRendererMemory(reading: RendererMemoryReading | null | undefined): reading is RendererMemoryReading {
  if (!reading) return false
  return !!(reading.rss || reading.heapUsed || reading.jsHeapUsed || reading.privateBytes)
}

const RENDERER_MEMORY_PROBE_JS = `
  (async function () {
    const KB = 1024;
    const toBytes = (value) => {
      const n = Number(value) || 0;
      return n > 0 ? n * KB : 0;
    };
    const out = {
      rss: 0,
      heapUsed: 0,
      heapTotal: 0,
      external: 0,
      jsHeapUsed: 0,
      jsHeapTotal: 0,
      privateBytes: 0,
      sharedBytes: 0,
      blinkAllocated: 0,
      blinkTotal: 0,
    };
    try {
      if (typeof process !== 'undefined' && typeof process.getProcessMemoryInfo === 'function') {
        const mem = await process.getProcessMemoryInfo();
        const resident = toBytes(mem && mem.residentSet);
        const privateBytes = toBytes(mem && mem.private);
        out.privateBytes = privateBytes;
        out.sharedBytes = toBytes(mem && mem.shared);
        out.rss = resident || privateBytes;
      }
    } catch (_) {}
    try {
      if (typeof process !== 'undefined' && typeof process.getHeapStatistics === 'function') {
        const heap = process.getHeapStatistics();
        out.jsHeapUsed = toBytes(heap && heap.usedHeapSize);
        out.jsHeapTotal = toBytes(heap && heap.totalHeapSize);
        out.heapUsed = out.jsHeapUsed;
        out.heapTotal = out.jsHeapTotal;
        out.external = toBytes(heap && heap.mallocedMemory);
      }
    } catch (_) {}
    try {
      if (typeof performance !== 'undefined' && performance && performance.memory) {
        out.jsHeapUsed = out.jsHeapUsed || (Number(performance.memory.usedJSHeapSize) || 0);
        out.jsHeapTotal = out.jsHeapTotal || (Number(performance.memory.totalJSHeapSize) || 0);
        out.heapUsed = out.heapUsed || out.jsHeapUsed;
        out.heapTotal = out.heapTotal || out.jsHeapTotal;
      }
    } catch (_) {}
    try {
      if (typeof process !== 'undefined' && typeof process.getBlinkMemoryInfo === 'function') {
        const blink = process.getBlinkMemoryInfo();
        out.blinkAllocated = toBytes(blink && blink.allocated);
        out.blinkTotal = toBytes(blink && blink.total);
        out.external = out.external || out.blinkAllocated;
      }
    } catch (_) {}
    return out;
  })()
`

export async function readRendererMemoryFromWebContents(wc: any): Promise<RendererMemoryReading | null> {
  if (!wc || wc.isDestroyed?.()) return null
  let reading: RendererMemoryReading | null = null
  try {
    reading = await wc.executeJavaScript(RENDERER_MEMORY_PROBE_JS, true)
  } catch {
    reading = null
  }

  let metricReading: RendererMemoryReading | null = null
  try {
    const pid = wc.getOSProcessId?.()
    const metrics = app.getAppMetrics?.() || []
    const metric = metrics.find((m: any) => pid && m.pid === pid)
      || metrics.find((m: any) => m.type === 'Tab' || m.type === 'Renderer')
    const mem = metric?.memory
    if (mem) {
      const workingSet = Number(mem.workingSetSize || 0) * 1024
      const privateBytes = Number(mem.privateBytes || 0) * 1024
      metricReading = {
        rss: workingSet || privateBytes,
        privateBytes,
        sharedBytes: 0,
        heapUsed: 0,
        heapTotal: 0,
        external: 0,
        jsHeapUsed: 0,
        jsHeapTotal: 0,
      }
    }
  } catch {
    metricReading = null
  }

  if (hasRendererMemory(reading) && metricReading) {
    return {
      ...reading,
      rss: reading.rss || metricReading.rss,
      privateBytes: reading.privateBytes || metricReading.privateBytes,
      sharedBytes: reading.sharedBytes || metricReading.sharedBytes,
    }
  }
  if (hasRendererMemory(reading)) return reading
  return metricReading
}

export function initMemoryMonitor(targetDir: string) {
  if (logPath) return // already initialized
  if (!fs.existsSync(targetDir)) fs.mkdirSync(targetDir, { recursive: true })
  logPath = path.join(targetDir, LOG_FILE_NAME)
  // Clear at startup so we only see fresh data; comment out to keep history.
  try { fs.writeFileSync(logPath, `// memory monitor started at ${nowIso()} (pid=${process.pid})\n`, 'utf-8') } catch { /* ignore */ }
  startInterval('main')
}

function getMainSample(event: string, extra?: string, pid?: number): MemorySample {
  const mu = process.memoryUsage()
  const ts = nowIso()
  const prev = lastSampleAt.main
  lastSampleAt.main = Date.now()
  return {
    ts,
    source: 'main',
    event,
    pid,
    rssMb: mb(mu.rss),
    heapUsedMb: mb(mu.heapUsed),
    heapTotalMb: mb(mu.heapTotal),
    externalMb: mb(mu.external),
    deltaMs: prev ? Date.now() - prev : undefined,
    extra,
  }
}

export function markMain(event: string, extra?: string): void {
  if (!logPath) return
  writeLog(formatLine(getMainSample(event, extra, process.pid)))
}

function startInterval(_source: 'main') {
  if (intervalHandle) return
  intervalHandle = setInterval(async () => {
    try {
      writeLog(formatLine(getMainSample('tick')))
      try { await sampleRendererFromMain('renderer-pull') } catch { /* ignore */ }
    } catch { /* ignore */ }
  }, SAMPLE_INTERVAL_MS)
  intervalHandle.unref?.()
}

// Reference to the renderer BrowserWindow so we can pull its memory via
// webContents.getMemoryUsage() — a reliable Electron API that doesn't
// depend on flags or contextIsolation quirks.
let mainWindowRef: any = null
export function bindMainWindow(win: any) {
  mainWindowRef = win
}

// Pull renderer memory stats from the WebContents that owns the renderer and
// write the sample directly to the log file. Returns the sample (or null).
export async function sampleRendererFromMain(event = 'tick'): Promise<MemorySample | null> {
  if (!mainWindowRef || mainWindowRef.isDestroyed()) return null
  const wc = mainWindowRef.webContents
  if (!wc || wc.isDestroyed()) return null
  try {
    const stats = await readRendererMemoryFromWebContents(wc)
    if (!hasRendererMemory(stats)) return null
    const ts = nowIso()
    const extra: string[] = []
    if (stats.privateBytes) extra.push(`priv=${mb(stats.privateBytes)?.toFixed(1)}MB`)
    if (stats.sharedBytes) extra.push(`shared=${mb(stats.sharedBytes)?.toFixed(1)}MB`)
    if (stats.blinkAllocated) extra.push(`blink=${mb(stats.blinkAllocated)?.toFixed(1)}MB`)
    const sample: MemorySample = {
      ts,
      source: 'renderer',
      event,
      pid: 'renderer',
      rssMb: mb(stats.rss),
      jsHeapUsedMb: mb(stats.jsHeapUsed || stats.heapUsed),
      jsHeapTotalMb: mb(stats.jsHeapTotal || stats.heapTotal),
      externalMb: mb(stats.external),
      deltaMs: lastSampleAt['main:r-pull'] ? Date.now() - lastSampleAt['main:r-pull'] : undefined,
      extra: extra.join(' ') || undefined,
    }
    lastSampleAt['main:r-pull'] = Date.now()
    writeLog(formatLine(sample))
    return sample
  } catch {
    return null
  }
}

// Called from renderer via IPC. Accepts renderer-side performance.memory data.
export function reportRendererSample(sample: Partial<MemorySample> & { event: string }): void {
  if (!logPath) return
  const ts = sample.ts || nowIso()
  const source: MemorySample['source'] = 'renderer'
  const prev = lastSampleAt[`r-${sample.pid ?? 'x'}`] || 0
  lastSampleAt[`r-${sample.pid ?? 'x'}`] = Date.now()
  const line: MemorySample = {
    ts,
    source,
    event: sample.event,
    pid: sample.pid,
    jsHeapUsedMb: sampleMb(sample.jsHeapUsedMb),
    jsHeapTotalMb: sampleMb(sample.jsHeapTotalMb),
    rendererHeapMb: sampleMb(sample.rendererHeapMb),
    rendererHeapTotalMb: sampleMb(sample.rendererHeapTotalMb),
    rendererExtMb: sampleMb(sample.rendererExtMb),
    heapUsedMb: sampleMb(sample.heapUsedMb),
    heapTotalMb: sampleMb(sample.heapTotalMb),
    rssMb: sampleMb(sample.rssMb),
    deltaMs: prev ? Date.now() - prev : undefined,
    extra: sample.extra,
    externalMb: undefined,
  }
  writeLog(formatLine(line))
}

export function stopMemoryMonitor() {
  if (intervalHandle) {
    clearInterval(intervalHandle)
    intervalHandle = null
  }
}

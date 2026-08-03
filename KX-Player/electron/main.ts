import { app, BrowserWindow, ipcMain, dialog, clipboard, shell, Tray, Menu, nativeImage, protocol, net } from 'electron'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import { execFile } from 'node:child_process'
import { scanFoldersWithProgress, scanFoldersIncremental, startWatching, stopWatching, terminateWorkerPool } from './fileScanner'
import { loadLibrarySnapshot, loadTrackListSnapshot, loadTrackMetadataIndex, loadFullMetadataIndex, saveLibrarySnapshot } from './libraryDb'
import { initCoverDir, loadFolderCoverMap, saveTrackCover, saveFolderCover, saveExternalCover, setFolderCoverMapping, getTrackCoverDataUrl, getTrackCoversBatchAsync, getFolderCoversBatchAsync, getFolderCoverByMapping, getFolderCoverPathByMapping, getAllFolderCoversFromMapAsync, findExternalCoverInDir, getCoversDir, getTrackCoverPath } from './coverService'
import { initMemoryMonitor, markMain, reportRendererSample, bindMainWindow, sampleRendererFromMain, readRendererMemoryFromWebContents } from './memMonitor'

// Shared MIME type mapping for image files
const IMG_MIME: Record<string, string> = { jpg: 'jpeg', jpeg: 'jpeg', png: 'png', bmp: 'bmp', webp: 'webp', gif: 'gif' }

// Log file helper with rotation: keep last 1MB of logs
const MAX_LOG_BYTES = 1024 * 1024
function writeLog(msg: string) {
  try {
    const logPath = path.join(getUserDataDir(), 'kx-player-log.txt')
    const ts = new Date().toISOString()
    const line = `[${ts}] ${msg}\n`
    // Rotate if file exceeds max size
    if (fs.existsSync(logPath)) {
      const stat = fs.statSync(logPath)
      if (stat.size > MAX_LOG_BYTES) {
        const content = fs.readFileSync(logPath, 'utf-8')
        // Keep only the last ~500KB
        const keepFrom = Math.max(0, content.length - MAX_LOG_BYTES / 2)
        fs.writeFileSync(logPath, content.slice(keepFrom) + line, 'utf-8')
        return
      }
    }
    fs.appendFileSync(logPath, line, 'utf-8')
  } catch { /* ignore */ }
}

// Disable crash reporting and GPU cache to suppress startup errors
app.commandLine.appendSwitch('disable-crashpad')
app.commandLine.appendSwitch('disable-breakpad')
app.commandLine.appendSwitch('disable-gpu-shader-disk-cache')
app.commandLine.appendSwitch('disable-extensions')
// Expose `performance.memory.usedJSHeapSize` / `totalJSHeapSize` to the renderer
// with real (non-zero) values instead of Chromium's default placeholder.
// Both Chromium switch and V8 flag are needed in Electron:
app.commandLine.appendSwitch('enable-precise-memory-info')
app.commandLine.appendSwitch('js-flags', '--expose-gc --enable-precise-memory-info')

let mainWindow: BrowserWindow | null = null

function getUserDataDir(): string {
  return app.getPath('userData')
}

function getSettingsPath(): string {
  return path.join(getUserDataDir(), 'kx-player-settings.json')
}

function getLibraryDbPath(): string {
  return path.join(getUserDataDir(), 'kx-player-library.sqlite')
}

function getBgImagePath(): string {
  // Keep background image in userData so it survives version upgrades
  return path.join(getUserDataDir(), 'kx-player-bg.png')
}

function stripCoverPayloadForRenderer(snapshot: any) {
  if (!snapshot) return snapshot
  const stripTrack = (track: any) => {
    if (!track) return
    track.coverData = null
    track.albumCoverData = null
  }
  for (const track of snapshot.allTracks || []) stripTrack(track)
  for (const artist of snapshot.artists || []) {
    for (const album of artist.albums || []) {
      album.coverData = null
      for (const track of album.tracks || []) stripTrack(track)
    }
  }
  const stripFolders = (nodes: any[]) => {
    for (const node of nodes || []) {
      node.coverData = null
      for (const track of node.tracks || []) stripTrack(track)
      stripFolders(node.children || [])
    }
  }
  stripFolders(snapshot.folderTree || [])
  return snapshot
}

// Project root (one level above `electron/`) — used for memory.log etc.
function getProjectRoot(): string {
  return path.resolve(__dirname, '..')
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1200,
    height: 800,
    minWidth: 900,
    minHeight: 600,
    frame: false,
    titleBarStyle: 'hidden',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      // Disable webSecurity so the renderer can reference local cover files
      // via file:// URLs. Safe here because all assets come from our own
      // userData/<covers> dir; only the renderer (running our packaged UI)
      // can issue requests, and CSP still blocks inline scripts elsewhere.
      webSecurity: false,
    },
    icon: path.join(__dirname, '../public/favicon.ico'),
    backgroundColor: '#1a1a1e',
  })

  mainWindow.webContents.session.setPermissionRequestHandler((_webContents, permission, callback) => {
    const allowed = ['media']
    callback(allowed.includes(permission))
  })

  mainWindow.on('maximize', () => {
    mainWindow?.webContents.send('window:maximizeChange', true)
  })

  mainWindow.on('unmaximize', () => {
    mainWindow?.webContents.send('window:maximizeChange', false)
  })

  mainWindow.on('closed', () => {
    mainWindow = null
  })

  if (process.env.VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(process.env.VITE_DEV_SERVER_URL)
  } else {
    mainWindow.loadFile(path.join(__dirname, '../dist/index.html'))
  }
}

app.whenReady().then(() => {
  // Set cache to userData subdirectory to avoid access-denied errors
  app.setPath('cache', path.join(getUserDataDir(), 'Cache'))

  // Initialize cover file service
  initCoverDir(getUserDataDir())
  loadFolderCoverMap(getUserDataDir())

  // Initialize memory monitor — log lives in KX-Player/memory.log so it's
  // easy to inspect during development and postmortem analysis.
  initMemoryMonitor(getProjectRoot())
  markMain('main:appReady')

  createWindow()
  if (mainWindow) bindMainWindow(mainWindow)
  createTray() // Create system tray icon at startup

  // Diagnostic: write Electron process metrics every 10s.
  // Also, for the Renderer (Tab) process, getProcessMemoryInfo() returns more
  // detail (privateBytes, sharedBytes, workingSetSize + V8 heap stats via
  // `pid`. We aggregate the top consumers so we can attribute RSS growth.
  setInterval(() => {
    try {
      const metrics = app.getAppMetrics?.() || []
      const summary = metrics.map((m: any) => {
        const mem = m.memory || {}
        const ws = mem.workingSetSize || mem.privateBytes || 0
        const cpu = m.cpu ? (m.cpu.percentCPUUsage || 0).toFixed(1) : '0'
        return `${m.type}:${m.pid}:${Math.round(ws / 1024)}MB/cpu=${cpu}`
      }).join(' | ')
      markMain('main:metrics', `procs=${metrics.length} ${summary}`)
      // For the renderer process, get a separate detailed readout.
      try {
        for (const m of metrics) {
          if (m.type === 'Tab' || m.type === 'Renderer') {
            const info = (process as any).getProcessMemoryInfo?.() || null
            // process.getProcessMemoryInfo is for the current process; for other
            // processes we use app.getProcessMemoryInfo (Electron 28+).
            const fullInfo = (app as any).getProcessMemoryInfo?.(m.pid) || null
            const ws = (m.memory?.workingSetSize || 0) / 1024
            const priv = (m.memory?.privateBytes || 0) / 1024
            const shared = (m.memory?.sharedBytes || 0) / 1024
            markMain('main:procDetail',
              `type=${m.type} pid=${m.pid}`
              + ` ws=${ws.toFixed(0)}MB priv=${priv.toFixed(0)}MB shared=${shared.toFixed(0)}MB`
              + (fullInfo ? ` peak=${(fullInfo.peakWorkingSetSize || 0) / 1024 | 0}MB` : '')
            )
          }
        }
      } catch {}
    } catch {}
  }, 10000).unref?.()

  // Periodically sample renderer memory directly from Electron APIs.
  setInterval(() => {
    sampleRendererFromMain('renderer-pull').catch(() => {})
  }, 10000).unref?.()

  // Intercept window close to hide to tray instead (catches Alt+F4, taskbar close, etc.)
  mainWindow?.on('close', (event) => {
    if (!forceCloseFlag) {
      event.preventDefault()
      mainWindow?.hide()
    }
  })

  // Bypass certificate verification only for dev server
  if (process.env.VITE_DEV_SERVER_URL) {
    mainWindow?.webContents.session.setCertificateVerifyProc((_request, callback) => {
      callback(0) // 0 = net::OK, accept all certificates
    })
  }

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow()
    }
  })
})

app.on('window-all-closed', () => {
  // If tray is active and not force-closing, keep app running in background
  if (tray && !forceCloseFlag) return
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

ipcMain.handle('dialog:openFolder', async () => {
  if (!mainWindow) return null
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openDirectory'],
    title: '选择音乐文件夹',
  })
  if (result.canceled || result.filePaths.length === 0) return null
  return result.filePaths
})

ipcMain.handle('dialog:openImageFile', async () => {
  if (!mainWindow) return null
  const result = await dialog.showOpenDialog(mainWindow, {
    filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'gif'] }],
    properties: ['openFile'],
  })
  return result.canceled ? null : result.filePaths[0]
})

ipcMain.handle('dialog:openAudioFiles', async () => {
  if (!mainWindow) return []
  const result = await dialog.showOpenDialog(mainWindow, {
    filters: [{ name: '音频/视频', extensions: ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'wma', 'opus', 'ape', 'wv', 'aiff', 'mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'] }],
    properties: ['openFile', 'multiSelections'],
  })
  return result.canceled ? [] : result.filePaths
})

ipcMain.on('mem:report', (_event, sample: any) => {
  try { reportRendererSample(sample || {}) } catch { /* ignore */ }
})

// Pull renderer-side memory from Electron's process APIs. performance.memory
// is zero on some Electron builds, so the shared monitor probes
// process.getProcessMemoryInfo()/getHeapStatistics() first and falls back to
// app.getAppMetrics() when the renderer API is unavailable.
ipcMain.handle('mem:getRendererMemory', async (event) => {
  try {
    const sender = event.sender
    if (!sender || sender.isDestroyed()) return null
    return await readRendererMemoryFromWebContents(sender)
  } catch {
    return null
  }
})

ipcMain.handle('scanner:scanFoldersWithProgress', async (event, folderPaths: string[]) => {
  const sender = event.sender
  const t0 = Date.now()
  markMain('main:scanFolders:start', `paths=${folderPaths.length}`)
  try {
    console.time('[scan] loadMetadataIndex')
    const metadataIndex = await loadTrackMetadataIndex(getLibraryDbPath())
    console.timeEnd('[scan] loadMetadataIndex')
    markMain('main:scanFolders:metaIndexLoaded', `indexSize=${metadataIndex?.size ?? 0}`)
    const result = await scanFoldersWithProgress(folderPaths, metadataIndex,
      (completed, total) => {
        if (!sender.isDestroyed()) {
          sender.send('scanner:progress', { completed, total, stage: '解析元数据...' })
          // Mark at 10/50/100% checkpoints so we can correlate memory peaks.
          if (total > 0) {
            const pct = (completed / total) * 100
            if (pct >= 100 && (sender as any)._markScan100 !== true) {
              ;(sender as any)._markScan100 = true
              markMain('main:scanFolders:progress=100%', `files=${total}`)
            } else if (pct >= 50 && (sender as any)._markScan50 !== true) {
              ;(sender as any)._markScan50 = true
              markMain('main:scanFolders:progress=50%', `files=${Math.floor(total / 2)}`)
            }
          }
        }
      },
      (stage) => {
        if (!sender.isDestroyed()) {
          sender.send('scanner:stage', stage)
        }
      }
    )
    ;(sender as any)._markScan50 = false; (sender as any)._markScan100 = false
    // Save scan results to library database (no cover data in SQLite)
    console.time('[scan] saveLibrary')
    try {
      await saveLibrarySnapshot(getLibraryDbPath(), {
        folderPaths,
        artists: result.artists,
        folderTree: result.folderTree,
        allTracks: result.allTracks,
        fileCount: result.fileCount,
        scannedAt: Date.now(),
      })
    } catch { /* ignore */ }
    console.timeEnd('[scan] saveLibrary')

    // Save covers to filesystem (blocking — must complete before renderer loads them)
    console.time('[scan] saveCovers')
    try {
      await saveCoversToFileSystem(result)
      console.timeEnd('[scan] saveCovers')
    } catch (e) {
      console.error('[scan] saveCovers failed:', e)
    }
    stripCoverPayloadForRenderer(result)
    markMain('main:scanFolders:done', `tracks=${result.fileCount} dt=${Date.now() - t0}ms`)
    try { if (typeof global.gc === 'function') global.gc() } catch { /* ignore */ }
    return result
  } catch (err: any) {
    console.error('[scan] Fatal error:', err?.message || err)
    markMain('main:scanFolders:error', err?.message || String(err))
    return { artists: [], folderTree: [], allTracks: [], fileCount: 0 }
  }
})

ipcMain.handle('library:load', async () => {
  try {
    const t0 = Date.now()
    const snap = await loadLibrarySnapshot(getLibraryDbPath())
    markMain('main:library:load', `dt=${Date.now() - t0}ms tracks=${snap?.fileCount ?? 0}`)
    return stripCoverPayloadForRenderer(snap)
  } catch {
    return null
  }
})

// Lightweight load without cover data for faster startup
ipcMain.handle('library:loadFast', async () => {
  try {
    const t0 = Date.now()
    const snap = await loadTrackListSnapshot(getLibraryDbPath())
    markMain('main:library:loadFast', `dt=${Date.now() - t0}ms tracks=${snap?.fileCount ?? 0}`)
    return snap
  } catch {
    return null
  }
})

// Load cover data for specific tracks from filesystem. Returns path-based
// references (file URLs) instead of base64 data URLs so the renderer never
// holds the JPEG bytes in its JS object heap — only the decoded ImageBitmap
// is retained for as long as the <img> is alive. Big wins for libraries of
// thousands of tracks.
ipcMain.handle('library:getCovers', async (_event, trackIds: string[]) => {
  try {
    if (!trackIds || !trackIds.length) return {}
    const result: Record<string, string> = {}
    const covers = getCoversDir()
    for (const id of trackIds) {
      const p = getTrackCoverPath(id)
      if (!p) continue
      try {
        if (fs.existsSync(p)) {
          // Build a file:// URL; on Windows this becomes file:///C:/path
          result[id] = pathToFileURL(p).href
        }
      } catch { /* skip */ }
    }
    return result
  } catch {
    return {}
  }
})

// Load cover data for specific folders from filesystem (blob storage)
ipcMain.handle('library:getFolderCovers', async (_event, folderPaths: string[]) => {
  try {
    if (!folderPaths || !folderPaths.length) return {}
    const result: Record<string, string> = {}
    for (const folderPath of folderPaths) {
      try {
        const filePath = getFolderCoverPathByMapping(folderPath)
        if (filePath && fs.existsSync(filePath)) {
          result[folderPath] = pathToFileURL(filePath).href
        }
      } catch { /* skip */ }
    }
    return result
  } catch {
    return {}
  }
})

// Load all folder cover file URLs (no base64). Renderer uses these to
// reference local <img> elements without carrying the bytes in JS.
ipcMain.handle('library:loadFolderCovers', async () => {
  try {
    const allCovers = await getAllFolderCoversFromMapAsync()
    const result: Record<string, string> = {}
    for (const [folderPath, dataUrl] of Object.entries(allCovers || {})) {
      const fp = getFolderCoverPathByMapping(folderPath)
      if (fp && fs.existsSync(fp)) {
        result[folderPath] = pathToFileURL(fp).href
      }
      // Data URL fallback for any pre-existing mapping that wasn't on disk
      else if (typeof dataUrl === 'string') {
        // keep consumer contract: drop these; UI uses fallback icon
      }
    }
    return result
  } catch {
    return {}
  }
})

// Incremental scan: only parse new/changed files, merge with existing library
ipcMain.handle('library:scanIncremental', async (event, folderPaths: string[]) => {
  const sender = event.sender
  let fullMeta: Map<string, any> | null = null
  const t0 = Date.now()
  markMain('main:scanIncremental:start', `paths=${folderPaths.length}`)
  try {
    console.time('[scan-incr] loadFullMeta')
    fullMeta = await loadFullMetadataIndex(getLibraryDbPath())
    console.timeEnd('[scan-incr] loadFullMeta')

    const result = await scanFoldersIncremental(folderPaths, fullMeta,
      (completed, total) => {
        if (!sender.isDestroyed()) sender.send('scanner:progress', { completed, total, stage: '解析元数据...' })
      },
      (stage) => {
        if (!sender.isDestroyed()) sender.send('scanner:stage', stage)
      }
    )

    // Save merged result to DB
    console.time('[scan-incr] saveLibrary')
    try {
      await saveLibrarySnapshot(getLibraryDbPath(), {
        folderPaths,
        artists: result.artists,
        folderTree: result.folderTree,
        allTracks: result.allTracks,
        fileCount: result.fileCount,
        scannedAt: Date.now(),
      })
    } catch { /* ignore */ }
    console.timeEnd('[scan-incr] saveLibrary')

    // Save covers to filesystem — only for new/changed tracks (incremental)
    console.time('[scan-incr] saveCovers')
    try {
      await saveCoversIncremental(result, result.changedPaths || new Set())
      console.timeEnd('[scan-incr] saveCovers')
    } catch (e) {
      console.error('[scan-incr] saveCovers failed:', e)
    }
    stripCoverPayloadForRenderer(result)
    markMain('main:scanIncremental:done', `tracks=${result.fileCount} dt=${Date.now() - t0}ms`)
    // Hint V8 (when --expose-gc is enabled) to reclaim worker-held buffers
    try { if (typeof global.gc === 'function') global.gc() } catch { /* ignore */ }
    return result
  } catch (err: any) {
    console.error('[scan-incr] Fatal error:', err?.message || err)
    markMain('main:scanIncremental:error', err?.message || String(err))
    return { artists: [], folderTree: [], allTracks: [], fileCount: 0 }
  } finally {
    fullMeta?.clear()
  }
})

// Remove a folder from the library without full rescan
ipcMain.handle('library:removeFolder', async (_event, folderPath: string, remainingPaths: string[]) => {
  const t0 = Date.now()
  markMain('main:removeFolder:start', `path=${folderPath}`)
  try {
    const dbPath = getLibraryDbPath()
    const snapshot = await loadLibrarySnapshot(dbPath)
    if (!snapshot) return { folderTree: [], allTracks: [], fileCount: 0, folderPaths: remainingPaths }

    const normalizedRemove = folderPath.replace(/\\/g, '/').replace(/\/+$/, '')

    // Filter tracks: remove those belonging to the removed folder
    const keptTracks = (snapshot.allTracks || []).filter((t: any) => {
      const np = (t.path || '').replace(/\\/g, '/').replace(/\/+$/, '')
      return np !== normalizedRemove && !np.startsWith(normalizedRemove + '/')
    })

    // Rebuild artists structure from kept tracks
    const artistMap = new Map<string, { path: string; albums: Map<string, any> }>()
    for (const track of keptTracks) {
      const tp = (track.path || '').replace(/\\/g, '/')
      let matchedRoot: string | null = null
      for (const rp of remainingPaths) {
        const nrp = rp.replace(/\\/g, '/').replace(/\/+$/, '')
        if (tp === nrp || tp.startsWith(nrp + '/')) { matchedRoot = nrp; break }
      }
      if (!matchedRoot) continue

      const rel = tp.slice(matchedRoot.length + 1)
      const parts = rel.split('/')
      let artistName = matchedRoot.split('/').pop() || matchedRoot
      let albumName: string
      if (parts.length >= 2) { albumName = parts[0] } else { albumName = artistName }
      if (track.artist && track.artist.trim()) artistName = track.artist.trim()

      if (!artistMap.has(artistName)) artistMap.set(artistName, { path: matchedRoot, albums: new Map() })
      const artist = artistMap.get(artistName)!
      if (!artist.albums.has(albumName)) {
        artist.albums.set(albumName, {
          name: albumName, artist: artistName,
          dirPath: parts.length >= 1 ? matchedRoot + '/' + parts[0] : matchedRoot,
          coverPath: null, coverData: null, tracks: [],
        })
      }
      artist.albums.get(albumName)!.tracks.push(track)
    }
    const artists = Array.from(artistMap.entries()).map(([name, data]) => ({
      name, path: data.path, albums: Array.from(data.albums.values()),
    }))

    // Rebuild folder tree from kept tracks
    const folderMap = new Map<string, { node: any; parent: string }>()
    for (const rp of remainingPaths) {
      const nrp = rp.replace(/\\/g, '/').replace(/\/+$/, '')
      if (!folderMap.has(nrp)) folderMap.set(nrp, { node: { path: nrp, name: nrp.split('/').pop() || nrp, children: [], tracks: [], trackCount: 0, coverData: null }, parent: '' })
    }
    for (const track of keptTracks) {
      const dir = (track.path || '').replace(/\\/g, '/').replace(/\/+$/, '')
      const dirParts = dir.split('/')
      for (let i = 1; i <= dirParts.length; i++) {
        const p = dirParts.slice(0, i).join('/')
        if (!folderMap.has(p)) {
          const parent = dirParts.slice(0, i - 1).join('/')
          folderMap.set(p, { node: { path: p, name: dirParts[i - 1] || p, children: [], tracks: [], trackCount: 0, coverData: null }, parent })
        }
      }
      const entry = folderMap.get(dir)
      if (entry) entry.node.tracks.push(track)
    }
    for (const [p, entry] of folderMap) {
      if (entry.parent && folderMap.has(entry.parent)) {
        folderMap.get(entry.parent)!.node.children.push(entry.node)
      }
    }
    for (const [, entry] of folderMap) {
      entry.node.children.sort((a: any, b: any) => a.name.localeCompare(b.name))
    }
    const roots = remainingPaths.map(rp => {
      const nrp = rp.replace(/\\/g, '/').replace(/\/+$/, '')
      return folderMap.get(nrp)?.node
    }).filter(Boolean)

    // Calculate track counts
    function calcCount(node: any): number {
      let c = node.tracks.length
      for (const child of node.children) c += calcCount(child)
      node.trackCount = c
      return c
    }
    for (const r of roots) calcCount(r)

    // Propagate covers
    const pathToCover = new Map<string, string>()
    for (const t of keptTracks) {
      const cover = t.coverData || t.albumCoverData
      if (cover) {
        const trackDir = (t.path || '').replace(/\\/g, '/').split('/').slice(0, -1).join('/')
        if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover)
      }
    }
    function propCovers(node: any) {
      const c = pathToCover.get(node.path)
      if (c && !node.coverData) node.coverData = c
      for (const child of node.children) { propCovers(child); if (!node.coverData && child.coverData) node.coverData = child.coverData }
    }
    for (const r of roots) propCovers(r)

    // Save to DB
    await saveLibrarySnapshot(dbPath, {
      folderPaths: remainingPaths,
      artists,
      folderTree: roots,
      allTracks: keptTracks,
      fileCount: keptTracks.length,
      scannedAt: Date.now(),
    })

    // Clean up cover files for removed tracks
    try {
      const coversDir = getCoversDir()
      const removedTracks = (snapshot.allTracks || []).filter((t: any) => {
        const np = (t.path || '').replace(/\\/g, '/').replace(/\/+$/, '')
        return np === normalizedRemove || np.startsWith(normalizedRemove + '/')
      })
      for (const t of removedTracks) {
        try { fs.unlinkSync(path.join(coversDir, `${t.id}.jpg`)) } catch { /* ignore */ }
      }
    } catch { /* ignore */ }

    markMain('main:removeFolder:done', `kept=${keptTracks.length} dt=${Date.now() - t0}ms`)
    return stripCoverPayloadForRenderer({ folderTree: roots, allTracks: keptTracks, fileCount: keptTracks.length, folderPaths: remainingPaths })
  } catch (err: any) {
    console.error('[removeFolder] Error:', err?.message || err)
    markMain('main:removeFolder:error', err?.message || String(err))
    return { folderTree: [], allTracks: [], fileCount: 0, folderPaths: remainingPaths }
  }
})

// Save covers from scan result to filesystem (blob storage, not SQLite)
async function saveCoversToFileSystem(result: any) {
  let trackSaved = 0, trackTotal = 0
  // Dedup: same cover data URL → save once, reuse for all track IDs sharing it
  const coverDedup = new Map<string, string[]>()
  const trackPromises: Promise<any>[] = []
  for (const track of result.allTracks || []) {
    const cd = track.coverData || track.albumCoverData
    if (cd) {
      trackTotal++
      if (coverDedup.has(cd)) {
        coverDedup.get(cd)!.push(track.id)
      } else {
        coverDedup.set(cd, [track.id])
      }
    }
  }
  for (const [dataUrl, trackIds] of coverDedup) {
    // Save the cover once, then copy for all other track IDs sharing it
    trackPromises.push((async () => {
        const firstId = trackIds[0]
        const saved = await saveTrackCover(firstId, dataUrl)
        if (saved) {
          trackSaved++
          const covDir = getCoversDir()
          // Copy the saved file for remaining track IDs
          for (let i = 1; i < trackIds.length; i++) {
            try {
              const srcPath = path.join(covDir, `${firstId}.jpg`)
              const dstPath = path.join(covDir, `${trackIds[i]}.jpg`)
              fs.copyFileSync(srcPath, dstPath)
              trackSaved++
            } catch { /* skip copy errors */ }
          }
        }
      })())
    if (trackPromises.length > 50) {
      await Promise.all(trackPromises)
      trackPromises.length = 0
    }
  }
  if (trackPromises.length) await Promise.all(trackPromises)
  console.log(`[cover] saved ${trackSaved}/${trackTotal} track covers`)

  let folderSaved = 0, folderTotal = 0
  const saveFolderCovers = async (nodes: any[]): Promise<void> => {
    for (const node of nodes) {
      if (node.coverData) {
        folderTotal++
        const saved = await saveFolderCover(node.path, node.coverData)
        if (saved) { folderSaved++; setFolderCoverMapping(node.path) }
      }
      const extCover = findExternalCoverInDir(node.path)
      if (extCover && !node.coverData) {
        folderTotal++
        const saved = await saveExternalCover(node.path, extCover)
        if (saved) { folderSaved++; setFolderCoverMapping(node.path) }
      }
      if (node.children) await saveFolderCovers(node.children)
    }
  }
  await saveFolderCovers(result.folderTree || [])
  console.log(`[cover] saved ${folderSaved}/${folderTotal} folder covers`)
}

// Incremental cover saving: only save covers for tracks whose paths are in changedPaths
async function saveCoversIncremental(result: any, changedPaths: Set<string>) {
  let trackSaved = 0, trackTotal = 0
  const coverDedup = new Map<string, string[]>()

  // Only process tracks that are in the changed set
  for (const track of result.allTracks || []) {
    const normalizedPath = (track.path || '').replace(/\\/g, '/')
    if (!changedPaths.has(normalizedPath)) continue // skip unchanged tracks
    const cd = track.coverData || track.albumCoverData
    if (cd) {
      trackTotal++
      if (coverDedup.has(cd)) {
        coverDedup.get(cd)!.push(track.id)
      } else {
        coverDedup.set(cd, [track.id])
      }
    }
  }

  const trackPromises: Promise<any>[] = []
  for (const [dataUrl, trackIds] of coverDedup) {
    trackPromises.push((async () => {
      const firstId = trackIds[0]
      const saved = await saveTrackCover(firstId, dataUrl)
      if (saved) {
        trackSaved++
        const covDir = getCoversDir()
        for (let i = 1; i < trackIds.length; i++) {
          try {
            const srcPath = path.join(covDir, `${firstId}.jpg`)
            const dstPath = path.join(covDir, `${trackIds[i]}.jpg`)
            fs.copyFileSync(srcPath, dstPath)
            trackSaved++
          } catch { /* skip */ }
        }
      }
    })())
    if (trackPromises.length > 50) {
      await Promise.all(trackPromises)
      trackPromises.length = 0
    }
  }
  if (trackPromises.length) await Promise.all(trackPromises)
  console.log(`[cover-incr] saved ${trackSaved}/${trackTotal} track covers (incremental)`)

  // Only save folder covers for folders containing changed files
  const affectedFolders = new Set<string>()
  for (const cp of changedPaths) {
    // Add all parent directories up to the root folder paths
    let dir = path.dirname(cp).replace(/\\/g, '/')
    while (dir) {
      affectedFolders.add(dir)
      const parent = path.dirname(dir).replace(/\\/g, '/')
      if (parent === dir) break
      dir = parent
    }
  }

  let folderSaved = 0, folderTotal = 0
  const saveFolderCoversIncr = async (nodes: any[]): Promise<void> => {
    for (const node of nodes) {
      const nodePath = (node.path || '').replace(/\\/g, '/')
      if (affectedFolders.has(nodePath)) {
        if (node.coverData) {
          folderTotal++
          const saved = await saveFolderCover(node.path, node.coverData)
          if (saved) { folderSaved++; setFolderCoverMapping(node.path) }
        }
        const extCover = findExternalCoverInDir(node.path)
        if (extCover && !node.coverData) {
          folderTotal++
          const saved = await saveExternalCover(node.path, extCover)
          if (saved) { folderSaved++; setFolderCoverMapping(node.path) }
        }
      }
      if (node.children) await saveFolderCoversIncr(node.children)
    }
  }
  await saveFolderCoversIncr(result.folderTree || [])
  console.log(`[cover-incr] saved ${folderSaved}/${folderTotal} folder covers (incremental)`)
}

ipcMain.handle('scanner:startWatching', async (_event, folderPaths: string[]) => {
  if (!mainWindow) return
  markMain('main:watching:start', `paths=${folderPaths.length}`)
  await startWatching(folderPaths, () => {
    mainWindow?.webContents.send('scanner:fsChanged')
  })
})

ipcMain.handle('scanner:stopWatching', async () => {
  markMain('main:watching:stop')
  stopWatching()
})

ipcMain.handle('media:getAudioDevices', async () => {
  if (!mainWindow) return []
  try {
    const devices = await mainWindow.webContents.executeJavaScript(`
      (async () => {
        try {
          const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
          stream.getTracks().forEach(track => track.stop())
          const devices = await navigator.mediaDevices.enumerateDevices()
          return devices
            .filter(d => d.kind === 'audiooutput' && d.deviceId && d.deviceId !== 'communications' && d.deviceId !== 'default')
            .map(d => ({ deviceId: d.deviceId, label: d.label || (d.deviceId === 'default' ? '系统默认输出' : '音频设备'), kind: d.kind }))
        } catch {
          try {
            const devices = await navigator.mediaDevices.enumerateDevices()
            return devices
              .filter(d => d.kind === 'audiooutput')
              .map(d => ({ deviceId: d.deviceId, label: d.label || (d.deviceId === 'default' ? '系统默认输出' : '音频设备'), kind: d.kind }))
          } catch { return [] }
        }
      })()
    `)
    const seen = new Set<string>()
    const unique = devices.filter((d: { deviceId: string }) => {
      if (seen.has(d.deviceId)) return false
      seen.add(d.deviceId)
      return true
    })
    if (!unique.some((d: { deviceId: string }) => d.deviceId === 'default')) {
      unique.unshift({ deviceId: 'default', label: '系统默认输出', kind: 'audiooutput' })
    }
    return unique
  } catch {
    return [{ deviceId: 'default', label: '系统默认输出', kind: 'audiooutput' }]
  }
})

ipcMain.handle('media:setAudioDevice', async (_event, deviceId: string) => {
  if (!mainWindow) return false
  try {
    await mainWindow.webContents.executeJavaScript(`
      (async () => {
        const audioElements = document.querySelectorAll('audio');
        const deviceId = ${JSON.stringify(deviceId)};
        for (const audio of audioElements) {
          if (typeof audio.setSinkId === 'function') {
            try { await audio.setSinkId(deviceId) } catch {}
          }
        }
        return true;
      })()
    `)
    return true
  } catch {
    return false
  }
})

ipcMain.handle('file:readAsDataURL', async (_event, filePath: string) => {
  try {
    const buffer = await fs.promises.readFile(filePath)
    const ext = path.extname(filePath).toLowerCase().replace('.', '')
    const mime = IMG_MIME[ext] || ext
    return `data:image/${mime};base64,${buffer.toString('base64')}`
  } catch {
    return null
  }
})

ipcMain.handle('file:exists', async (_event, filePath: string) => {
  return fs.existsSync(filePath)
})

ipcMain.handle('file:readTextFile', async (_event, filePath: string) => {
  try {
    const buffer = await fsp.readFile(filePath)
    try {
      const utf8 = new TextDecoder('utf-8', { fatal: true }).decode(buffer)
      return utf8
    } catch {
      try {
        const gbk = new TextDecoder('gbk').decode(buffer)
        return gbk
      } catch {
        return buffer.toString('utf-8')
      }
    }
  } catch {
    return null
  }
})

ipcMain.handle('file:listDir', async (_event, dirPath: string) => {
  try {
    const entries = await fsp.readdir(dirPath, { withFileTypes: false })
    return entries
  } catch {
    return []
  }
})

ipcMain.handle('dialog:selectBgImage', async () => {
  if (!mainWindow) return null
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openFile'],
    title: '选择背景图片',
    filters: [{ name: '图片文件', extensions: ['jpg', 'jpeg', 'png', 'bmp', 'webp', 'gif'] }],
  })
  if (result.canceled || result.filePaths.length === 0) return null
  const filePath = result.filePaths[0]
  // Copy the chosen image into userData so it survives across machines
  // (and so we have a stable file path the renderer can reference via
  // file:// without re-reading base64 into the renderer process).
  try {
    const bgPath = getBgImagePath()
    const dir = path.dirname(bgPath)
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })
    fs.copyFileSync(filePath, bgPath)
    const mtime = fs.statSync(bgPath).mtimeMs
    return { path: bgPath, mtime }
  } catch {
    return { path: filePath }
  }
})

ipcMain.handle('settings:load', async () => {
  try {
    const settingsPath = getSettingsPath()
    if (fs.existsSync(settingsPath)) {
      const data = await fsp.readFile(settingsPath, 'utf-8')
      return JSON.parse(data)
    }
  } catch { /* ignore */ }
  return {}
})

ipcMain.handle('settings:save', async (_event, settings: unknown) => {
  try {
    const settingsPath = getSettingsPath()
    const dir = path.dirname(settingsPath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf-8')
    return true
  } catch {
    return false
  }
})

ipcMain.on('settings:syncSave', (_event, settings: unknown) => {
  try {
    const settingsPath = getSettingsPath()
    const dir = path.dirname(settingsPath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf-8')
  } catch { /* ignore */ }
})



ipcMain.handle('bgImage:load', async () => {
  // Return the absolute filesystem path rather than a base64 data URL. The
  // renderer can reference the file directly via `file://` (Electron allows
  // it in webview contexts), avoiding ~10-100MB of base64-decoded ImageBitmap
  // residency in the renderer process.
  // We attach `?v=<mtime>` so re-selecting the same image forces the
  // renderer to drop its decoded Bitmap cache.
  try {
    const bgPath = getBgImagePath()
    if (fs.existsSync(bgPath)) {
      const mtime = fs.statSync(bgPath).mtimeMs
      return { path: bgPath, mtime }
    }
  } catch { /* ignore */ }
  return null
})

ipcMain.handle('bgImage:save', async (_event, dataUrl: string) => {
  try {
    const bgPath = getBgImagePath()
    const dir = path.dirname(bgPath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    const base64 = dataUrl.replace(/^data:image\/\w+;base64,/, '')
    const buffer = Buffer.from(base64, 'base64')
    fs.writeFileSync(bgPath, buffer)
    return true
  } catch {
    return false
  }
})

ipcMain.handle('bgImage:remove', async () => {
  try {
    const bgPath = getBgImagePath()
    if (fs.existsSync(bgPath)) {
      fs.unlinkSync(bgPath)
    }
    return true
  } catch {
    return false
  }
})

ipcMain.handle('clipboard:writeText', async (_event, text: string) => {
  try {
    clipboard.writeText(text)
    return true
  } catch {
    return false
  }
})

ipcMain.handle('shell:showItemInFolder', async (_event, filePath: string) => {
  try {
    shell.showItemInFolder(path.normalize(filePath))
    return true
  } catch {
    return false
  }
})

// ffmpeg.exe path helper 鈥?returns path to bundled ffmpeg.exe
function getFfmpegExe(): string {
  const isDev = !!process.env.VITE_DEV_SERVER_URL
  if (isDev) {
    // In dev, look for ffmpeg.exe in project root
    const devPath = path.join(__dirname, '../ffmpeg.exe')
    if (fs.existsSync(devPath)) return devPath
  }
  // Production: extraResources places it at resources/ffmpeg/ffmpeg.exe
  const prodPath = path.join(process.resourcesPath, 'ffmpeg', 'ffmpeg.exe')
  if (fs.existsSync(prodPath)) return prodPath
  // Fallback: same directory as exe
  const exeDir = path.dirname(app.getPath('exe'))
  const fallback = path.join(exeDir, 'ffmpeg.exe')
  if (fs.existsSync(fallback)) return fallback
  return ''
}

// Run ffmpeg command via execFile
ipcMain.handle('ffmpeg:exec', async (_event, args: string[]) => {
  const exe = getFfmpegExe()
  if (!exe) return { code: -1, error: 'ffmpeg.exe 未找到' }
  return new Promise((resolve) => {
    const child = execFile(exe, args, { timeout: 600000, maxBuffer: 100 * 1024 * 1024 }, (error, stdout, stderr) => {
      resolve({ code: error ? error.code ?? 1 : 0, stdout, stderr })
    })
  })
})


// Save converted audio file to disk
ipcMain.handle('tools:saveFile', async (_event, filePath: string, base64Data: string) => {
  try {
    const buffer = Buffer.from(base64Data, 'base64')
    fs.writeFileSync(filePath, buffer)
    return true
  } catch (e: any) {
    writeLog(`[tools:saveFile] error: ${e.message}`)
    return false
  }
})

ipcMain.handle('window:minimize', () => { mainWindow?.minimize() })
ipcMain.handle('window:maximize', () => {
  if (mainWindow?.isMaximized()) mainWindow.unmaximize()
  else mainWindow?.maximize()
})

let forceCloseFlag = false
let tray: Tray | null = null

function createTray() {
  if (tray) return
  let icon: Electron.NativeImage | null = null
  // Prefer PNG for tray (more reliable than ICO inside asar), include ICO as fallback
  const iconCandidates = [
    path.join(__dirname, '../public/icon-256.png'),
    path.join(__dirname, '../public/favicon.ico'),
    path.join(__dirname, '../../public/icon-256.png'),
    path.join(__dirname, '../../public/favicon.ico'),
    path.join(process.resourcesPath || '', 'public', 'icon-256.png'),
    path.join(process.resourcesPath || '', 'public', 'favicon.ico'),
    path.join(process.resourcesPath || '', 'icon-256.png'),
    path.join(process.resourcesPath || '', 'favicon.ico'),
  ].filter(Boolean)
  for (const p of iconCandidates) {
    try {
      if (fs.existsSync(p)) {
        // Use readFileSync + createFromBuffer — works reliably inside app.asar
        const buf = fs.readFileSync(p)
        icon = nativeImage.createFromBuffer(buf)
        if (icon && !icon.isEmpty()) {
          // Resize to 32x32 for optimal tray display on Windows
          icon = icon.resize({ width: 32, height: 32 })
          console.log('[tray] icon loaded from:', p)
          break
        }
      }
    } catch { /* continue */ }
  }
  // Fallback: create a 16x16 icon from a raw RGBA buffer (blue dot)
  if (!icon || icon.isEmpty()) {
    const size = 16
    const rgba = Buffer.alloc(size * size * 4, 0)
    const cx = Math.floor(size / 2)
    for (let y = cx - 1; y <= cx; y++) {
      for (let x = cx - 1; x <= cx; x++) {
        const i = (y * size + x) * 4
        rgba[i] = 0x40; rgba[i + 1] = 0x80; rgba[i + 2] = 0xFF; rgba[i + 3] = 0xFF
      }
    }
    icon = nativeImage.createFromBuffer(rgba, { width: size, height: size })
  }
  tray = new Tray(icon)
  tray.setToolTip('KX 音乐播放器')
  const contextMenu = Menu.buildFromTemplate([
    {
      label: '显示窗口',
      click: () => { if (mainWindow) { mainWindow.show(); mainWindow.focus() } },
    },
    { type: 'separator' },
    {
      label: '退出',
      click: () => {
        forceCloseFlag = true
        app.quit()
      },
    },
  ])
  tray.setContextMenu(contextMenu)
  tray.on('click', () => {
    if (!mainWindow) return
    if (mainWindow.isVisible()) {
      mainWindow.focus()
    } else {
      mainWindow.show()
      mainWindow.focus()
    }
  })
}

// Note: settings:syncSave is handled by ipcMain.on above (line 357)

ipcMain.handle('window:close', () => {
  if (!mainWindow) return
  // Hide to system tray instead of closing
  mainWindow.hide()
})

ipcMain.handle('window:forceClose', async () => {
  if (!mainWindow) return
  forceCloseFlag = true
  // Send close notification and give renderer time to save state
  try {
    mainWindow.webContents.send('window:beforeClose')
    await new Promise(r => setTimeout(r, 200))
  } catch { /* renderer may already be destroyed */ }
  mainWindow.close()
})

ipcMain.handle('window:isMaximized', () => mainWindow?.isMaximized() ?? false)

app.on('before-quit', async () => {
  terminateWorkerPool()
  if (mainWindow && !mainWindow.isDestroyed()) {
    try {
      mainWindow.webContents.send('window:beforeClose')
      // Give renderer a moment to save state
      await new Promise(r => setTimeout(r, 200))
    } catch { /* ignore */ }
  }
})

// --- ffmpeg WASM runs entirely in renderer, no system ffmpeg needed ---

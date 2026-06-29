import { app, BrowserWindow, ipcMain, dialog, clipboard, shell, Tray, Menu } from 'electron'
import path from 'node:path'
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import { execFile, spawn } from 'node:child_process'
import { scanFoldersWithProgress, startWatching, stopWatching } from './fileScanner'
import { loadLibrarySnapshot, loadTrackMetadataIndex, saveLibrarySnapshot } from './libraryDb'

// Shared MIME type mapping for image files
const IMG_MIME: Record<string, string> = { jpg: 'jpeg', jpeg: 'jpeg', png: 'png', bmp: 'bmp', webp: 'webp', gif: 'gif' }

// Log file helper
function writeLog(msg: string) {
  try {
    const logPath = path.join(getUserDataDir(), 'kx-player-log.txt')
    const ts = new Date().toISOString()
    fs.appendFileSync(logPath, `[${ts}] ${msg}\n`, 'utf-8')
  } catch { /* ignore */ }
}

// Disable CRL/OCSP fetching to prevent SSL handshake errors and speed up startup
app.commandLine.appendSwitch('disable-crashpad')
app.commandLine.appendSwitch('no-report-upload')
app.commandLine.appendSwitch('disable-default-apps')
app.commandLine.appendSwitch('disable-extensions')

let mainWindow: BrowserWindow | null = null

function getUserDataDir(): string {
  return app.getPath('userData')
}

function getSettingsPath(): string {
  return path.join(getUserDataDir(), 'kx-player-settings.json')
}

function getCachePath(): string {
  return path.join(getUserDataDir(), 'kx-player-cache.json')
}

function getLibraryDbPath(): string {
  return path.join(getUserDataDir(), 'kx-player-library.sqlite')
}

function getBgImagePath(): string {
  // Keep background image in userData so it survives version upgrades
  return path.join(getUserDataDir(), 'kx-player-bg.png')
}

function getDsdTempDir(): string {
  const dir = path.join(getUserDataDir(), 'dsd-temp')
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })
  return dir
}

function cleanupDsdTemp() {
  try {
    const dir = getDsdTempDir()
    if (fs.existsSync(dir)) {
      const files = fs.readdirSync(dir)
      for (const f of files) {
        try { fs.unlinkSync(path.join(dir, f)) } catch { /* ignore */ }
      }
    }
  } catch { /* ignore */ }
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
  createWindow()

  // Intercept window close to hide to tray instead (catches Alt+F4, taskbar close, etc.)
  mainWindow?.on('close', (event) => {
    if (!forceCloseFlag) {
      event.preventDefault()
      mainWindow?.hide()
      if (!tray) createTray()
    }
  })

  // Bypass certificate verification for dev server to avoid SSL handshake errors
  mainWindow?.webContents.session.setCertificateVerifyProc((request, callback) => {
    callback(0) // 0 = net::OK, accept all certificates
  })

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow()
    }
  })
})

app.on('window-all-closed', () => {
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
  const result = await dialog.showOpenDialog(mainWindow!, {
    filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'gif'] }],
    properties: ['openFile'],
  })
  return result.canceled ? null : result.filePaths[0]
})

ipcMain.handle('dialog:openAudioFiles', async () => {
  const result = await dialog.showOpenDialog(mainWindow!, {
    filters: [{ name: '音频/视频', extensions: ['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'wma', 'opus', 'ape', 'wv', 'aiff', 'dsf', 'dff', 'mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'] }],
    properties: ['openFile', 'multiSelections'],
  })
  return result.canceled ? [] : result.filePaths
})

ipcMain.handle('scanner:scanFoldersWithProgress', async (event, folderPaths: string[]) => {
  const sender = event.sender
  const metadataIndex = await loadTrackMetadataIndex(getLibraryDbPath())
  const result = await scanFoldersWithProgress(folderPaths, metadataIndex,
    (completed, total) => {
      if (!sender.isDestroyed()) {
        sender.send('scanner:progress', { completed, total, stage: '解析元数据...' })
      }
    },
    (stage) => {
      if (!sender.isDestroyed()) {
        sender.send('scanner:stage', stage)
      }
    }
  )
  // Auto-save cache after scan (fire and forget to avoid blocking UI)
  ;(async () => {
    try {
      await saveLibrarySnapshot(getLibraryDbPath(), {
        folderPaths,
        artists: result.artists,
        folderTree: result.folderTree,
        allTracks: result.allTracks,
        fileCount: result.fileCount,
        scannedAt: Date.now(),
      })

      const cachePath = getCachePath()
      const dir = path.dirname(cachePath)
      if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })
      const cache = { folderPaths, scanResult: result }
      await fsp.writeFile(cachePath, JSON.stringify(cache), 'utf-8')
    } catch { /* ignore */ }
  })()
  return result
})

ipcMain.handle('library:load', async () => {
  try {
    return await loadLibrarySnapshot(getLibraryDbPath())
  } catch {
    return null
  }
})

ipcMain.handle('scanner:startWatching', async (_event, folderPaths: string[]) => {
  if (!mainWindow) return
  await startWatching(folderPaths, () => {
    mainWindow?.webContents.send('scanner:fsChanged')
  })
})

ipcMain.handle('scanner:stopWatching', async () => {
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
        for (const audio of audioElements) {
          if (typeof audio.setSinkId === 'function') {
            try { await audio.setSinkId('${deviceId}') } catch {}
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
    const buffer = fs.readFileSync(filePath)
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

ipcMain.handle('dialog:selectBgImage', async () => {
  if (!mainWindow) return null
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openFile'],
    title: '选择背景图片',
    filters: [{ name: '图片文件', extensions: ['jpg', 'jpeg', 'png', 'bmp', 'webp', 'gif'] }],
  })
  if (result.canceled || result.filePaths.length === 0) return null
  const filePath = result.filePaths[0]
  try {
    const buffer = await fs.promises.readFile(filePath)
    const ext = path.extname(filePath).toLowerCase().replace('.', '')
    const mime = IMG_MIME[ext] || ext
    return { dataUrl: `data:image/${mime};base64,${buffer.toString('base64')}`, path: filePath }
  } catch {
    return null
  }
})

ipcMain.handle('settings:load', async () => {
  try {
    const settingsPath = getSettingsPath()
    if (fs.existsSync(settingsPath)) {
      const data = fs.readFileSync(settingsPath, 'utf-8')
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

ipcMain.handle('cache:load', async () => {
  try {
    const cachePath = getCachePath()
    if (fs.existsSync(cachePath)) {
      const data = fs.readFileSync(cachePath, 'utf-8')
      return JSON.parse(data)
    }
  } catch { /* ignore */ }
  return null
})

ipcMain.handle('cache:save', async (_event, cache: unknown) => {
  try {
    const cachePath = getCachePath()
    const dir = path.dirname(cachePath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(cachePath, JSON.stringify(cache), 'utf-8')
    return true
  } catch {
    return false
  }
})

ipcMain.handle('bgImage:load', async () => {
  try {
    const bgPath = getBgImagePath()
    if (fs.existsSync(bgPath)) {
      const buffer = await fs.promises.readFile(bgPath)
      const ext = path.extname(bgPath).toLowerCase().replace('.', '')
      const mime = IMG_MIME[ext] || ext
      return { dataUrl: `data:image/${mime};base64,${buffer.toString('base64')}`, path: bgPath }
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
  const isDev = process.env.VITE_DEV_SERVER_URL ? true : false
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
    // Send progress updates via IPC events
    child.on('exit', () => { /* handled by callback */ })
  })
})


ipcMain.handle('dsd:decodePcm', async (_event, filePath: string) => {
  const exe = getFfmpegExe()
  if (!exe) return { ok: false, error: 'ffmpeg.exe 未找到' }

  return new Promise((resolve) => {
    const child = spawn(exe, [
      '-v', 'error',
      '-i', filePath,
      '-f', 's16le',
      '-acodec', 'pcm_s16le',
      '-ar', '44100',
      '-ac', '2',
      '-',
    ], { stdio: ['ignore', 'pipe', 'pipe'] })

    const stdoutChunks: Buffer[] = []
    const stderrChunks: Buffer[] = []
    let settled = false

    const finish = (payload: unknown) => {
      if (settled) return
      settled = true
      resolve(payload)
    }

    child.stdout.on('data', (chunk: Buffer) => stdoutChunks.push(Buffer.from(chunk)))
    child.stderr.on('data', (chunk: Buffer) => stderrChunks.push(Buffer.from(chunk)))
    child.on('error', (error) => finish({ ok: false, error: error.message }))
    child.on('close', (code) => {
      if (code !== 0) {
        const stderr = Buffer.concat(stderrChunks).toString('utf-8').trim()
        finish({ ok: false, error: stderr || `ffmpeg exited with code ${code}` })
        return
      }
      const pcmBuffer = Buffer.concat(stdoutChunks)
      finish({ ok: true, sampleRate: 44100, channels: 2, bitsPerSample: 16, pcmBase64: pcmBuffer.toString('base64') })
    })

    setTimeout(() => {
      try { child.kill() } catch { }
      finish({ ok: false, error: 'DSD decode timeout' })
    }, 600000)
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
  // Use favicon as tray icon
  const iconPath = path.join(__dirname, '../public/favicon.ico')
  if (!fs.existsSync(iconPath)) return
  tray = new Tray(iconPath)
  tray.setToolTip('KX 音乐播放器')
  const contextMenu = Menu.buildFromTemplate([
    {
      label: '显示窗口',
      click: () => { if (mainWindow) { mainWindow.show(); mainWindow.focus() } },
    },
    { type: 'separator' },
    {
      label: '退出',
      click: () => { app.quit() },
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

function syncSaveSettingsToFile(settings: unknown) {
  try {
    const settingsPath = getSettingsPath()
    const dir = path.dirname(settingsPath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf-8')
  } catch { /* ignore */ }
}

ipcMain.handle('settings:syncSave', async (_event, settings: unknown) => {
  syncSaveSettingsToFile(settings)
})

async function ensureSettingsSaved() {
  // Try to load existing settings from file - if it exists and is recent, trust it
  try {
    const settingsPath = getSettingsPath()
    if (fs.existsSync(settingsPath)) {
      const stat = fs.statSync(settingsPath)
      const age = Date.now() - stat.mtimeMs
      if (age < 10000) return // saved within last 10s, trust it
    }
  } catch { /* ignore */ }
}

ipcMain.handle('window:close', () => {
  if (!mainWindow) return
  // Hide to system tray instead of closing
  mainWindow.hide()
  if (!tray) createTray()
})

ipcMain.handle('window:forceClose', () => {
  if (!mainWindow) return
  forceCloseFlag = true
  mainWindow.webContents.send('window:beforeClose')
  try {
    const settingsPath = getSettingsPath()
    if (fs.existsSync(settingsPath)) {
      const data = fs.readFileSync(settingsPath, 'utf-8')
      fs.writeFileSync(settingsPath, data, 'utf-8')
    }
  } catch { /* ignore */ }
  mainWindow.close()
})

ipcMain.handle('window:isMaximized', () => mainWindow?.isMaximized() ?? false)

ipcMain.handle('dsd:getTempPath', () => getDsdTempDir())

app.on('before-quit', () => {
  cleanupDsdTemp()
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send('window:beforeClose')
  }
})

// --- ffmpeg WASM runs entirely in renderer, no system ffmpeg needed ---


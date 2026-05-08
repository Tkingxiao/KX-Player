import { app, BrowserWindow, ipcMain, dialog } from 'electron'
import path from 'node:path'
import fs from 'node:fs'
import os from 'node:os'
import { execFile } from 'node:child_process'
import { scanFoldersWithProgress, startWatching, stopWatching } from './fileScanner'

// Shared MIME type mapping for image files
const IMG_MIME: Record<string, string> = { jpg: 'jpeg', jpeg: 'jpeg', png: 'png', bmp: 'bmp', webp: 'webp', gif: 'gif' }

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
  return await scanFoldersWithProgress(folderPaths,
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

ipcMain.handle('window:minimize', () => { mainWindow?.minimize() })
ipcMain.handle('window:maximize', () => {
  if (mainWindow?.isMaximized()) mainWindow.unmaximize()
  else mainWindow?.maximize()
})

let forceCloseFlag = false

ipcMain.handle('window:close', () => {
  if (!mainWindow) return
  mainWindow.webContents.send('window:beforeClose')
  setTimeout(() => {
    if (!forceCloseFlag) {
      forceCloseFlag = true
      mainWindow?.close()
    }
  }, 300)
})

ipcMain.handle('window:forceClose', () => {
  forceCloseFlag = true
  mainWindow?.close()
})

ipcMain.handle('window:isMaximized', () => mainWindow?.isMaximized() ?? false)

app.on('before-quit', () => {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send('window:beforeClose')
  }
})

function findFfmpeg(): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile('ffmpeg', ['-version'], (err) => {
      if (err) {
        const ffPath = path.join(process.resourcesPath || path.dirname(app.getPath('exe')), 'ffmpeg.exe')
        if (fs.existsSync(ffPath)) { resolve(ffPath); return; }
        reject(new Error('未找到ffmpeg，请安装ffmpeg或将ffmpeg.exe放在应用目录'))
      } else {
        resolve('ffmpeg')
      }
    })
  })
}

ipcMain.handle('tools:extractAudio', async (_event, filePath: string, targetFormat?: string) => {
  try {
    const ffmpeg = await findFfmpeg()
    const dir = path.dirname(filePath)
    const baseName = path.basename(filePath, path.extname(filePath))
    const fmt = targetFormat || 'mp3'
    const outPath = path.join(dir, baseName + '.' + fmt)
    const codecMap: Record<string, string[]> = {
      mp3: ['-vn', '-acodec', 'libmp3lame', '-q:a', '2'],
      flac: ['-vn', '-acodec', 'flac'],
      wav: ['-vn', '-acodec', 'pcm_s16le'],
      ogg: ['-vn', '-acodec', 'libvorbis', '-q:a', '4'],
      aac: ['-vn', '-acodec', 'aac', '-b:a', '192k'],
    }
    const args = codecMap[fmt] || ['-vn', '-acodec', 'libmp3lame', '-q:a', '2']
    return new Promise((resolve, reject) => {
      execFile(ffmpeg, ['-i', filePath, ...args, '-y', outPath], (err) => {
        if (err) reject(new Error(err.message))
        else resolve(outPath)
      })
    })
  } catch (e: any) {
    throw new Error(e.message)
  }
})

ipcMain.handle('tools:convertAudio', async (_event, filePath: string, targetFormat: string) => {
  try {
    const ffmpeg = await findFfmpeg()
    const dir = path.dirname(filePath)
    const baseName = path.basename(filePath, path.extname(filePath))
    const outPath = path.join(dir, baseName + '.' + targetFormat)
    const codecMap: Record<string, string[]> = {
      mp3: ['-acodec', 'libmp3lame', '-q:a', '2'],
      flac: ['-acodec', 'flac'],
      wav: ['-acodec', 'pcm_s16le'],
      ogg: ['-acodec', 'libvorbis', '-q:a', '4'],
      m4a: ['-acodec', 'aac', '-b:a', '192k'],
      aac: ['-acodec', 'aac', '-b:a', '192k'],
    }
    const args = codecMap[targetFormat] || ['-acodec', 'copy']
    return new Promise((resolve, reject) => {
      execFile(ffmpeg, ['-i', filePath, ...args, '-y', outPath], (err) => {
        if (err) reject(new Error(err.message))
        else resolve(outPath)
      })
    })
  } catch (e: any) {
    throw new Error(e.message)
  }
})

ipcMain.handle('media:decodeDSD', async (_event, filePath: string) => {
  try {
    const cachedDir = path.join(os.tmpdir(), 'kxplayer-dsd-cache')
    if (!fs.existsSync(cachedDir)) fs.mkdirSync(cachedDir, { recursive: true })
    const hash = filePath.split('').reduce((a, c) => ((a << 5) - a + c.charCodeAt(0)) | 0, 0).toString(16)
    const outPath = path.join(cachedDir, hash + '.wav')
    if (fs.existsSync(outPath)) return outPath
    const ffmpeg = await findFfmpeg()
    return new Promise((resolve, reject) => {
      execFile(ffmpeg, [
        '-i', filePath,
        '-acodec', 'pcm_s16le',
        '-ar', '44100',
        '-ac', '2',
        '-y', outPath
      ], (err) => {
        if (err) reject(new Error(err.message))
        else resolve(outPath)
      })
    })
  } catch (e: any) {
    throw new Error(e.message)
  }
})

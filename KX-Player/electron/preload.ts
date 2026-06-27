import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('electronAPI', {
  openFolder: () => ipcRenderer.invoke('dialog:openFolder'),
  openImageFile: () => ipcRenderer.invoke('dialog:openImageFile'),
  openAudioFiles: () => ipcRenderer.invoke('dialog:openAudioFiles'),
  scanFoldersWithProgress: (paths: string[]) => ipcRenderer.invoke('scanner:scanFoldersWithProgress', paths),
  loadLibrary: () => ipcRenderer.invoke('library:load'),
  readAsDataURL: (filePath: string) => ipcRenderer.invoke('file:readAsDataURL', filePath),
  readTextFile: (filePath: string) => ipcRenderer.invoke('file:readTextFile', filePath),
  fileExists: (filePath: string) => ipcRenderer.invoke('file:exists', filePath),
  loadSettings: () => ipcRenderer.invoke('settings:load'),
  saveSettings: (settings: unknown) => ipcRenderer.invoke('settings:save', settings),
  syncSaveSettings: (settings: unknown) => ipcRenderer.send('settings:syncSave', settings),
  getAudioDevices: () => ipcRenderer.invoke('media:getAudioDevices'),
  setAudioDevice: (deviceId: string) => ipcRenderer.invoke('media:setAudioDevice', deviceId),
  selectBgImage: () => ipcRenderer.invoke('dialog:selectBgImage'),
  minimizeWindow: () => ipcRenderer.invoke('window:minimize'),
  maximizeWindow: () => ipcRenderer.invoke('window:maximize'),
  closeWindow: () => ipcRenderer.invoke('window:close'),
  forceCloseWindow: () => ipcRenderer.invoke('window:forceClose'),
  isMaximized: () => ipcRenderer.invoke('window:isMaximized'),
  onMaximizeChange: (callback: (maximized: boolean) => void) => {
    ipcRenderer.on('window:maximizeChange', (_event, maximized) => callback(maximized))
  },
  onScanProgress: (callback: (data: any) => void) => {
    ipcRenderer.on('scanner:progress', (_event, data) => callback(data))
  },
  onScannerProgress: (callback: (data: any) => void) => {
    ipcRenderer.on('scanner:progress', (_event, data) => callback(data))
  },
  removeScanProgressListener: () => {
    ipcRenderer.removeAllListeners('scanner:progress')
  },
  onBeforeClose: (callback: () => void) => {
    ipcRenderer.on('window:beforeClose', () => callback())
  },
  startWatching: (folderPaths: string[]) => ipcRenderer.invoke('scanner:startWatching', folderPaths),
  stopWatching: () => ipcRenderer.invoke('scanner:stopWatching'),
  onFsChanged: (callback: () => void) => {
    const handler = () => callback()
    ipcRenderer.on('scanner:fsChanged', handler)
    return () => ipcRenderer.removeListener('scanner:fsChanged', handler)
  },
  loadCache: () => ipcRenderer.invoke('cache:load'),
  saveCache: (cache: unknown) => ipcRenderer.invoke('cache:save', cache),
  loadBgImage: () => ipcRenderer.invoke('bgImage:load'),
  saveBgImage: (dataUrl: string) => ipcRenderer.invoke('bgImage:save', dataUrl),
  removeBgImage: () => ipcRenderer.invoke('bgImage:remove'),
  toolsSaveFile: (filePath: string, base64Data: string) => ipcRenderer.invoke('tools:saveFile', filePath, base64Data),
  ffmpegExec: (args: string[]) => ipcRenderer.invoke('ffmpeg:exec', args),
  dsdDecodePcm: (filePath: string) => ipcRenderer.invoke('dsd:decodePcm', filePath),
  dsdTempPath: () => ipcRenderer.invoke('dsd:getTempPath'),
  clipboardWriteText: (text: string) => ipcRenderer.invoke('clipboard:writeText', text),
  showItemInFolder: (filePath: string) => ipcRenderer.invoke('shell:showItemInFolder', filePath),
})

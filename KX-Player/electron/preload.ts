import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('electronAPI', {
  openFolder: () => ipcRenderer.invoke('dialog:openFolder'),
  openImageFile: () => ipcRenderer.invoke('dialog:openImageFile'),
  openAudioFiles: () => ipcRenderer.invoke('dialog:openAudioFiles'),
  scanFoldersWithProgress: (paths: string[]) => ipcRenderer.invoke('scanner:scanFoldersWithProgress', paths),
  scanFoldersIncremental: (paths: string[]) => ipcRenderer.invoke('library:scanIncremental', paths),
  removeFolder: (folderPath: string, remainingPaths: string[]) => ipcRenderer.invoke('library:removeFolder', folderPath, remainingPaths),
  loadLibrary: () => ipcRenderer.invoke('library:load'),
  loadLibraryFast: () => ipcRenderer.invoke('library:loadFast'),
  getTrackCovers: (trackIds: string[]) => ipcRenderer.invoke('library:getCovers', trackIds),
  loadFolderCovers: () => ipcRenderer.invoke('library:loadFolderCovers'),
  readAsDataURL: (filePath: string) => ipcRenderer.invoke('file:readAsDataURL', filePath),
  readTextFile: (filePath: string) => ipcRenderer.invoke('file:readTextFile', filePath),
  fileExists: (filePath: string) => ipcRenderer.invoke('file:exists', filePath),
  listDir: (dirPath: string) => ipcRenderer.invoke('file:listDir', dirPath),
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
    const handler = (_event: any, maximized: boolean) => callback(maximized)
    ipcRenderer.on('window:maximizeChange', handler)
    return () => ipcRenderer.removeListener('window:maximizeChange', handler)
  },
  onScanProgress: (callback: (data: any) => void) => {
    const handler = (_event: any, data: any) => callback(data)
    ipcRenderer.on('scanner:progress', handler)
    return () => ipcRenderer.removeListener('scanner:progress', handler)
  },
  onScannerStage: (callback: (data: string) => void) => {
    const handler = (_event: any, data: string) => callback(data)
    ipcRenderer.on('scanner:stage', handler)
    return () => ipcRenderer.removeListener('scanner:stage', handler)
  },
  removeScanProgressListener: () => {
    ipcRenderer.removeAllListeners('scanner:progress')
    ipcRenderer.removeAllListeners('scanner:stage')
  },
  onBeforeClose: (callback: () => void) => {
    const handler = () => callback()
    ipcRenderer.on('window:beforeClose', handler)
    return () => ipcRenderer.removeListener('window:beforeClose', handler)
  },
  startWatching: (folderPaths: string[]) => ipcRenderer.invoke('scanner:startWatching', folderPaths),
  stopWatching: () => ipcRenderer.invoke('scanner:stopWatching'),
  onFsChanged: (callback: () => void) => {
    const handler = () => callback()
    ipcRenderer.on('scanner:fsChanged', handler)
    return () => ipcRenderer.removeListener('scanner:fsChanged', handler)
  },
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

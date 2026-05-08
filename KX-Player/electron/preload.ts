import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('electronAPI', {
  openFolder: () => ipcRenderer.invoke('dialog:openFolder'),
  openImageFile: () => ipcRenderer.invoke('dialog:openImageFile'),
  openAudioFiles: () => ipcRenderer.invoke('dialog:openAudioFiles'),
  scanFoldersWithProgress: (paths: string[]) => ipcRenderer.invoke('scanner:scanFoldersWithProgress', paths),
  readAsDataURL: (filePath: string) => ipcRenderer.invoke('file:readAsDataURL', filePath),
  readTextFile: (filePath: string) => ipcRenderer.invoke('file:readTextFile', filePath),
  fileExists: (filePath: string) => ipcRenderer.invoke('file:exists', filePath),
  loadSettings: () => ipcRenderer.invoke('settings:load'),
  saveSettings: (settings: unknown) => ipcRenderer.invoke('settings:save', settings),
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
  removeScanProgressListener: () => {
    ipcRenderer.removeAllListeners('scanner:progress')
  },
  onBeforeClose: (callback: () => void) => {
    ipcRenderer.on('window:beforeClose', () => callback())
  },
  extractAudio: (filePath: string, format?: string) => ipcRenderer.invoke('tools:extractAudio', filePath, format),
  convertAudio: (filePath: string, format: string) => ipcRenderer.invoke('tools:convertAudio', filePath, format),
  decodeDSD: (filePath: string) => ipcRenderer.invoke('media:decodeDSD', filePath),
  startWatching: (folderPaths: string[]) => ipcRenderer.invoke('scanner:startWatching', folderPaths),
  stopWatching: () => ipcRenderer.invoke('scanner:stopWatching'),
  onFsChanged: (callback: () => void) => {
    const handler = () => callback()
    ipcRenderer.on('scanner:fsChanged', handler)
    return () => ipcRenderer.removeListener('scanner:fsChanged', handler)
  },
})

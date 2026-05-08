const e=window.electronAPI||{}

export const api={
  openFolder:()=>e.openFolder?e.openFolder():null,
  openImageFile:async()=>{
    if(!e.openImageFile)return null
    const p=await e.openImageFile()
    if(!p)return null
    if(e.readAsDataURL){const d=await e.readAsDataURL(p);return{data:d,path:p}}
    return{data:null,path:p}
  },
  openAudioFiles:()=>e.openAudioFiles?e.openAudioFiles():[],
  scanFoldersWithProgress:paths=>e.scanFoldersWithProgress?e.scanFoldersWithProgress(paths):null,
  readAsDataURL:p=>e.readAsDataURL?e.readAsDataURL(p):null,
  readTextFile:p=>e.readTextFile?e.readTextFile(p):null,
  fileExists:p=>e.fileExists?e.fileExists(p):false,
  loadSettings:()=>e.loadSettings?e.loadSettings():{},
  saveSettings:s=>{if(e.saveSettings)e.saveSettings(s)},
  getAudioDevices:()=>e.getAudioDevices?e.getAudioDevices():[],
  setAudioDevice:id=>{if(e.setAudioDevice)e.setAudioDevice(id)},
  selectBgImage:()=>e.selectBgImage?e.selectBgImage():null,
  minimize:()=>{if(e.minimizeWindow)e.minimizeWindow()},
  maximize:()=>{if(e.maximizeWindow)e.maximizeWindow()},
  close:()=>{if(e.closeWindow)e.closeWindow()},
  forceClose:()=>{if(e.forceCloseWindow)e.forceCloseWindow()},
  isMaximized:()=>e.isMaximized?e.isMaximized():false,
  onMaximized:cb=>{if(e.onMaximizeChange)e.onMaximizeChange(m=>{if(m)cb()})},
  onUnmaximized:cb=>{if(e.onMaximizeChange)e.onMaximizeChange(m=>{if(!m)cb()})},
  onScannerProgress:cb=>{if(e.onScanProgress)e.onScanProgress(cb)},
  removeScanProgressListener:()=>{if(e.removeScanProgressListener)e.removeScanProgressListener()},
  onBeforeClose:cb=>{if(e.onBeforeClose)e.onBeforeClose(cb)},
  extractAudio:(p,f)=>e.extractAudio?e.extractAudio(p,f):null,
  convertAudio:(p,f)=>e.convertAudio?e.convertAudio(p,f):null,
  decodeDSD:p=>e.decodeDSD?e.decodeDSD(p):null,
  startWatching:(paths,cb)=>{
    if(e.startWatching)e.startWatching(paths)
    if(e.onFsChanged&&cb)e.onFsChanged(cb)
  },
  stopWatching:()=>{if(e.stopWatching)e.stopWatching()},
  onFsChanged:cb=>{if(e.onFsChanged)return e.onFsChanged(cb)},
}

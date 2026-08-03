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
  scanFoldersIncremental:paths=>e.scanFoldersIncremental?e.scanFoldersIncremental(paths):null,
  removeFolder:(fp,rp)=>e.removeFolder?e.removeFolder(fp,rp):null,
  loadLibrary:()=>e.loadLibrary?e.loadLibrary():null,
  loadLibraryFast:()=>e.loadLibraryFast?e.loadLibraryFast():null,
  getTrackCovers:ids=>e.getTrackCovers?e.getTrackCovers(ids):{},
  getFolderCovers:paths=>e.getFolderCovers?e.getFolderCovers(paths):{},
  loadFolderCovers:()=>e.loadFolderCovers?e.loadFolderCovers():{},
  readAsDataURL:p=>e.readAsDataURL?e.readAsDataURL(p):null,
  readTextFile:p=>e.readTextFile?e.readTextFile(p):null,
  fileExists:p=>e.fileExists?e.fileExists(p):false,
  listDir:p=>e.listDir?e.listDir(p):Promise.resolve([]),
  loadSettings:()=>e.loadSettings?e.loadSettings():{},
  saveSettings:s=>e.saveSettings?e.saveSettings(s):Promise.resolve(),
  syncSaveSettings:s=>{if(e.syncSaveSettings)e.syncSaveSettings(s)},
  getAudioDevices:()=>e.getAudioDevices?e.getAudioDevices():[],
  setAudioDevice:id=>e.setAudioDevice?e.setAudioDevice(id):Promise.resolve(false),
  selectBgImage:()=>e.selectBgImage?e.selectBgImage():null,
  minimize:()=>{if(e.minimizeWindow)e.minimizeWindow()},
  maximize:()=>{if(e.maximizeWindow)e.maximizeWindow()},
  close:()=>{if(e.closeWindow)e.closeWindow()},
  forceClose:()=>{if(e.forceCloseWindow)e.forceCloseWindow()},
  isMaximized:()=>e.isMaximized?e.isMaximized():false,
  onMaximized:cb=>{if(e.onMaximizeChange)return e.onMaximizeChange(m=>{if(m)cb()})},
  onUnmaximized:cb=>{if(e.onMaximizeChange)return e.onMaximizeChange(m=>{if(!m)cb()})},
  onScanProgress:cb=>e.onScanProgress&&e.onScanProgress(cb),
  onScannerProgress:cb=>e.onScanProgress&&e.onScanProgress(cb),
  onScannerStage:cb=>e.onScannerStage&&e.onScannerStage(cb),
  removeScanProgressListener:()=>e.removeScanProgressListener&&e.removeScanProgressListener(),
  onBeforeClose:cb=>{if(e.onBeforeClose)e.onBeforeClose(cb)},
  toolsSaveFile:(f,b)=>e.toolsSaveFile?e.toolsSaveFile(f,b):Promise.resolve(false),
  ffmpegExec:a=>e.ffmpegExec?e.ffmpegExec(a):Promise.resolve({code:-1,error:'not available'}),
  startWatching:paths=>e.startWatching?e.startWatching(paths):Promise.resolve(),
  stopWatching:()=>{if(e.stopWatching)e.stopWatching()},
  onFsChanged:cb=>{if(e.onFsChanged)return e.onFsChanged(cb)},

  loadBgImage:()=>e.loadBgImage?e.loadBgImage():null,
  saveBgImage:d=>e.saveBgImage?e.saveBgImage(d):Promise.resolve(false),
  removeBgImage:()=>e.removeBgImage?e.removeBgImage():Promise.resolve(),
  clipboardWriteText:text=>e.clipboardWriteText?e.clipboardWriteText(text):Promise.resolve(false),
  showItemInFolder:filePath=>e.showItemInFolder?e.showItemInFolder(filePath):Promise.resolve(false),
}

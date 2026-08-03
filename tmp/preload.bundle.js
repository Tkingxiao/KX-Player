// KX-Player/electron/preload.ts
var import_electron = require("electron");
import_electron.contextBridge.exposeInMainWorld("electronAPI", {
  openFolder: () => import_electron.ipcRenderer.invoke("dialog:openFolder"),
  openImageFile: () => import_electron.ipcRenderer.invoke("dialog:openImageFile"),
  openAudioFiles: () => import_electron.ipcRenderer.invoke("dialog:openAudioFiles"),
  scanFoldersWithProgress: (paths) => import_electron.ipcRenderer.invoke("scanner:scanFoldersWithProgress", paths),
  scanFoldersIncremental: (paths) => import_electron.ipcRenderer.invoke("library:scanIncremental", paths),
  removeFolder: (folderPath, remainingPaths) => import_electron.ipcRenderer.invoke("library:removeFolder", folderPath, remainingPaths),
  loadLibrary: () => import_electron.ipcRenderer.invoke("library:load"),
  loadLibraryFast: () => import_electron.ipcRenderer.invoke("library:loadFast"),
  getTrackCovers: (trackIds) => import_electron.ipcRenderer.invoke("library:getCovers", trackIds),
  getFolderCovers: (folderPaths) => import_electron.ipcRenderer.invoke("library:getFolderCovers", folderPaths),
  loadFolderCovers: () => import_electron.ipcRenderer.invoke("library:loadFolderCovers"),
  readAsDataURL: (filePath) => import_electron.ipcRenderer.invoke("file:readAsDataURL", filePath),
  readTextFile: (filePath) => import_electron.ipcRenderer.invoke("file:readTextFile", filePath),
  fileExists: (filePath) => import_electron.ipcRenderer.invoke("file:exists", filePath),
  listDir: (dirPath) => import_electron.ipcRenderer.invoke("file:listDir", dirPath),
  loadSettings: () => import_electron.ipcRenderer.invoke("settings:load"),
  saveSettings: (settings) => import_electron.ipcRenderer.invoke("settings:save", settings),
  syncSaveSettings: (settings) => import_electron.ipcRenderer.send("settings:syncSave", settings),
  getAudioDevices: () => import_electron.ipcRenderer.invoke("media:getAudioDevices"),
  setAudioDevice: (deviceId) => import_electron.ipcRenderer.invoke("media:setAudioDevice", deviceId),
  selectBgImage: () => import_electron.ipcRenderer.invoke("dialog:selectBgImage"),
  minimizeWindow: () => import_electron.ipcRenderer.invoke("window:minimize"),
  maximizeWindow: () => import_electron.ipcRenderer.invoke("window:maximize"),
  closeWindow: () => import_electron.ipcRenderer.invoke("window:close"),
  forceCloseWindow: () => import_electron.ipcRenderer.invoke("window:forceClose"),
  isMaximized: () => import_electron.ipcRenderer.invoke("window:isMaximized"),
  onMaximizeChange: (callback) => {
    const handler = (_event, maximized) => callback(maximized);
    import_electron.ipcRenderer.on("window:maximizeChange", handler);
    return () => import_electron.ipcRenderer.removeListener("window:maximizeChange", handler);
  },
  onScanProgress: (callback) => {
    const handler = (_event, data) => callback(data);
    import_electron.ipcRenderer.on("scanner:progress", handler);
    return () => import_electron.ipcRenderer.removeListener("scanner:progress", handler);
  },
  onScannerStage: (callback) => {
    const handler = (_event, data) => callback(data);
    import_electron.ipcRenderer.on("scanner:stage", handler);
    return () => import_electron.ipcRenderer.removeListener("scanner:stage", handler);
  },
  removeScanProgressListener: () => {
    import_electron.ipcRenderer.removeAllListeners("scanner:progress");
    import_electron.ipcRenderer.removeAllListeners("scanner:stage");
  },
  onBeforeClose: (callback) => {
    const handler = () => callback();
    import_electron.ipcRenderer.on("window:beforeClose", handler);
    return () => import_electron.ipcRenderer.removeListener("window:beforeClose", handler);
  },
  startWatching: (folderPaths) => import_electron.ipcRenderer.invoke("scanner:startWatching", folderPaths),
  stopWatching: () => import_electron.ipcRenderer.invoke("scanner:stopWatching"),
  onFsChanged: (callback) => {
    const handler = () => callback();
    import_electron.ipcRenderer.on("scanner:fsChanged", handler);
    return () => import_electron.ipcRenderer.removeListener("scanner:fsChanged", handler);
  },
  loadBgImage: () => import_electron.ipcRenderer.invoke("bgImage:load"),
  saveBgImage: (dataUrl) => import_electron.ipcRenderer.invoke("bgImage:save", dataUrl),
  removeBgImage: () => import_electron.ipcRenderer.invoke("bgImage:remove"),
  toolsSaveFile: (filePath, base64Data) => import_electron.ipcRenderer.invoke("tools:saveFile", filePath, base64Data),
  ffmpegExec: (args) => import_electron.ipcRenderer.invoke("ffmpeg:exec", args),
  clipboardWriteText: (text) => import_electron.ipcRenderer.invoke("clipboard:writeText", text),
  showItemInFolder: (filePath) => import_electron.ipcRenderer.invoke("shell:showItemInFolder", filePath),
  reportMemSample: (sample) => {
    try {
      import_electron.ipcRenderer.send("mem:report", sample);
    } catch {
    }
  }
});

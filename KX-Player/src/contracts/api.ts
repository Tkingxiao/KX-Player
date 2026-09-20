/** 契约层：渲染层与 Rust 后端之间的 DTO 与 API 类型（端到端类型化）。 */

export interface Track {
  id: string
  name: string
  path: string
  duration: number
  artist: string
  album: string
  format: string
  isVideo: boolean
  coverPath: string | null
  coverData: string | null
  lyricsPath: string | null
  fileMtime: number
  fileSize: number
  metaTitle: string | null
  metaArtist: string | null
  genre: string | null
  bitrate: number | null
  sampleRate: number | null
  loudnessLufs?: number | null
  albumCoverData?: string | null
}

export interface FolderNode {
  name: string
  path: string
  children: FolderNode[]
  tracks: Track[]
  trackCount: number
  coverData: string | null
}

export interface ScanResult {
  folderPaths: string[]
  artists: unknown[]
  folderTree: FolderNode[]
  allTracks: Track[]
  fileCount: number
}

export interface DirEntry {
  name: string
  isFile: boolean
  isDirectory: boolean
}

export interface AudioDevice {
  deviceId: string
  label: string
}

/** 字幕轨（mpv/libass）；id<=0 表示「关闭字幕」 */
export interface SubtitleTrack {
  id: number
  title: string
  lang: string
  selected: boolean
}

export interface FfmpegInfo {
  available: boolean
  path: string | null
  version: string | null
}

export interface ConvertItem {
  id: number
  path: string
  outPath: string
  kind: 'convert' | 'extract'
  format: string
}

export interface ConvertProgress {
  taskId: number
  itemId: number
  state: 'running' | 'done' | 'failed' | 'cancelled' | 'queue-finished'
  error?: string
}

export interface SubStyle {
  font?: string
  fontSize?: number
  color?: string
  borderColor?: string
  borderSize?: number
  shadowOffset?: number
  pos?: number
}

export interface PlayProgress {
  trackId: string
  positionMs: number
  completed: boolean
  playCount: number
  playedAt: number
  lastSpeed: number | null
}

export interface Bookmark {
  id: string
  trackId: string
  atMs: number
  label: string
  createdAt: number
}

export interface BgImageData {
  path: string
  mtime?: number
}

export interface FavItem {
  id: string
  name: string
  trackIds: string[]
  isDefault?: boolean
  cover?: string
}

/** mpv 播放状态镜像（Rust player:state 事件载荷） */
export interface MpvPlayerState {
  playing: boolean
  position: number
  duration: number
  speed: number
  volume: number
  muted: boolean
  trackPath: string | null
  isVideo: boolean
  videoActive: boolean
}

/** AppApi 形状（src/bridge/ipc.ts 经 Tauri invoke/listen 实现） */
export interface AppApi {
  openFolder: () => Promise<string[] | null>
  openImageFile: () => Promise<string | null>
  openAudioFiles: () => Promise<string[]>
  scanFoldersWithProgress: (paths: string[]) => Promise<ScanResult | null>
  scanFoldersIncremental: (paths: string[]) => Promise<ScanResult | null>
  /** 启动静默增量同步：无变更时不触发扫描态 UI */
  startupSync: (paths: string[]) => Promise<ScanResult | null>
  removeFolder: (folderPath: string, remainingPaths: string[]) => Promise<ScanResult | null>
  loadLibrary: () => Promise<ScanResult | null>
  loadLibraryFast: () => Promise<ScanResult | null>
  getTrackCovers: (trackIds: string[]) => Promise<Record<string, string>>
  getFolderCovers: (paths: string[]) => Promise<Record<string, string>>
  loadFolderCovers: () => Promise<Record<string, string>>
  readAsDataURL: (p: string) => Promise<string | null>
  readTextFile: (p: string) => Promise<string | null>
  fileExists: (p: string) => Promise<boolean>
  listDir: (p: string) => Promise<DirEntry[]>
  loadSettings: () => Promise<Record<string, unknown>>
  saveSettings: (s: Record<string, unknown>) => Promise<boolean>
  syncSaveSettings: (s: Record<string, unknown>) => void
  selectBgImage: () => Promise<BgImageData | null>
  minimizeWindow: () => Promise<void>
  maximizeWindow: () => Promise<void>
  closeWindow: () => Promise<void>
  forceCloseWindow: () => Promise<void>
  isMaximized: () => Promise<boolean>
  /** 短别名（TitleBar 用） */
  maximize: () => Promise<void>
  minimize: () => Promise<void>
  close: () => Promise<void>
  onMaximizeChange: (cb: (max: boolean) => void) => () => void
  onScanProgress: (cb: (d: { completed: number; total: number }) => void) => () => void
  onScannerStage: (cb: (stage: string) => void) => () => void
  onBeforeClose: (cb: () => void) => () => void
  startWatching: (paths: string[]) => Promise<boolean>
  stopWatching: () => Promise<boolean>
  onFsChanged: (cb: () => void) => () => void
  loadBgImage: () => Promise<BgImageData | null>
  saveBgImage: (dataUrl: string) => Promise<boolean>
  removeBgImage: () => Promise<boolean>
  toolsSaveFile: (p: string, b64: string) => Promise<boolean>
  ffmpegExec: (args: string[]) => Promise<{ code: number; stdout?: string; stderr?: string }>
  /** ffmpeg 可用性探测（转换页据此降级显示） */
  ffmpegProbe: () => Promise<FfmpegInfo>
  /** 启动转换队列（任务 + convert:progress 事件），返回 taskId */
  convertRun: (items: ConvertItem[]) => Promise<number>
  convertCancel: (taskId: number) => Promise<boolean>
  aiChat: (payload: {
    baseURL: string
    apiKey: string
    model: string
    messages: { role: string; content: string }[]
    temperature?: number
  }) => Promise<{ ok: boolean; content: string; error?: string }>
  aiPing: (payload: { baseURL: string; apiKey: string; model: string }) => Promise<{ ok: boolean; message: string }>
  aiListModels: (payload: { baseURL: string; apiKey: string }) => Promise<{ ok: boolean; models: string[]; error?: string }>
  renameDir: (oldPath: string, newPath: string) => Promise<{ ok: boolean; error?: string }>
  clipboardWriteText: (t: string) => Promise<boolean>
  showItemInFolder: (p: string) => Promise<boolean>
  getProgressAll: () => Promise<PlayProgress[]>
  setProgress: (trackId: string, positionMs: number, completed: boolean | null, lastSpeed: number | null) => Promise<boolean>
  markTrackCompleted: (trackId: string, completed: boolean) => Promise<boolean>
  clearTrackProgress: (trackId: string) => Promise<boolean>
  listBookmarks: (trackId: string) => Promise<Bookmark[]>
  addBookmark: (trackId: string, atMs: number, label: string) => Promise<Bookmark | null>
  renameBookmark: (id: string, label: string) => Promise<boolean>
  removeBookmark: (id: string) => Promise<boolean>

  // ── 播放内核（mpv）──
  playerPlay: (opts: {
    trackPath: string
    resumeMs?: number
    vol?: number
    muted?: boolean
    speed?: number
    videoActive?: boolean
  }) => Promise<boolean>
  playerToggle: (paused: boolean) => Promise<boolean>
  playerSeek: (sec: number) => Promise<boolean>
  playerSetVolume: (vol: number) => Promise<boolean>
  playerSetMuted: (muted: boolean) => Promise<boolean>
  playerSetSpeed: (speed: number) => Promise<boolean>
  playerStop: () => Promise<boolean>
  playerSetVideoEnabled: (enabled: boolean) => Promise<boolean>
  playerSetStageRect: (rect: { x: number; y: number; w: number; h: number; visible: boolean }) => Promise<boolean>
  /** 播放状态快照（pip 窗口冷启动时拉取一次） */
  playerGetState: () => Promise<MpvPlayerState>
  /** 打开独立悬浮窗（物理像素）；成功返回 true */
  pipOpen: (x: number, y: number, w: number, h: number) => Promise<boolean>
  /** 关闭/隐藏独立悬浮窗，mpv 覆盖窗口挂回主窗口 */
  pipClose: () => Promise<boolean>
  /** 悬浮窗已关闭（由 pip 窗口或主窗口触发） */
  onPipClosed: (cb: () => void) => () => void
  /** 钉住状态变更（pip 窗口内切换时同步到主窗口） */
  onPipPinned: (cb: (pinned: boolean) => void) => () => void
  /** pip 窗口内设置钉住状态 */
  pipSetPinned: (pinned: boolean) => Promise<boolean>
  playerListDevices: () => Promise<AudioDevice[]>
  playerSetDevice: (deviceId: string) => Promise<boolean>
  playerSubtitleTracks: () => Promise<SubtitleTrack[]>
  playerSetSubtitleTrack: (id: number) => Promise<boolean>
  playerSetSubtitleVisible: (visible: boolean) => Promise<boolean>
  playerSetSubtitleDelay: (sec: number) => Promise<boolean>
  playerSetSubStyle: (style: {
    font?: string
    fontSize?: number
    color?: string
    borderColor?: string
    borderSize?: number
    shadowOffset?: number
    pos?: number
  }) => Promise<boolean>
  playerApplyLoudnessGain: (trackLufs: number | null, targetLufs: number) => Promise<boolean>
  /** 后台响度分析（任务 + loudness:progress 事件）：[trackId, path] 列表 */
  analyzeLoudness: (tracks: [string, string][]) => Promise<boolean>
  cancelLoudness: () => Promise<boolean>
  toggleFullscreen: () => Promise<boolean>

  // ── 分类与标签（P0-11/12/13）──
  taxonomyListCategories: () => Promise<Category[]>
  taxonomyCreateCategory: (parentId: number | null, name: string) => Promise<Category | null>
  taxonomyRenameCategory: (id: number, name: string) => Promise<boolean>
  taxonomyDeleteCategory: (id: number) => Promise<boolean>
  taxonomyMoveCategory: (id: number, parentId: number | null, sortIndex: number) => Promise<boolean>
  taxonomyAssignCategory: (categoryId: number, trackIds: string[]) => Promise<boolean>
  taxonomyUnassignCategory: (categoryId: number, trackIds: string[]) => Promise<boolean>
  taxonomyListTags: () => Promise<Tag[]>
  taxonomyUpsertTag: (name: string, color?: string | null, kind?: string) => Promise<Tag | null>
  taxonomyRenameTag: (id: number, name: string) => Promise<boolean>
  taxonomyDeleteTag: (id: number) => Promise<boolean>
  taxonomyTagTracks: (tagIds: number[], trackIds: string[], mode?: 'add' | 'replace') => Promise<boolean>
  taxonomyUntagTracks: (tagId: number, trackIds: string[]) => Promise<boolean>
  taxonomySuggestTags: (trackIds: string[]) => Promise<{ trackId: string; suggestions: TagSuggestion[] }[]>
  taxonomyApplySuggestedTags: (accepted: { trackId: string; tagName: string }[]) => Promise<boolean>
  taxonomyTrackTags: (trackIds: string[]) => Promise<Record<string, Tag[]>>
  taxonomyAllTrackTags: () => Promise<{ trackId: string; tagId: number }[]>
  taxonomyAllCategoryItems: () => Promise<{ categoryId: number; trackId: string }[]>
}

export interface Category {
  id: number
  parentId: number | null
  name: string
  icon: string | null
  color: string | null
  sortIndex: number
  trackCount: number
  children: Category[]
}

export interface Tag {
  id: number
  name: string
  color: string | null
  kind: string
  useCount: number
  autoGenerated: boolean
}

export interface TagSuggestion {
  name: string
  score: number
}

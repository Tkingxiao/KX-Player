/**
 * 契约层 · `AppApi`：渲染层可见的对象接口形状，由 `src/bridge/ipc.ts` 实现。
 *
 * DTO 在 `./dto`，命令名与参数键在 `./commands`，事件名与载荷在 `./events`。
 * 这里只描述**方法形状**（含前端侧的适配结果，如 `{ ok, error }` 包装），
 * 与 Rust 命令签名的差异由 `bridge/ipc.ts` 在边界收口。
 */
import type {
  AiChatResult,
  AiExportResult,
  AiImportResult,
  AiJobOptions,
  AiModelsResult,
  AiPingResult,
  AudioDevice,
  BgImageData,
  Bookmark,
  Category,
  ConvertItem,
  DirEntry,
  ExecResult,
  FfmpegInfo,
  MpvPlayerState,
  PlayProgress,
  RenamePreview,
  ScanKind,
  ScanResult,
  SubStyle,
  SubtitleScanItem,
  SubtitleTrack,
  Tag,
  TagSuggestion,
} from './dto'

export * from './dto'

/** AppApi 形状（src/bridge/ipc.ts 经 Tauri invoke/listen 实现） */
export interface AppApi {
  openFolder: () => Promise<string[] | null>
  openImageFile: () => Promise<string | null>
  openAudioFiles: () => Promise<string[]>
  /** AI-16 导入用的多选文本框；取消 = 空数组 */
  openTextFiles: () => Promise<string[]>
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
  /** 启动自检（P0-64）降级提示：取一次即清空，用于首屏 Toast */
  startupWarnings: () => Promise<string[]>
  toolsSaveFile: (p: string, b64: string) => Promise<boolean>
  ffmpegExec: (args: string[]) => Promise<ExecResult>
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
  }) => Promise<AiChatResult>
  aiPing: (payload: { baseURL: string; apiKey: string; model: string }) => Promise<AiPingResult>
  aiListModels: (payload: { baseURL: string; apiKey: string }) => Promise<AiModelsResult>
  /** 字幕翻译任务（B3）：返回 taskId，进度与结果走 ai:progress 事件，可 aiTaskCancel */
  aiTranslateStart: (opts: AiJobOptions & { paths: string[] }) => Promise<number>
  /** 纯文本批量翻译任务（文件夹名译名）：结果在收尾事件的 translated 里 */
  aiTextsStart: (opts: AiJobOptions & { texts: string[] }) => Promise<number>
  aiTaskCancel: (taskId: number) => Promise<boolean>
  /**
   * AI-16 导出：把选中字幕写成 `### 序号 ###` 批次文件（保存框由用户定位置）。
   * 不配 Provider 也能走通的那条兜底路径；保存框被取消会 reject `CANCELLED`。
   */
  aiExportPrompts: (paths: string[]) => Promise<AiExportResult>
  /** AI-16 导入：`apply = false` 只做校验与差异预览，一点不碰磁盘 */
  aiImportTranslations: (paths: string[], text: string, apply: boolean) => Promise<AiImportResult>
  /** Rust 侧递归扫描：一次调用替代旧的 api.listDir 逐层递归 */
  subtitleScanDir: (dir: string, kind: ScanKind, maxDepth?: number, limit?: number) => Promise<SubtitleScanItem[]>
  renameDir: (oldPath: string, newPath: string) => Promise<{ ok: boolean; error?: string }>
  /**
   * AI-6/7/10 改名预览（`04 §7.8` 只读的那一半）：一次列出每个根目录的下一层，
   * 给出档位分布 + 「当前名 → 规范化后」清单 + 冲突表。**不写盘、不改一个文件**。
   */
  renamePreview: (paths: string[]) => Promise<RenamePreview>
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
  }) => Promise<void>
  playerToggle: (paused: boolean) => Promise<void>
  playerSeek: (sec: number) => Promise<void>
  playerSetVolume: (vol: number) => Promise<void>
  playerSetMuted: (muted: boolean) => Promise<void>
  playerSetSpeed: (speed: number) => Promise<void>
  playerStop: () => Promise<void>
  playerSetVideoEnabled: (enabled: boolean) => Promise<void>
  playerSetStageRect: (rect: { x: number; y: number; w: number; h: number; visible: boolean }) => Promise<boolean>
  /** 播放状态快照（pip 窗口冷启动时拉取一次） */
  playerGetState: () => Promise<MpvPlayerState>
  /** 打开独立悬浮窗（物理像素）；成功返回 true */
  pipOpen: (x: number, y: number, w: number, h: number) => Promise<boolean>
  /** PipRoot.onMounted 握手：WebView 已就绪，触发 attach_overlay + pip:shown */
  pipReady: () => Promise<boolean>
  /** 关闭/隐藏独立悬浮窗，mpv 覆盖窗口挂回主窗口 */
  pipClose: () => Promise<boolean>
  /** 从悬浮窗还原：关浮窗 + 主窗口到前台 + 广播 pip:restored */
  pipRestore: () => Promise<boolean>
  /** 悬浮窗已关闭（由 pip 窗口或主窗口触发） */
  onPipClosed: (cb: () => void) => () => void
  /** 悬浮窗还原到舞台（双击浮窗画面 / Esc 触发） */
  onPipRestored: (cb: () => void) => () => void
  /** 钉住状态变更（pip 窗口内切换时同步到主窗口） */
  onPipPinned: (cb: (pinned: boolean) => void) => () => void
  /** pip 窗口内设置钉住状态 */
  pipSetPinned: (pinned: boolean) => Promise<boolean>
  playerListDevices: () => Promise<AudioDevice[]>
  playerSetDevice: (deviceId: string) => Promise<boolean>
  playerSubtitleTracks: () => Promise<SubtitleTrack[]>
  playerSetSubtitleTrack: (id: number) => Promise<void>
  playerSetSubtitleVisible: (visible: boolean) => Promise<void>
  playerSetSubtitleDelay: (sec: number) => Promise<void>
  playerSetSubStyle: (style: SubStyle) => Promise<void>
  playerApplyLoudnessGain: (trackLufs: number | null, targetLufs: number) => Promise<void>
  /** 后台响度分析（任务 + loudness:progress 事件）：[trackId, path] 列表 */
  analyzeLoudness: (tracks: [string, string][]) => Promise<boolean>
  cancelLoudness: () => Promise<boolean>
  toggleFullscreen: () => Promise<boolean>

  // ── 分类与标签（P0-11/12/13）──
  // `Promise<void>` = 成功即 resolve(null)，失败一律 reject（`02 §6.1`）；
  // 调用方用 try/catch 分流，不能拿返回值当成功标志——Rust 侧没有回传布尔值。
  taxonomyListCategories: () => Promise<Category[]>
  taxonomyCreateCategory: (parentId: number | null, name: string) => Promise<Category>
  taxonomyRenameCategory: (id: number, name: string) => Promise<void>
  taxonomyDeleteCategory: (id: number) => Promise<void>
  taxonomyMoveCategory: (id: number, parentId: number | null, sortIndex: number) => Promise<void>
  /** 受影响的行数：0 表示本来就是这个状态，不是失败 */
  taxonomyAssignCategory: (categoryId: number, trackIds: string[]) => Promise<number>
  taxonomyUnassignCategory: (categoryId: number, trackIds: string[]) => Promise<number>
  taxonomyListTags: () => Promise<Tag[]>
  taxonomyUpsertTag: (name: string, color?: string | null, kind?: string) => Promise<Tag>
  taxonomyRenameTag: (id: number, name: string) => Promise<void>
  taxonomyDeleteTag: (id: number) => Promise<void>
  taxonomyTagTracks: (tagIds: number[], trackIds: string[], mode?: 'add' | 'replace') => Promise<number>
  taxonomyUntagTracks: (tagId: number, trackIds: string[]) => Promise<number>
  taxonomySuggestTags: (trackIds: string[]) => Promise<{ trackId: string; suggestions: TagSuggestion[] }[]>
  taxonomyApplySuggestedTags: (accepted: { trackId: string; tagName: string }[]) => Promise<number>
  taxonomyTrackTags: (trackIds: string[]) => Promise<Record<string, Tag[]>>
  taxonomyAllTrackTags: () => Promise<{ trackId: string; tagId: number }[]>
  taxonomyAllCategoryItems: () => Promise<{ categoryId: number; trackId: string }[]>
}

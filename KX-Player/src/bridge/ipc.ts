/** 唯一 IPC 出口：所有 Rust 后端调用都经此模块（Tauri invoke + asset URL 转换）。 */
import { invoke as tauriInvoke, convertFileSrc } from '@tauri-apps/api/core'
import type { AppApi, ScanResult, Track, FolderNode, Bookmark, BgImageData, PlayProgress, AudioDevice, DirEntry, Category, Tag, TagSuggestion, SubtitleTrack, FfmpegInfo, ConvertItem, MpvPlayerState } from '@/contracts/api'
import type { CommandSpecs } from '@/contracts/commands'
import { toAppError, errText } from '@/contracts/result'

/** 命令名 → 参数对象 / resolve 值，两者都以 `contracts/commands.ts` 为源。 */
type ArgsOf<K extends keyof CommandSpecs> = CommandSpecs[K]['args']
type ResolveOf<K extends keyof CommandSpecs> = CommandSpecs[K]['returns']

/**
 * 唯一的 invoke 出口：命令名、参数键名、resolve 类型全部由 `CommandSpecs` 约束，
 * 写错键名或多传一个键在编译期就红，不再只靠 `check:ipc` 的静态对账。
 * Rust 侧 `Err(IpcError)` 经 Tauri 是 reject：这里只把载荷归一化成 `AppError`，不改 reject 语义。
 */
async function invoke<K extends keyof CommandSpecs>(cmd: K, args?: ArgsOf<K>): Promise<ResolveOf<K>> {
  try {
    return (await tauriInvoke(cmd, args)) as ResolveOf<K>
  } catch (e) {
    throw toAppError(e)
  }
}

/** 本地文件路径 → asset 协议 URL（封面/背景图等 <img> 引用） */
export function assetUrl(p: string | null | undefined): string | null {
  if (!p) return null
  if (p.startsWith('http://') || p.startsWith('https://') || p.startsWith('data:') || p.startsWith('blob:')) return p
  return convertFileSrc(p)
}

function coverOf(t: Track | null | undefined): Track | null {
  if (!t) return null
  // 路径型封面字段统一转 asset URL（Rust 侧只回传路径，不传字节）
  t.coverData = assetUrl(t.coverData ?? t.coverPath)
  t.albumCoverData = assetUrl(t.albumCoverData)
  return t
}

function decorateScan(result: ScanResult | null): ScanResult | null {
  if (!result) return null
  for (const t of result.allTracks ?? []) coverOf(t)
  for (const artist of result.artists ?? []) {
    const a = artist as { albums?: { tracks?: Track[]; coverData?: string | null }[] }
    for (const al of a.albums ?? []) {
      al.coverData = assetUrl(al.coverData)
      for (const t of al.tracks ?? []) coverOf(t)
    }
  }
  const walk = (nodes: FolderNode[]) => {
    for (const n of nodes) {
      n.coverData = assetUrl(n.coverData)
      for (const t of n.tracks ?? []) coverOf(t)
      walk(n.children ?? [])
    }
  }
  walk(result.folderTree ?? [])
  return result
}

function decorateBookmarks(list: Bookmark[]): Bookmark[] {
  return list ?? []
}

function decorateBg(b: BgImageData | null): BgImageData | null {
  // 保留原始绝对路径（设置持久化需要真实路径）；展示 URL 由 App.vue 用 assetUrl() 派生
  return b
}

export const api: AppApi = {
  openFolder: () => invoke('open_folder'),
  openImageFile: () => invoke('open_image_file'),
  openAudioFiles: () => invoke('open_audio_files'),
  openTextFiles: () => invoke('open_text_files'),
  scanFoldersWithProgress: (p) => invoke('scan_folders', { paths: p }).then(decorateScan),
  scanFoldersIncremental: (p) => invoke('scan_folders_incremental', { paths: p }).then(decorateScan),
  startupSync: (p) => invoke('startup_sync', { paths: p }).then(decorateScan),
  removeFolder: (fp, rp) => invoke('remove_folder', { folderPath: fp, remainingPaths: rp }).then(decorateScan),
  loadLibrary: () => invoke('load_library').then(decorateScan),
  loadLibraryFast: () => invoke('load_library_fast').then(decorateScan),
  getTrackCovers: (ids) =>
    invoke('get_track_covers', { trackIds: ids }).then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  getFolderCovers: (paths) =>
    invoke('get_folder_covers', { folderPaths: paths }).then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  loadFolderCovers: () =>
    invoke('load_folder_covers').then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  readAsDataURL: (p) => invoke('read_as_data_url', { path: p }),
  readTextFile: (p) => invoke('read_text_file', { path: p }),
  fileExists: (p) => invoke('file_exists', { path: p }),
  listDir: (p) => invoke('list_dir', { path: p }),
  loadSettings: () => invoke('load_settings'),
  saveSettings: (s) => invoke('save_settings', { settings: s }),
  syncSaveSettings: (s) => {
    void invoke('save_settings', { settings: s }).catch(() => false)
  },
  selectBgImage: () => invoke('select_bg_image').then(decorateBg),
  minimizeWindow: () => invoke('minimize_window'),
  maximizeWindow: () => invoke('maximize_window'),
  closeWindow: () => invoke('close_window'),
  forceCloseWindow: () => invoke('force_close_window'),
  isMaximized: () => invoke('is_maximized'),
  maximize: () => invoke('maximize_window'),
  minimize: () => invoke('minimize_window'),
  close: () => invoke('close_window'),
  onMaximizeChange: (cb) => onEvent('window:maximizeChange', cb),
  onScanProgress: (cb) => onEvent('scanner:progress', cb),
  onScannerStage: (cb) => onEvent('scanner:stage', cb),
  onBeforeClose: (cb) => onEvent('window:beforeClose', cb),
  startWatching: (p) => invoke('start_watching', { paths: p }).catch(() => false),
  stopWatching: () => invoke('stop_watching').catch(() => false),
  onFsChanged: (cb) => onEvent('scanner:fsChanged', cb),
  loadBgImage: () => invoke('load_bg_image').then(decorateBg),
  saveBgImage: (d) => invoke('save_bg_image', { dataUrl: d }),
  removeBgImage: () => invoke('remove_bg_image'),
  startupWarnings: () => invoke('startup_warnings'),
  toolsSaveFile: (p, b) => invoke('tools_save_file', { path: p, base64Data: b }),
  ffmpegExec: (a) => invoke('ffmpeg_exec', { args: a }),
  ffmpegProbe: () =>
    invoke('ffmpeg_probe').catch(() => ({ available: false, path: null, version: null })),
  convertRun: (items: ConvertItem[]) => invoke('convert_run', { items }).catch(() => 0),
  convertCancel: (taskId: number) => invoke('convert_cancel', { taskId }).catch(() => false),
  // 注意：Tauri 会把 Rust 参数 base_url 归一化为 baseUrl（serde camelCase 同理），
  // 传 baseURL 会因缺参数直接反序列化失败——这里统一在边界做映射。
  aiChat: (payload) =>
    invoke('ai_chat', {
      payload: {
        baseUrl: payload.baseURL,
        apiKey: payload.apiKey,
        model: payload.model,
        messages: payload.messages,
        temperature: payload.temperature,
      },
    }),
  aiPing: (payload) =>
    invoke('ai_ping', {
      payload: { baseUrl: payload.baseURL, apiKey: payload.apiKey, model: payload.model },
    }),
  aiListModels: (payload) =>
    invoke('ai_list_models', {
      baseUrl: payload.baseURL,
      apiKey: payload.apiKey,
    }),
  // ── AI 批量任务（B3）──
  // 与 aiChat 的区别：这里只「启动」任务，逐块翻译在 Rust 里跑，进度走 ai:progress 事件。
  // 前端不再拿 await 串起整条链路，所以关页面不影响任务、也才有地方能取消它。
  aiTranslateStart: ({ baseURL, paths, ...opts }) =>
    invoke('ai_translate_start', { spec: { baseUrl: baseURL, paths, ...opts } }),
  aiTextsStart: ({ baseURL, texts, ...opts }) =>
    invoke('ai_texts_start', { spec: { baseUrl: baseURL, texts, ...opts } }),
  aiTaskCancel: (taskId: number) => invoke('ai_task_cancel', { taskId }).catch(() => false),
  // AI-16 半自动模式：导出走保存框（取消 = reject CANCELLED，调用方别当故障报），
  // 导入的译文文本由前端读好再传（编码探测在 read_text_file 那份，不重复实现一遍）
  aiExportPrompts: (paths) => invoke('ai_export_prompts', { paths }),
  aiImportTranslations: (paths, text, apply) => invoke('ai_import_translations', { paths, text, apply }),
  subtitleScanDir: (dir, kind, maxDepth, limit) =>
    invoke('subtitle_scan_dir', { dir, kind, maxDepth, limit }),
  // 改名主干：预览与落地是**分开的命令**，落地那三条的输入只有「用户勾的目录」或「库里的批次号」——
  // 前端手里永远只是一份预览，从不回传「要改哪些」（04 §7.7）。
  // 不入 settings、不进历史队列（04 §7.8 的「同一条流水线」是 S2 的事）
  renamePreview: (paths) => invoke('rename_preview', { paths }),
  renameApply: (paths) => invoke('rename_apply', { paths }),
  // 生成译名：只写库不改盘，所以它排在 apply 前面 —— 用户先按这一条拿到名字，再按 apply 落地。
  // 接入参数前端叫 baseURL（`AiJobOptions` 的口径），线形状是 baseUrl，映射收在边界这一层。
  renameSuggestNames: (paths, baseURL, apiKey, model, modelKind) =>
    invoke('rename_suggest_names', { paths, baseUrl: baseURL, apiKey, model, modelKind }),
  // 拒绝一个译名 = 只往拒绝表写一行；生效的地方在后端每次查库时复核，不在这里记账
  renameRejectSuggestion: (path, suggestion) => invoke('rename_reject_suggestion', { path, suggestion }),
  renameRollback: (batchId) => invoke('rename_rollback', { batchId }),
  renameRedo: (batchId) => invoke('rename_redo', { batchId }),
  renameBatches: (limit) => invoke('rename_batches', { limit }),
  aiCacheFolderSuggestions: () =>
    invoke('ai_cache_folder_suggestions').catch(() => ({})),
  aiCacheRecordFolder: (path, suggestion) =>
    invoke('ai_cache_record_folder', { path, suggestion }).catch(() => false),
  renameDir: async (oldPath, newPath) => {
    try {
      await invoke('rename_dir', { oldPath, newPath })
      return { ok: true }
    } catch (e) {
      return { ok: false, error: errText(e) }
    }
  },
  clipboardWriteText: (t) => invoke('clipboard_write_text', { text: t }),
  showItemInFolder: (p) => invoke('show_item_in_folder', { path: p }),
  getProgressAll: () => invoke('get_progress_all'),
  setProgress: (id, ms, c, sp) =>
    invoke('set_progress', { trackId: id, positionMs: Math.round(ms), completed: c, lastSpeed: sp }).catch(() => false),
  markTrackCompleted: (id, c) => invoke('mark_track_completed', { trackId: id, completed: c }).catch(() => false),
  clearTrackProgress: (id) => invoke('clear_track_progress', { trackId: id }).catch(() => false),
  listBookmarks: (id) => invoke('list_bookmarks', { trackId: id }).then(decorateBookmarks).catch(() => []),
  addBookmark: async (id, ms, label) => {
    try {
      return await invoke('add_bookmark', { trackId: id, atMs: Math.round(ms), label })
    } catch {
      return null
    }
  },
  renameBookmark: (id, label) => invoke('rename_bookmark', { id, label }).catch(() => false),
  removeBookmark: (id) => invoke('remove_bookmark', { id }).catch(() => false),

  // ── 播放内核 ──
  // 这些命令的 Rust 侧一律 resolve `()`，成功/失败只由 reject 表达；
  // 带 `.catch(() => {})` 的是「尽力而为」的调节（音量/字幕），失败不提示，UI 以 player:state 事件收敛。
  playerPlay: (opts) =>
    invoke('player_play', {
      trackPath: opts.trackPath,
      resumeMs: opts.resumeMs ?? 0,
      vol: opts.vol ?? 0.85,
      muted: opts.muted ?? false,
      speed: opts.speed ?? 1,
      videoActive: opts.videoActive ?? false,
    }),
  playerToggle: (paused) => invoke('player_toggle', { paused }),
  playerSeek: (sec) => invoke('player_seek', { sec }),
  playerSetVolume: (vol) => invoke('player_set_volume', { vol }),
  playerSetMuted: (muted) => invoke('player_set_muted', { muted }),
  playerSetSpeed: (speed) => invoke('player_set_speed', { speed }),
  playerStop: () => invoke('player_stop'),
  playerSetVideoEnabled: (enabled) => invoke('player_set_video_enabled', { enabled }),
  playerSetStageRect: (rect) =>
    invoke('player_set_stage_rect', {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      w: Math.round(rect.w),
      h: Math.round(rect.h),
      visible: rect.visible,
    }).catch(() => false),
  playerGetState: () => invoke('player_get_state').catch(() => ({
    playing: false, position: 0, duration: 0, speed: 1, volume: 85, muted: false,
    trackPath: null, videoActive: false, videoW: 0, videoH: 0,
  })),
  pipOpen: (x, y, w, h) =>
    invoke('pip_open', { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) }).catch(() => false),
  pipReady: () => invoke('pip_ready').catch(() => false),
  pipClose: () => invoke('pip_close').catch(() => false),
  pipRestore: () => invoke('pip_restore').catch(() => false),
  onPipClosed: (cb) => onEvent('pip:closed', cb),
  onPipRestored: (cb) => onEvent('pip:restored', cb),
  onPipPinned: (cb) => onEvent<boolean>('pip:pinned', cb),
  pipSetPinned: (pinned) => invoke('pip_set_pinned', { pinned }).catch(() => false),
  playerListDevices: () => invoke('player_list_devices').catch(() => []),
  playerSetDevice: (deviceId) => invoke('player_set_device', { deviceId }).catch(() => false),
  toggleFullscreen: () => invoke('toggle_fullscreen').catch(() => false),

  // ── 字幕（mpv + libass）──
  playerSubtitleTracks: () => invoke('player_subtitle_tracks').catch(() => []),
  playerSetSubtitleTrack: (id) => invoke('player_set_subtitle_track', { id }).catch(() => {}),
  playerSetSubtitleVisible: (visible) =>
    invoke('player_set_subtitle_visible', { visible }).catch(() => {}),
  playerSetSubtitleDelay: (sec) => invoke('player_set_subtitle_delay', { sec }).catch(() => {}),
  // 整对象透传，**不再逐字段白名单**：白名单曾把 backColor / backOpacity /
  // assOverride 三个新字段悄悄吃掉，后端实现齐全却永远收不到（P0-24/P0-25 一度因此
  // 完全失效）。Rust 侧 SubStyle 全是 Option，多余的 undefined 键在 JSON 里本就会消失。
  playerSetSubStyle: (style) => invoke('player_set_sub_style', { style }).catch(() => {}),
  playerApplyLoudnessGain: (trackLufs, targetLufs) =>
    invoke('player_apply_loudness_gain', {
      trackLufs,
      targetLufs,
    }).catch(() => {}),

  // ── 后台响度分析（任务 + loudness:progress 事件）──
  analyzeLoudness: (tracks: [string, string][]) =>
    invoke('analyze_loudness', { tracks }).catch(() => false),
  cancelLoudness: () => invoke('cancel_loudness').catch(() => false),

  // ── 分类与标签 ──
  // 写命令不在这里吞错：Rust 的 `Err(IpcError)` 直接 reject 到 store，由 store 弹带真实原因的 Toast。
  taxonomyListCategories: () => invoke('taxonomy_list_categories').catch(() => []),
  taxonomyCreateCategory: (parentId, name) =>
    invoke('taxonomy_create_category', { parentId, name }),
  taxonomyRenameCategory: (id, name) => invoke('taxonomy_rename_category', { id, name }),
  taxonomyDeleteCategory: (id) => invoke('taxonomy_delete_category', { id }),
  taxonomyMoveCategory: (id, parentId, sortIndex) =>
    invoke('taxonomy_move_category', { id, parentId, sortIndex }),
  taxonomyAssignCategory: (categoryId, trackIds) =>
    invoke('taxonomy_assign_category', { categoryId, trackIds }),
  taxonomyUnassignCategory: (categoryId, trackIds) =>
    invoke('taxonomy_unassign_category', { categoryId, trackIds }),
  taxonomyListTags: () => invoke('taxonomy_list_tags').catch(() => []),
  taxonomyUpsertTag: (name, color, kind) =>
    invoke('taxonomy_upsert_tag', { name, color: color ?? null, kind: kind ?? null }),
  taxonomyRenameTag: (id, name) => invoke('taxonomy_rename_tag', { id, name }),
  taxonomyDeleteTag: (id) => invoke('taxonomy_delete_tag', { id }),
  taxonomyTagTracks: (tagIds, trackIds, mode) =>
    invoke('taxonomy_tag_tracks', { tagIds, trackIds, mode: mode ?? 'add' }),
  taxonomyUntagTracks: (tagId, trackIds) => invoke('taxonomy_untag_tracks', { tagId, trackIds }),
  taxonomySuggestTags: (trackIds) =>
    invoke('taxonomy_suggest_tags', { trackIds })
      .then((list) => (list ?? []).map(([trackId, suggestions]) => ({ trackId, suggestions })))
      .catch(() => []),
  taxonomyApplySuggestedTags: (accepted) =>
    invoke('taxonomy_apply_suggested_tags', {
      accepted: accepted.map((a) => [a.trackId, a.tagName]),
    }),
  taxonomyTrackTags: (trackIds) => invoke('taxonomy_track_tags', { trackIds }).catch(() => ({})),
  taxonomyAllTrackTags: () =>
    invoke('taxonomy_all_track_tags')
      .then((rows) => (rows ?? []).map(([trackId, tagId]) => ({ trackId, tagId })))
      .catch(() => []),
  taxonomyAllCategoryItems: () =>
    invoke('taxonomy_all_category_items')
      .then((rows) => (rows ?? []).map(([categoryId, trackId]) => ({ categoryId, trackId })))
      .catch(() => []),
}

// ── 事件层（唯一的 listen 出口）─────────────────────────────
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export function onEvent<T>(name: string, cb: (data: T) => void): () => void {
  let un: UnlistenFn | null = null
  let disposed = false
  void listen<T>(name, (e) => cb(e.payload))
    .then((u) => {
      if (disposed) u()
      else un = u
    })
    .catch(() => {})
  return () => {
    disposed = true
    if (un) un()
    un = null
  }
}

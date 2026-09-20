/** 唯一 IPC 出口：所有 Rust 后端调用都经此模块（Tauri invoke + asset URL 转换）。 */
import { invoke as tauriInvoke, convertFileSrc } from '@tauri-apps/api/core'
import type { AppApi, ScanResult, Track, FolderNode, Bookmark, BgImageData, PlayProgress, AudioDevice, DirEntry, Category, Tag, TagSuggestion, SubtitleTrack, FfmpegInfo, ConvertItem, MpvPlayerState } from '@/contracts/api'

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args)
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
  openFolder: () => invoke<string[] | null>('open_folder'),
  openImageFile: () => invoke<string | null>('open_image_file'),
  openAudioFiles: () => invoke<string[]>('open_audio_files'),
  scanFoldersWithProgress: (p) => invoke<ScanResult | null>('scan_folders', { paths: p }).then(decorateScan),
  scanFoldersIncremental: (p) => invoke<ScanResult | null>('scan_folders_incremental', { paths: p }).then(decorateScan),
  startupSync: (p) => invoke<ScanResult | null>('startup_sync', { paths: p }).then(decorateScan),
  removeFolder: (fp, rp) => invoke<ScanResult | null>('remove_folder', { folderPath: fp, remainingPaths: rp }).then(decorateScan),
  loadLibrary: () => invoke<ScanResult | null>('load_library').then(decorateScan),
  loadLibraryFast: () => invoke<ScanResult | null>('load_library_fast').then(decorateScan),
  getTrackCovers: (ids) =>
    invoke<Record<string, string>>('get_track_covers', { trackIds: ids }).then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  getFolderCovers: (paths) =>
    invoke<Record<string, string>>('get_folder_covers', { folderPaths: paths }).then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  loadFolderCovers: () =>
    invoke<Record<string, string>>('load_folder_covers').then((m) => {
      const out: Record<string, string> = {}
      for (const [k, v] of Object.entries(m ?? {})) {
        const url = assetUrl(v)
        if (url) out[k] = url
      }
      return out
    }),
  readAsDataURL: (p) => invoke<string | null>('read_as_data_url', { path: p }),
  readTextFile: (p) => invoke<string | null>('read_text_file', { path: p }),
  fileExists: (p) => invoke<boolean>('file_exists', { path: p }),
  listDir: (p) => invoke<DirEntry[]>('list_dir', { path: p }),
  loadSettings: () => invoke<Record<string, unknown>>('load_settings'),
  saveSettings: (s) => invoke<boolean>('save_settings', { settings: s }),
  syncSaveSettings: (s) => {
    void invoke<boolean>('save_settings', { settings: s }).catch(() => false)
  },
  selectBgImage: () => invoke<BgImageData | null>('select_bg_image').then(decorateBg),
  minimizeWindow: () => invoke<void>('minimize_window'),
  maximizeWindow: () => invoke<void>('maximize_window'),
  closeWindow: () => invoke<void>('close_window'),
  forceCloseWindow: () => invoke<void>('force_close_window'),
  isMaximized: () => invoke<boolean>('is_maximized'),
  maximize: () => invoke<void>('maximize_window'),
  minimize: () => invoke<void>('minimize_window'),
  close: () => invoke<void>('close_window'),
  onMaximizeChange: (cb) => onEvent('window:maximizeChange', cb),
  onScanProgress: (cb) => onEvent('scanner:progress', cb),
  onScannerStage: (cb) => onEvent('scanner:stage', cb),
  onBeforeClose: (cb) => onEvent('window:beforeClose', cb),
  startWatching: (p) => invoke<boolean>('start_watching', { paths: p }).catch(() => false),
  stopWatching: () => invoke<boolean>('stop_watching').catch(() => false),
  onFsChanged: (cb) => onEvent('scanner:fsChanged', cb),
  loadBgImage: () => invoke<BgImageData | null>('load_bg_image').then(decorateBg),
  saveBgImage: (d) => invoke<boolean>('save_bg_image', { dataUrl: d }),
  removeBgImage: () => invoke<boolean>('remove_bg_image'),
  toolsSaveFile: (p, b) => invoke<boolean>('tools_save_file', { path: p, base64Data: b }),
  ffmpegExec: (a) => invoke<{ code: number; stdout?: string; stderr?: string }>('ffmpeg_exec', { args: a }),
  ffmpegProbe: () =>
    invoke<FfmpegInfo>('ffmpeg_probe').catch(() => ({ available: false, path: null, version: null })),
  convertRun: (items: ConvertItem[]) => invoke<number>('convert_run', { items }).catch(() => 0),
  convertCancel: (taskId: number) => invoke<boolean>('convert_cancel', { taskId }).catch(() => false),
  // 注意：Tauri 会把 Rust 参数 base_url 归一化为 baseUrl（serde camelCase 同理），
  // 传 baseURL 会因缺参数直接反序列化失败——这里统一在边界做映射。
  aiChat: (payload) =>
    invoke<{ ok: boolean; content: string; error?: string }>('ai_chat', {
      payload: {
        baseUrl: payload.baseURL,
        apiKey: payload.apiKey,
        model: payload.model,
        messages: payload.messages,
        temperature: payload.temperature,
      },
    }),
  aiPing: (payload) =>
    invoke<{ ok: boolean; message: string }>('ai_ping', {
      payload: { baseUrl: payload.baseURL, apiKey: payload.apiKey, model: payload.model },
    }),
  aiListModels: (payload) =>
    invoke<{ ok: boolean; models: string[]; error?: string }>('ai_list_models', {
      baseUrl: payload.baseURL,
      apiKey: payload.apiKey,
    }),
  renameDir: async (oldPath, newPath) => {
    try {
      await invoke<void>('rename_dir', { oldPath, newPath })
      return { ok: true }
    } catch (e) {
      return { ok: false, error: String(e) }
    }
  },
  clipboardWriteText: (t) => invoke<boolean>('clipboard_write_text', { text: t }),
  showItemInFolder: (p) => invoke<boolean>('show_item_in_folder', { path: p }),
  getProgressAll: () => invoke<PlayProgress[]>('get_progress_all'),
  setProgress: (id, ms, c, sp) =>
    invoke<boolean>('set_progress', { trackId: id, positionMs: Math.round(ms), completed: c, lastSpeed: sp }).catch(() => false),
  markTrackCompleted: (id, c) => invoke<boolean>('mark_track_completed', { trackId: id, completed: c }).catch(() => false),
  clearTrackProgress: (id) => invoke<boolean>('clear_track_progress', { trackId: id }).catch(() => false),
  listBookmarks: (id) => invoke<Bookmark[]>('list_bookmarks', { trackId: id }).then(decorateBookmarks).catch(() => []),
  addBookmark: async (id, ms, label) => {
    try {
      return await invoke<Bookmark | null>('add_bookmark', { trackId: id, atMs: Math.round(ms), label })
    } catch {
      return null
    }
  },
  renameBookmark: (id, label) => invoke<boolean>('rename_bookmark', { id, label }).catch(() => false),
  removeBookmark: (id) => invoke<boolean>('remove_bookmark', { id }).catch(() => false),

  // ── 播放内核 ──
  playerPlay: (opts) =>
    invoke<boolean>('player_play', {
      trackPath: opts.trackPath,
      resumeMs: opts.resumeMs ?? 0,
      vol: opts.vol ?? 0.85,
      muted: opts.muted ?? false,
      speed: opts.speed ?? 1,
      videoActive: opts.videoActive ?? false,
    }).then(() => true),
  playerToggle: (paused) => invoke<boolean>('player_toggle', { paused }).then(() => true),
  playerSeek: (sec) => invoke<boolean>('player_seek', { sec }).then(() => true),
  playerSetVolume: (vol) => invoke<boolean>('player_set_volume', { vol }),
  playerSetMuted: (muted) => invoke<boolean>('player_set_muted', { muted }),
  playerSetSpeed: (speed) => invoke<boolean>('player_set_speed', { speed }),
  playerStop: () => invoke<boolean>('player_stop').then(() => true),
  playerSetVideoEnabled: (enabled) => invoke<boolean>('player_set_video_enabled', { enabled }),
  playerSetStageRect: (rect) =>
    invoke<boolean>('player_set_stage_rect', {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      w: Math.round(rect.w),
      h: Math.round(rect.h),
      visible: rect.visible,
    }).catch(() => false),
  playerGetState: () => invoke<MpvPlayerState>('player_get_state').catch(() => ({
    playing: false, position: 0, duration: 0, speed: 1, volume: 85, muted: false,
    trackPath: null, isVideo: false, videoActive: false,
  })),
  pipOpen: (x, y, w, h) =>
    invoke<boolean>('pip_open', { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) }).catch(() => false),
  pipClose: () => invoke<boolean>('pip_close').catch(() => false),
  onPipClosed: (cb) => onEvent('pip:closed', cb),
  onPipPinned: (cb) => onEvent<boolean>('pip:pinned', cb),
  pipSetPinned: (pinned) => invoke<boolean>('pip_set_pinned', { pinned }).catch(() => false),
  playerListDevices: () => invoke<AudioDevice[]>('player_list_devices').catch(() => []),
  playerSetDevice: (deviceId) => invoke<boolean>('player_set_device', { deviceId }).catch(() => false),
  toggleFullscreen: () => invoke<boolean>('toggle_fullscreen').catch(() => false),

  // ── 字幕（mpv + libass）──
  playerSubtitleTracks: () => invoke<SubtitleTrack[]>('player_subtitle_tracks').catch(() => []),
  playerSetSubtitleTrack: (id) => invoke<boolean>('player_set_subtitle_track', { id }).catch(() => false),
  playerSetSubtitleVisible: (visible) =>
    invoke<boolean>('player_set_subtitle_visible', { visible }).catch(() => false),
  playerSetSubtitleDelay: (sec) => invoke<boolean>('player_set_subtitle_delay', { sec }).catch(() => false),
  playerSetSubStyle: (style) =>
    invoke<boolean>('player_set_sub_style', {
      style: {
        font: style.font,
        fontSize: style.fontSize,
        color: style.color,
        borderColor: style.borderColor,
        borderSize: style.borderSize,
        shadowOffset: style.shadowOffset,
        pos: style.pos,
      },
    }).catch(() => false),
  playerApplyLoudnessGain: (trackLufs, targetLufs) =>
    invoke<boolean>('player_apply_loudness_gain', {
      trackLufs,
      targetLufs,
    }).catch(() => false),

  // ── 后台响度分析（任务 + loudness:progress 事件）──
  analyzeLoudness: (tracks: [string, string][]) =>
    invoke<boolean>('analyze_loudness', { tracks }).catch(() => false),
  cancelLoudness: () => invoke<boolean>('cancel_loudness').catch(() => false),

  // ── 分类与标签 ──
  taxonomyListCategories: () => invoke<Category[]>('taxonomy_list_categories').catch(() => []),
  taxonomyCreateCategory: (parentId, name) =>
    invoke<Category>('taxonomy_create_category', { parentId, name }).catch(() => null),
  taxonomyRenameCategory: (id, name) => invoke<boolean>('taxonomy_rename_category', { id, name }).catch(() => false),
  taxonomyDeleteCategory: (id) => invoke<boolean>('taxonomy_delete_category', { id }).catch(() => false),
  taxonomyMoveCategory: (id, parentId, sortIndex) =>
    invoke<boolean>('taxonomy_move_category', { id, parentId, sortIndex }).catch(() => false),
  taxonomyAssignCategory: (categoryId, trackIds) =>
    invoke<boolean>('taxonomy_assign_category', { categoryId, trackIds }).catch(() => false),
  taxonomyUnassignCategory: (categoryId, trackIds) =>
    invoke<boolean>('taxonomy_unassign_category', { categoryId, trackIds }).catch(() => false),
  taxonomyListTags: () => invoke<Tag[]>('taxonomy_list_tags').catch(() => []),
  taxonomyUpsertTag: (name, color, kind) => invoke<Tag | null>('taxonomy_upsert_tag', { name, color, kind }).catch(() => null),
  taxonomyRenameTag: (id, name) => invoke<void>('taxonomy_rename_tag', { id, name }).then(() => true).catch(() => false),
  taxonomyDeleteTag: (id) => invoke<boolean>('taxonomy_delete_tag', { id }).catch(() => false),
  taxonomyTagTracks: (tagIds, trackIds, mode) =>
    invoke<boolean>('taxonomy_tag_tracks', { tagIds, trackIds, mode: mode ?? 'add' }).catch(() => false),
  taxonomyUntagTracks: (tagId, trackIds) => invoke<boolean>('taxonomy_untag_tracks', { tagId, trackIds }).catch(() => false),
  taxonomySuggestTags: (trackIds) =>
    invoke<[string, TagSuggestion[]][]>('taxonomy_suggest_tags', { trackIds })
      .then((list) => (list ?? []).map(([trackId, suggestions]) => ({ trackId, suggestions })))
      .catch(() => []),
  taxonomyApplySuggestedTags: (accepted) =>
    invoke<boolean>('taxonomy_apply_suggested_tags', {
      accepted: accepted.map((a) => [a.trackId, a.tagName]),
    }).catch(() => false),
  taxonomyTrackTags: (trackIds) => invoke<Record<string, Tag[]>>('taxonomy_track_tags', { trackIds }).catch(() => ({})),
  taxonomyAllTrackTags: () =>
    invoke<[string, number][]>('taxonomy_all_track_tags')
      .then((rows) => (rows ?? []).map(([trackId, tagId]) => ({ trackId, tagId })))
      .catch(() => []),
  taxonomyAllCategoryItems: () =>
    invoke<[number, string][]>('taxonomy_all_category_items')
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

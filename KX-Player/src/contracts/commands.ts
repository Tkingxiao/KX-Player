/**
 * 契约层 · 命令清单：每条 `#[tauri::command]` 一行，是「前端能调什么」的唯一来源。
 *
 * 与 Rust 侧的一致性由 `npm run check:ipc`（`scripts/gen-ipc-doc.mjs`）把三件事：
 * 命令名集合、`lib.rs` 的 `generate_handler!` 注册表、`bridge/ipc.ts` 实际传的**参数键名**
 * （Rust 的 `snake_case` 形参在 JS 侧是 `camelCase`）。改完 Rust 记得回来同步这里，
 * 再 `npm run gen:ipc` 刷新 `docs/IPC.md`。
 *
 * `returns` 写的是 **resolve 值** 的类型，并且是编译期约束：`bridge/ipc.ts` 的 `invoke()` 泛型
 * 直接取这里的 `args`/`returns`，写错键名或漏传参数会在调用点报错，不只是 `check:ipc` 的静态对账。
 * 失败不经 `returns`：Rust 的 `Err(IpcError)` 走 reject（`02 §6.1`），由 `contracts/result.ts` 归一化成 `AppError`。
 * Rust 的 `()` 在这里记作 `void`（线上是 JSON `null`，但前端不该消费它）。
 */
import type {
  AiChatPayload,
  AiChatResult,
  AiExportResult,
  AiImportResult,
  AiModelsResult,
  AiPingResult,
  AiTextsSpec,
  AiTranslateSpec,
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
  ScanResult,
  SubStyle,
  SubtitleScanItem,
  SubtitleTrack,
  Tag,
  TagSuggestion,
} from './dto'

export interface CommandSpecs {
  // ── 文件对话框与磁盘访问（commands/dialog_fs.rs）──
  clipboard_write_text: { args: { text: string }, returns: boolean }
  file_exists: { args: { path: string }, returns: boolean }
  list_dir: { args: { path: string }, returns: DirEntry[] }
  open_audio_files: { args: {}, returns: string[] }
  open_folder: { args: {}, returns: string[] | null }
  open_image_file: { args: {}, returns: string | null }
  open_text_files: { args: {}, returns: string[] }
  read_as_data_url: { args: { path: string }, returns: string | null }
  read_text_file: { args: { path: string }, returns: string | null }
  rename_dir: { args: { oldPath: string, newPath: string }, returns: void }
  select_bg_image: { args: {}, returns: BgImageData | null }
  show_item_in_folder: { args: { path: string }, returns: boolean }
  tools_save_file: { args: { path: string, base64Data: string }, returns: boolean }

  // ── 曲库、进度、书签、分类与标签（commands/library.rs）──
  add_bookmark: { args: { trackId: string, atMs: number, label: string }, returns: Bookmark | null }
  analyze_loudness: { args: { tracks: [string, string][] }, returns: boolean }
  cancel_loudness: { args: {}, returns: boolean }
  clear_track_progress: { args: { trackId: string }, returns: boolean }
  get_folder_covers: { args: { folderPaths: string[] }, returns: Record<string, string> }
  get_progress_all: { args: {}, returns: PlayProgress[] }
  get_track_covers: { args: { trackIds: string[] }, returns: Record<string, string> }
  list_bookmarks: { args: { trackId: string }, returns: Bookmark[] }
  load_folder_covers: { args: {}, returns: Record<string, string> }
  load_library: { args: {}, returns: ScanResult | null }
  load_library_fast: { args: {}, returns: ScanResult | null }
  mark_track_completed: { args: { trackId: string, completed: boolean }, returns: boolean }
  remove_bookmark: { args: { id: string }, returns: boolean }
  remove_folder: { args: { folderPath: string, remainingPaths: string[] }, returns: ScanResult | null }
  rename_bookmark: { args: { id: string, label: string }, returns: boolean }
  scan_folders: { args: { paths: string[] }, returns: ScanResult | null }
  scan_folders_incremental: { args: { paths: string[] }, returns: ScanResult | null }
  set_progress: { args: { trackId: string, positionMs: number, completed: boolean | null, lastSpeed: number | null }, returns: boolean }
  start_watching: { args: { paths: string[] }, returns: boolean }
  startup_sync: { args: { paths: string[] }, returns: ScanResult | null }
  stop_watching: { args: {}, returns: boolean }
  taxonomy_all_category_items: { args: {}, returns: [number, string][] }
  taxonomy_all_track_tags: { args: {}, returns: [string, number][] }
  taxonomy_apply_suggested_tags: { args: { accepted: [string, string][] }, returns: number }
  taxonomy_assign_category: { args: { categoryId: number, trackIds: string[] }, returns: number }
  taxonomy_category_track_ids: { args: { categoryId: number, includeChildren: boolean | null }, returns: string[] }
  taxonomy_create_category: { args: { parentId: number | null, name: string }, returns: Category }
  taxonomy_delete_category: { args: { id: number }, returns: void }
  taxonomy_delete_tag: { args: { id: number }, returns: void }
  taxonomy_list_categories: { args: {}, returns: Category[] }
  taxonomy_list_tags: { args: {}, returns: Tag[] }
  taxonomy_move_category: { args: { id: number, parentId: number | null, sortIndex: number }, returns: void }
  taxonomy_rename_category: { args: { id: number, name: string }, returns: void }
  taxonomy_rename_tag: { args: { id: number, name: string }, returns: void }
  taxonomy_suggest_tags: { args: { trackIds: string[] }, returns: [string, TagSuggestion[]][] }
  taxonomy_tag_tracks: { args: { tagIds: number[], trackIds: string[], mode: string | null }, returns: number }
  taxonomy_track_tags: { args: { trackIds: string[] }, returns: Record<string, Tag[]> }
  taxonomy_unassign_category: { args: { categoryId: number, trackIds: string[] }, returns: number }
  taxonomy_untag_tracks: { args: { tagId: number, trackIds: string[] }, returns: number }
  taxonomy_upsert_tag: { args: { name: string, color: string | null, kind: string | null }, returns: Tag }

  // ── 播放内核与悬浮窗（commands/player.rs）──
  app_force_quit: { args: {}, returns: void }
  pip_close: { args: {}, returns: boolean }
  pip_open: { args: { x: number, y: number, w: number, h: number }, returns: boolean }
  pip_ready: { args: {}, returns: boolean }
  pip_restore: { args: {}, returns: boolean }
  pip_set_pinned: { args: { pinned: boolean }, returns: boolean }
  player_apply_loudness_gain: { args: { trackLufs: number | null, targetLufs: number }, returns: void }
  player_get_state: { args: {}, returns: MpvPlayerState }
  player_list_devices: { args: {}, returns: AudioDevice[] }
  player_play: { args: { trackPath: string, resumeMs: number | null, vol: number | null, muted: boolean | null, speed: number | null, videoActive: boolean | null }, returns: void }
  player_seek: { args: { sec: number }, returns: void }
  player_set_device: { args: { deviceId: string }, returns: boolean }
  player_set_muted: { args: { muted: boolean }, returns: void }
  player_set_speed: { args: { speed: number }, returns: void }
  player_set_stage_rect: { args: { x: number, y: number, w: number, h: number, visible: boolean }, returns: boolean }
  player_set_sub_style: { args: { style: SubStyle }, returns: void }
  player_set_subtitle_delay: { args: { sec: number }, returns: void }
  player_set_subtitle_track: { args: { id: number }, returns: void }
  player_set_subtitle_visible: { args: { visible: boolean }, returns: void }
  player_set_video_enabled: { args: { enabled: boolean }, returns: void }
  player_set_volume: { args: { vol: number }, returns: void }
  player_stop: { args: {}, returns: void }
  player_subtitle_tracks: { args: {}, returns: SubtitleTrack[] }
  player_toggle: { args: { paused: boolean }, returns: void }

  // ── 窗口、设置、ffmpeg、AI（commands/system.rs）──
  ai_chat: { args: { payload: AiChatPayload }, returns: AiChatResult }
  ai_export_prompts: { args: { paths: string[] }, returns: AiExportResult }
  ai_import_translations: { args: { paths: string[], text: string, apply: boolean }, returns: AiImportResult }
  ai_list_models: { args: { baseUrl: string, apiKey: string }, returns: AiModelsResult }
  ai_ping: { args: { payload: AiChatPayload }, returns: AiPingResult }
  ai_task_cancel: { args: { taskId: number }, returns: boolean }
  ai_texts_start: { args: { spec: AiTextsSpec }, returns: number }
  ai_translate_start: { args: { spec: AiTranslateSpec }, returns: number }
  close_window: { args: {}, returns: void }
  convert_cancel: { args: { taskId: number }, returns: boolean }
  convert_run: { args: { items: ConvertItem[] }, returns: number }
  ffmpeg_exec: { args: { args: string[] }, returns: ExecResult }
  ffmpeg_probe: { args: {}, returns: FfmpegInfo }
  force_close_window: { args: {}, returns: void }
  is_maximized: { args: {}, returns: boolean }
  load_bg_image: { args: {}, returns: BgImageData | null }
  load_settings: { args: {}, returns: Record<string, unknown> }
  maximize_window: { args: {}, returns: void }
  minimize_window: { args: {}, returns: void }
  remove_bg_image: { args: {}, returns: boolean }
  save_bg_image: { args: { dataUrl: string }, returns: boolean }
  save_settings: { args: { settings: Record<string, unknown> }, returns: boolean }
  startup_warnings: { args: {}, returns: string[] }
  subtitle_scan_dir: { args: { dir: string, kind: string, maxDepth?: number, limit?: number }, returns: SubtitleScanItem[] }
  toggle_fullscreen: { args: {}, returns: boolean }

  // ── 改名主干（commands/rename.rs）──
  rename_preview: { args: { paths: string[] }, returns: RenamePreview }
}

export type CommandName = keyof CommandSpecs
/** 前端 `invoke` 第二个参数的键名集合；`{}` 表示无参。 */
export type CommandArgs<K extends CommandName> = CommandSpecs[K]['args']
/** resolve 值类型（错误目前走 reject，见文件头）。 */
export type CommandReturn<K extends CommandName> = CommandSpecs[K]['returns']

# IPC 契约（命令与事件）

> 由 `npm run gen:ipc`（`scripts/gen-ipc-doc.mjs`）生成，请勿手改。
> 一致性由 `npm run check:ipc` 把关：Rust `#[tauri::command]` · `lib.rs` 注册表 ·
> `src/contracts/commands.ts` · `src/bridge/ipc.ts` 的参数键名，四方对账（`02 §6.4`）。

类型映射：`String→string`、`i64/u64/f64→number`、`bool→boolean`、`Vec<T>→T[]`、
`Option<T>→T | null`（JSON 是 `null`，不是 `undefined`）、`HashMap<K,V>→Record<K,V>`、
`(A,B)→[A, B]`、`serde_json::Value→unknown`。

> 参数键名：Rust 的 `snake_case` 形参在 JS 侧是 `camelCase`（Tauri v2 默认归一化）。
> 入参若是结构体，字段名以该结构体的 `#[serde]` 属性为准。

> **返回列是 resolve 值，即 `IpcResult<T>`/`Result<T, _>` 的 `Ok` 分支**。
> 失败经 Tauri 走 reject，载荷是 `IpcError { code, message, detail }`（`02 §6.1`），
> `bridge/ipc.ts` 统一归一化为 `AppError`；`message` 面向用户，`detail` 只进日志。

## dialog_fs

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `clipboard_write_text` | `text: String` | `boolean` | src-tauri/src/commands/dialog_fs.rs |
| `file_exists` | `path: String` | `boolean` | src-tauri/src/commands/dialog_fs.rs |
| `list_dir` | `path: String` | `DirEntryInfo[]` | src-tauri/src/commands/dialog_fs.rs |
| `open_audio_files` | — | `string[]` | src-tauri/src/commands/dialog_fs.rs |
| `open_folder` | — | `string[] ∣ null` | src-tauri/src/commands/dialog_fs.rs |
| `open_image_file` | — | `string ∣ null` | src-tauri/src/commands/dialog_fs.rs |
| `open_text_files` | — | `string[]` | src-tauri/src/commands/dialog_fs.rs |
| `read_as_data_url` | `path: String` | `string ∣ null` | src-tauri/src/commands/dialog_fs.rs |
| `read_text_file` | `path: String` | `string ∣ null` | src-tauri/src/commands/dialog_fs.rs |
| `rename_dir` | `oldPath: String`, `newPath: String` | `null` | src-tauri/src/commands/dialog_fs.rs |
| `select_bg_image` | — | `BgImageData ∣ null` | src-tauri/src/commands/dialog_fs.rs |
| `show_item_in_folder` | `path: String` | `boolean` | src-tauri/src/commands/dialog_fs.rs |
| `tools_save_file` | `path: String`, `base64Data: String` | `boolean` | src-tauri/src/commands/dialog_fs.rs |

## library

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `add_bookmark` | `trackId: String`, `atMs: i64`, `label: String` | `Bookmark ∣ null` | src-tauri/src/commands/library.rs |
| `analyze_loudness` | `tracks: Vec<(String, String)>` | `boolean` | src-tauri/src/commands/library.rs |
| `cancel_loudness` | — | `boolean` | src-tauri/src/commands/library.rs |
| `clear_track_progress` | `trackId: String` | `boolean` | src-tauri/src/commands/library.rs |
| `get_folder_covers` | `folderPaths: Vec<String>` | `Record<string, string>` | src-tauri/src/commands/library.rs |
| `get_progress_all` | — | `PlayProgress[]` | src-tauri/src/commands/library.rs |
| `get_track_covers` | `trackIds: Vec<String>` | `Record<string, string>` | src-tauri/src/commands/library.rs |
| `list_bookmarks` | `trackId: String` | `Bookmark[]` | src-tauri/src/commands/library.rs |
| `load_folder_covers` | — | `Record<string, string>` | src-tauri/src/commands/library.rs |
| `load_library` | — | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `load_library_fast` | — | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `mark_track_completed` | `trackId: String`, `completed: bool` | `boolean` | src-tauri/src/commands/library.rs |
| `remove_bookmark` | `id: String` | `boolean` | src-tauri/src/commands/library.rs |
| `remove_folder` | `folderPath: String`, `remainingPaths: Vec<String>` | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `rename_bookmark` | `id: String`, `label: String` | `boolean` | src-tauri/src/commands/library.rs |
| `scan_folders` | `paths: Vec<String>` | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `scan_folders_incremental` | `paths: Vec<String>` | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `set_progress` | `trackId: String`, `positionMs: i64`, `completed: Option<bool>`, `lastSpeed: Option<f64>` | `boolean` | src-tauri/src/commands/library.rs |
| `start_watching` | `paths: Vec<String>` | `boolean` | src-tauri/src/commands/library.rs |
| `startup_sync` | `paths: Vec<String>` | `ScanResult ∣ null` | src-tauri/src/commands/library.rs |
| `stop_watching` | — | `boolean` | src-tauri/src/commands/library.rs |

## player

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `app_force_quit` | — | `null` | src-tauri/src/commands/player.rs |
| `pip_close` | — | `boolean` | src-tauri/src/commands/player.rs |
| `pip_open` | `x: i32`, `y: i32`, `w: u32`, `h: u32` | `boolean` | src-tauri/src/commands/player.rs |
| `pip_ready` | — | `boolean` | src-tauri/src/commands/player.rs |
| `pip_restore` | — | `boolean` | src-tauri/src/commands/player.rs |
| `pip_set_pinned` | `pinned: bool` | `boolean` | src-tauri/src/commands/player.rs |
| `player_apply_loudness_gain` | `trackLufs: Option<f64>`, `targetLufs: f64` | `null` | src-tauri/src/commands/player.rs |
| `player_get_state` | — | `PlayerState` | src-tauri/src/commands/player.rs |
| `player_list_devices` | — | `AudioDevice[]` | src-tauri/src/commands/player.rs |
| `player_play` | `trackPath: String`, `resumeMs: Option<f64>`, `vol: Option<f64>`, `muted: Option<bool>`, `speed: Option<f64>`, `videoActive: Option<bool>` | `null` | src-tauri/src/commands/player.rs |
| `player_seek` | `sec: f64` | `null` | src-tauri/src/commands/player.rs |
| `player_set_device` | `deviceId: String` | `boolean` | src-tauri/src/commands/player.rs |
| `player_set_muted` | `muted: bool` | `null` | src-tauri/src/commands/player.rs |
| `player_set_speed` | `speed: f64` | `null` | src-tauri/src/commands/player.rs |
| `player_set_stage_rect` | `x: i32`, `y: i32`, `w: i32`, `h: i32`, `visible: bool` | `boolean` | src-tauri/src/commands/player.rs |
| `player_set_sub_style` | `style: SubStyle` | `null` | src-tauri/src/commands/player.rs |
| `player_set_subtitle_delay` | `sec: f64` | `null` | src-tauri/src/commands/player.rs |
| `player_set_subtitle_track` | `id: i64` | `null` | src-tauri/src/commands/player.rs |
| `player_set_subtitle_visible` | `visible: bool` | `null` | src-tauri/src/commands/player.rs |
| `player_set_video_enabled` | `enabled: bool` | `null` | src-tauri/src/commands/player.rs |
| `player_set_volume` | `vol: f64` | `null` | src-tauri/src/commands/player.rs |
| `player_stop` | — | `null` | src-tauri/src/commands/player.rs |
| `player_subtitle_tracks` | — | `SubtitleTrack[]` | src-tauri/src/commands/player.rs |
| `player_toggle` | `paused: bool` | `null` | src-tauri/src/commands/player.rs |

## rename

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `rename_apply` | `paths: Vec<String>` | `Report` | src-tauri/src/commands/rename.rs |
| `rename_batches` | `limit: u32` | `Batch[]` | src-tauri/src/commands/rename.rs |
| `rename_preview` | `paths: Vec<String>` | `Preview` | src-tauri/src/commands/rename.rs |
| `rename_redo` | `batchId: String` | `Report` | src-tauri/src/commands/rename.rs |
| `rename_reject_suggestion` | `path: String`, `suggestion: String` | `null` | src-tauri/src/commands/rename.rs |
| `rename_rollback` | `batchId: String` | `Report` | src-tauri/src/commands/rename.rs |
| `rename_suggest_names` | `paths: Vec<String>`, `baseUrl: String`, `apiKey: String`, `model: String`, `modelKind: Option<ModelKind>` | `NameReport` | src-tauri/src/commands/rename.rs |

## system

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `ai_cache_folder_suggestions` | — | `Record<string, string>` | src-tauri/src/commands/system.rs |
| `ai_cache_record_folder` | `path: String`, `suggestion: String` | `boolean` | src-tauri/src/commands/system.rs |
| `ai_chat` | `payload: AiChatPayload` | `AiChatResult` | src-tauri/src/commands/system.rs |
| `ai_export_prompts` | `paths: Vec<String>` | `ExportResult` | src-tauri/src/commands/system.rs |
| `ai_import_translations` | `paths: Vec<String>`, `text: String`, `apply: bool` | `ImportResult` | src-tauri/src/commands/system.rs |
| `ai_list_models` | `baseUrl: String`, `apiKey: String` | `AiModelsResult` | src-tauri/src/commands/system.rs |
| `ai_ping` | `payload: AiChatPayload` | `AiPingResult` | src-tauri/src/commands/system.rs |
| `ai_task_cancel` | `taskId: u64` | `boolean` | src-tauri/src/commands/system.rs |
| `ai_texts_start` | `spec: crate::ai_job::AiTextsSpec` | `number` | src-tauri/src/commands/system.rs |
| `ai_translate_start` | `spec: crate::ai_job::AiTranslateSpec` | `number` | src-tauri/src/commands/system.rs |
| `close_window` | — | `null` | src-tauri/src/commands/system.rs |
| `convert_cancel` | `taskId: u64` | `boolean` | src-tauri/src/commands/system.rs |
| `convert_run` | `items: Vec<crate::fftools::ConvertItem>` | `number` | src-tauri/src/commands/system.rs |
| `ffmpeg_exec` | `args: Vec<String>` | `ExecResult` | src-tauri/src/commands/system.rs |
| `ffmpeg_probe` | — | `FfmpegInfo` | src-tauri/src/commands/system.rs |
| `force_close_window` | — | `null` | src-tauri/src/commands/system.rs |
| `is_maximized` | — | `boolean` | src-tauri/src/commands/system.rs |
| `load_bg_image` | — | `BgImageData ∣ null` | src-tauri/src/commands/system.rs |
| `load_settings` | — | `unknown` | src-tauri/src/commands/system.rs |
| `maximize_window` | — | `null` | src-tauri/src/commands/system.rs |
| `minimize_window` | — | `null` | src-tauri/src/commands/system.rs |
| `remove_bg_image` | — | `boolean` | src-tauri/src/commands/system.rs |
| `save_bg_image` | `dataUrl: String` | `boolean` | src-tauri/src/commands/system.rs |
| `save_settings` | `settings: Value` | `boolean` | src-tauri/src/commands/system.rs |
| `startup_warnings` | — | `string[]` | src-tauri/src/commands/system.rs |
| `subtitle_scan_dir` | `dir: String`, `kind: String`, `maxDepth: Option<usize>`, `limit: Option<usize>` | `ScannedItem[]` | src-tauri/src/commands/system.rs |
| `toggle_fullscreen` | — | `boolean` | src-tauri/src/commands/system.rs |

## taxonomy

| 命令 | 参数（JS 键名 : Rust 类型） | resolve | 位置 |
|---|---|---|---|
| `taxonomy_all_category_items` | — | `[number, string][]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_all_track_tags` | — | `[string, number][]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_apply_suggested_tags` | `accepted: Vec<(String, String)>` | `number` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_assign_category` | `categoryId: i64`, `trackIds: Vec<String>` | `number` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_category_track_ids` | `categoryId: i64`, `includeChildren: Option<bool>` | `string[]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_create_category` | `parentId: Option<i64>`, `name: String` | `Category` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_delete_category` | `id: i64` | `null` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_delete_tag` | `id: i64` | `null` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_list_categories` | — | `Category[]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_list_tags` | — | `Tag[]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_move_category` | `id: i64`, `parentId: Option<i64>`, `sortIndex: i64` | `null` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_rename_category` | `id: i64`, `name: String` | `null` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_rename_tag` | `id: i64`, `name: String` | `null` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_suggest_tags` | `trackIds: Vec<String>` | `[string, TagSuggestion[]][]` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_tag_tracks` | `tagIds: Vec<i64>`, `trackIds: Vec<String>`, `mode: Option<String>` | `number` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_track_tags` | `trackIds: Vec<String>` | `Record<string, Tag[]>` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_unassign_category` | `categoryId: i64`, `trackIds: Vec<String>` | `number` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_untag_tracks` | `tagId: i64`, `trackIds: Vec<String>` | `number` | src-tauri/src/commands/taxonomy.rs |
| `taxonomy_upsert_tag` | `name: String`, `color: Option<String>`, `kind: Option<String>` | `Tag` | src-tauri/src/commands/taxonomy.rs |

## 事件

载荷类型的权威定义在 `src/contracts/events.ts`；下表核对两侧名字与位置。

| 事件 | 载荷 | 发射（Rust） | 监听（前端） |
|---|---|---|---|
| `ai:progress` | `AiProgress` | src-tauri/src/ai_chunk.rs<br>src-tauri/src/ai_job.rs | src/services/aiTranslate.ts |
| `convert:progress` | `ConvertProgress` | src-tauri/src/fftools.rs | src/features/convert/ConvertView.vue |
| `loudness:progress` | `LoudnessProgress` | src-tauri/src/commands/library.rs | src/stores/library.ts |
| `pip:closed` | `null` | src-tauri/src/commands/player.rs<br>src-tauri/src/lib.rs | src/bridge/ipc.ts<br>src/components/PipRoot.vue |
| `pip:pinned` | `boolean` | src-tauri/src/commands/player.rs | src/bridge/ipc.ts |
| `pip:restored` | `null` | src-tauri/src/commands/player.rs | src/bridge/ipc.ts |
| `pip:shown` | `null` | src-tauri/src/commands/player.rs | src/components/PipRoot.vue |
| `player:ended` | `{ reason: string }` | src-tauri/src/player/mod.rs | src/components/PipRoot.vue<br>src/stores/player.ts |
| `player:error` | `{ message: string }` | src-tauri/src/player/mod.rs | src/stores/player.ts |
| `player:loading` | `null` | src-tauri/src/player/mod.rs | src/stores/player.ts |
| `player:state` | `MpvPlayerState` | src-tauri/src/player/mod.rs | src/components/PipRoot.vue<br>src/stores/player.ts |
| `scanner:fsChanged` | `null` | src-tauri/src/watcher.rs | src/bridge/ipc.ts |
| `scanner:progress` | `{ completed: number; total: number }` | src-tauri/src/scanner/mod.rs | src/bridge/ipc.ts |
| `scanner:stage` | `string` | src-tauri/src/scanner/mod.rs | src/bridge/ipc.ts |
| `window:beforeClose` | `null` | src-tauri/src/commands/player.rs<br>src-tauri/src/commands/system.rs<br>src-tauri/src/lib.rs | src/bridge/ipc.ts |
| `window:maximizeChange` | `boolean` | src-tauri/src/lib.rs | src/bridge/ipc.ts |

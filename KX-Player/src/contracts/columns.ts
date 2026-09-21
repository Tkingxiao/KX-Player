/**
 * 契约层 · 列名单一事实源（`02 §4` 要求）。
 *
 * ── 这个文件是什么 ────────────────────────────────────────────────
 * 数据库列名**只在这里写一次**。它同时供三处引用：
 *   1. 字段级自查：`DTO_FIELDS` / `DTO_TABLES` 让 TS 侧一眼看清「DTO 字段 ↔ 列」；
 *   2. `scripts/check-columns.mjs`：DDL ↔ Rust SQL ↔ DTO 三方对账；
 *   3. 人读：想知道 `Track.isVideo` 落在哪一列，看 `DTO_FIELDS.Track`，不用翻 Rust。
 *
 * ── 谁消费它 ──────────────────────────────────────────────────────
 * `npm run check:columns`（`npm run check` 的一环）。目前**没有运行期消费者** ——
 * 生产路径上的列名仍然是字面量，写在 `src-tauri/src/db/*.rs` 的 SQL 串与
 * `src-tauri/migrations/*.sql` 的建表语句里。本文件的价值不在运行期，而在于把这三处
 * 事实源钉在一起：**它们不一致时 `check:columns` 必须红**。
 *
 * ── 怎么被校验 ────────────────────────────────────────────────────
 * `node scripts/check-columns.mjs` 解析源码文本，四项对账：
 *   (a) `DB_COLUMNS` 里每个列名都真实存在于 schema（`migrations/*.sql` 的
 *       `CREATE TABLE` ∪ Rust 生产代码里的 `CREATE (TEMP) TABLE` ∪
 *       `ensure_column` / `ALTER TABLE ADD COLUMN`）—— 抓「这里改了、DDL 没改」；
 *   (b) `src-tauri/src/db/*.rs` 的每个列引用位置（含 `t.id` 这种带表别名的形式）都存在于
 *       schema —— 抓「SQL 改了、DDL 没改」或拼错列名。位置覆盖 SELECT 列表、INSERT 列清单、
 *       ON CONFLICT(...)、SET 左值、ORDER BY / GROUP BY 项、比较与 IN/LIKE/IS 左侧标识符；
 *   (c) `dto.ts` 里 `DTO_TABLES` 覆盖的接口，每个字段都要能映射到该表的列
 *       （camelCase → snake_case 自动归一化），否则必须在 `EXCEPTIONS.DTO_FIELDS` 里；
 *   (d) 反向：`DTO_TABLES` 用到的那张表，DDL 每一列要么被 `DB_COLUMNS` 收录，
 *       要么在 `EXCEPTIONS.DB_COLUMNS` 里 —— 防「加了列没人用 / 忘了登记」。
 *
 * 脚本还会检查 `dto.ts` 的**每个**接口是否已被分类（`DTO_TABLES` 或 `DTO_NO_TABLE`），
 * 并检查例外清单里有没有已经用不上的条目 —— 两侧都不许静默。
 *
 * ── DDL 之外的 schema 来源（脚本会显式标注，不是静默豁免）──────────
 * `db/migrations.rs` 的 `ensure_column("tracks","loudness_lufs","REAL")` 是为**老库接管**
 * 补的列（`001_init.sql` 里也有）；`schema_migrations` 由 `db/migrations.rs` 内联创建；
 * `legacy_map` 是 `db/mod.rs` 的事务内临时表。脚本按来源合并后再校验，输出里标明
 * 每张表/每个补列来自哪一份；出现在两份之外才算失败。
 *
 * ── 不要把这些表当第二份清单 ──────────────────────────────────────
 * 本文件只登记**列名**，不登记类型、约束、索引（那些的事实源是 DDL 本身）。
 * 加列的正确顺序：先改 `src-tauri/migrations/*.sql`（或 `migrations.rs` 的补列），
 * 再在本文件登记，最后按需在 `dto.ts` 加字段 —— 漏掉任何一步 `check:columns` 都会红。
 */

// ── 表名 ────────────────────────────────────────────────────────

export const TABLES = {
  libraryMeta: 'library_meta',
  artists: 'artists',
  albums: 'albums',
  tracks: 'tracks',
  folderNodes: 'folder_nodes',
  folderTracks: 'folder_tracks',
  categories: 'categories',
  categoryItems: 'category_items',
  tags: 'tags',
  trackTags: 'track_tags',
  playProgress: 'play_progress',
  bookmarks: 'bookmarks',
} as const

export type TableName = (typeof TABLES)[keyof typeof TABLES]

// ── 列名（唯一事实源）──────────────────────────────────────────
//
// 顺序与 `src-tauri/migrations/*.sql` 的建表语句一致，便于人肉对照；
// 脚本不依赖顺序，只依赖「名字都在这里」。

export const DB_COLUMNS = {
  library_meta: ['key', 'value'],

  artists: ['artist_id', 'name', 'root_path'],

  albums: ['album_id', 'artist_id', 'name', 'artist_name', 'cover_path'],

  tracks: [
    'id',
    'artist_id',
    'album_id',
    'name',
    'path',
    'duration',
    'artist',
    'album',
    'format',
    'is_video',
    'cover_path',
    'lyrics_path',
    'file_mtime',
    'file_size',
    'meta_title',
    'meta_artist',
    'genre',
    'bitrate',
    'sample_rate',
    'loudness_lufs',
    // cover_data / album_cover_data 有意不收：Rust 侧只写 NULL、SELECT 从不读，见 EXCEPTIONS.DB_COLUMNS
  ],

  folder_nodes: ['path', 'name', 'parent_path', 'track_count'],

  folder_tracks: ['folder_path', 'track_id'],

  // 003_taxonomy.sql
  categories: ['id', 'parent_id', 'name', 'icon', 'color', 'sort_index', 'created_at'],

  category_items: ['category_id', 'track_id', 'added_at'],

  tags: ['id', 'name', 'color', 'kind', 'use_count', 'auto_generated'],

  track_tags: ['track_id', 'tag_id', 'source'],

  play_progress: ['track_id', 'position_ms', 'completed', 'play_count', 'played_at', 'last_speed'],

  bookmarks: ['id', 'track_id', 'at_ms', 'label', 'created_at'],
} as const

export type DbTable = keyof typeof DB_COLUMNS

/** `DB_COLUMNS.tracks` 的联合类型；其它表按需照写。 */
export type TrackColumn = (typeof DB_COLUMNS)['tracks'][number]

// ── DTO 字段 → 列名（由 check 脚本实时比对，不是说明文字）─────────
//
// 每条是 `<DTO 字段>: '<列名>'`。值必须是**同表**的列，键是 `dto.ts` 里真实存在的字段名。
// 字段不来自同名列时**不要写在这里**，写进 `EXCEPTIONS.DTO_FIELDS` 并一行写清原因。

export const DTO_FIELDS = {
  /** `src/contracts/dto.ts` → `interface Track`（落在 tracks 表） */
  Track: {
    id: 'id',
    name: 'name',
    path: 'path',
    duration: 'duration',
    artist: 'artist',
    album: 'album',
    format: 'format',
    isVideo: 'is_video',
    coverPath: 'cover_path',
    coverData: 'cover_data',
    lyricsPath: 'lyrics_path',
    fileMtime: 'file_mtime',
    fileSize: 'file_size',
    metaTitle: 'meta_title',
    metaArtist: 'meta_artist',
    genre: 'genre',
    bitrate: 'bitrate',
    sampleRate: 'sample_rate',
    loudnessLufs: 'loudness_lufs',
    albumCoverData: 'album_cover_data',
  },

  /** `src/contracts/dto.ts` → `interface FolderNode`（落在 folder_nodes 表） */
  FolderNode: {
    name: 'name',
    path: 'path',
    trackCount: 'track_count',
    coverData: 'cover_data',
  },

  /** `src/contracts/dto.ts` → `interface PlayProgress`（落在 play_progress 表） */
  PlayProgress: {
    trackId: 'track_id',
    positionMs: 'position_ms',
    completed: 'completed',
    playCount: 'play_count',
    playedAt: 'played_at',
    lastSpeed: 'last_speed',
  },

  /** `src/contracts/dto.ts` → `interface Bookmark`（落在 bookmarks 表） */
  Bookmark: {
    id: 'id',
    trackId: 'track_id',
    atMs: 'at_ms',
    label: 'label',
    createdAt: 'created_at',
  },

  /** `src/contracts/dto.ts` → `interface Category`（落在 categories 表） */
  Category: {
    id: 'id',
    parentId: 'parent_id',
    name: 'name',
    icon: 'icon',
    color: 'color',
    sortIndex: 'sort_index',
  },

  /** `src/contracts/dto.ts` → `interface Tag`（落在 tags 表） */
  Tag: {
    id: 'id',
    name: 'name',
    color: 'color',
    kind: 'kind',
    useCount: 'use_count',
    autoGenerated: 'auto_generated',
  },
} as const

/** DTO 接口名 → 驱动它的表。这里列出的接口会被逐字段对账。 */
export const DTO_TABLES = {
  Track: 'tracks',
  FolderNode: 'folder_nodes',
  PlayProgress: 'play_progress',
  Bookmark: 'bookmarks',
  Category: 'categories',
  Tag: 'tags',
} as const

/**
 * 不是单表投影的 DTO 接口 —— 每一条都要写清为什么不做字段级对账。
 * 新增 DTO 必须在这张表或 DTO_TABLES 里二选一，否则 check:columns 直接红（不许静默跳过）。
 */
export const DTO_NO_TABLE = {
  // —— 由多表拼装的组装结果 ——
  ScanResult: '曲库快照的顶层容器：library_meta 的 KV + tracks 全量 + artists/albums 拼装（db/library.rs:57-222），没有单表对应。',

  // —— 不进库：文件系统与运行期探测 ——
  DirEntry: '文件系统条目（commands/dialog_fs.rs），不进库。',
  FfmpegInfo: '运行期探测 ffmpeg 的结果（paths::locate_ffmpeg），不落库。',
  ExecResult: 'ffmpeg 子进程的 stdout/stderr 返回体，不落库。',

  // —— 不进库：mpv 运行期状态（Rust 侧只是镜像）——
  MpvPlayerState: 'mpv 属性镜像（player/* 推给前端），不落库。',
  SubtitleTrack: 'mpv track-list 运行期读取；02 §5.1 的 subtitle_streams 表尚未建。',
  AudioDevice: 'mpv audio-device-list 运行期读取，不落库。',
  SubStyle: '字幕样式是设置项（settings.json 的 sub* 键），不落库。',

  // —— 不进库：settings.json / 前端队列 ——
  BgImageData: '背景图的路径与 mtime，存 settings.json（stores/settings.ts），不落库。',
  FavItem: '收藏列表存 settings.json 的 favs（stores/playlists.ts）；02 §5.1 的 playlists 表尚未建。',
  ConvertItem: '转换队列项由前端逐项传给 convert_run，不落库。',
  ConvertProgress: 'convert:progress 事件载荷，不落库。',
  TagSuggestion: '自动打标的建议（taxonomy.rs:389 起由关键词表实时算出），不落库（宪法禁止项 24：只建议不写入）。',

  // —— 不进库：AI 线形状的请求/返回体（AI 7 张表尚未建）——
  ChatMessage: 'AI 请求体里的消息数组（线形状），不落库。',
  AiChatPayload: 'AI 命令入参（线形状），不落库。',
  AiChatResult: 'AI 命令返回体（线形状），不落库。',
  AiPingResult: 'AI 连通性测试回显（线形状），不落库。',
  AiModelsResult: 'AI 模型清单返回体（线形状），不落库。',
} as const

// ── 例外清单 ────────────────────────────────────────────────────
//
// 这是**审计用**的清单，不是绕开检查的后门。判定口径：
//   · DB_COLUMNS：DDL 里有、但 DB_COLUMNS 故意不登记的列（写死 NULL 的死列）；
//   · DTO_FIELDS：`dto.ts` 里不是同名列、也不是自动归一化能命中的字段（派生 / 组树）；
//   · RUST_SQL_COLUMNS：Rust SQL 里出现、但不属于任何表的系统标识符。
// 脚本会核对：键名真实存在、且**确实还需要**（用不上的条目算失败），免得清单越长越像遮羞布。
// 新增漏项一律先当 bug 查，确认是上面几类之一才登记。

export const EXCEPTIONS = {
  /** DDL 有、DB_COLUMNS 不收的列。键格式 `<表>.<列>`。 */
  DB_COLUMNS: {
    'tracks.cover_data':
      '死列：Rust 侧只写 NULL（db/library.rs:35 回填 None、:425 的 INSERT 写 NULL），全仓 SELECT 从不读（读封面走 cover_path + 文件系统）。前端 Track.coverData 因此恒为 null，靠 bridge/ipc.ts:34 的「?? coverPath」兜底。Electron 时代封面入库的遗留，保留只为老库零迁移。',
    'tracks.album_cover_data':
      '死列：同 tracks.cover_data（db/library.rs:45 回填 None、:425 写 NULL），SELECT 列表里从未出现；专辑封面走 albums.cover_path。',
    'albums.cover_data':
      '死列：唯一写入点是 db/library.rs:418 的 INSERT ... cover_data) VALUES (?1,?2,?3,?4,NULL)，两处 SELECT（:82、:347）只取 cover_path。与 tracks.cover_data 同源遗留。',
    'folder_nodes.cover_data':
      '死列：唯一写入点是 db/library.rs:525 的 INSERT 写 NULL，两处 SELECT（:134、:394）都不取它；目录封面走 folder_cover_map 缓存文件（utils/folderTree.ts 在前端回填）。',
  },

  /** dto.ts 里有、但不来自同名列的字段。键格式 `<接口>.<字段>`。 */
  DTO_FIELDS: {
    'FolderNode.children':
      '不是列：Rust 侧按 folder_nodes.parent_path 递归组树后的嵌套数组（db/library.rs:181-202）。',
    'FolderNode.tracks':
      '不是列：曲目归属来自 folder_tracks 关联后回填（db/library.rs:164-170）。',
    'Category.trackCount':
      '子查询派生：(SELECT COUNT(*) FROM category_items WHERE category_id = c.id) AS track_count（taxonomy.rs:58），categories 表没有这一列。',
    'Category.children':
      '不是列：Rust 侧由扁平列表按 parent_id 组树（taxonomy.rs:83-110 build_category_tree）。',
  },

  /**
   * Rust SQL 里出现、但不属于任何表的标识符。键就是字面量。
   * 目前为空：db/*.rs 生产代码里没有这类引用（唯一一处 rowid 在 db/library.rs 的测试辅助函数里，
   * 而测试整体不参与对账）。SQLite 的隐式 rowid、内建目录表（sqlite_master 等）由脚本自己识别。
   */
  RUST_SQL_COLUMNS: {},
} as const

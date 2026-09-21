# KX-Player 架构（实现版）

> 本文是 `docs-vibecoding/02-architecture.md` 的**实现版**：写的是代码现在长什么样，不是应该长什么样。
> 凡是与规范不一致的地方一律标 **⚠️ 差距** 并给出证据（`文件:行`）。无法从代码确认的写「现状如此」。
> 核对方式：逐个读 `src-tauri/src/` 与 `src/`，未跑构建（同工作区有并行改动）。
> IPC 命令/事件的完整清单由脚本生成在 `docs/IPC.md`，本文只讲链路与约束。

---

## 1. 运行形态

Tauri 2 桌面应用：单个 `kx-player.exe` + WebView2 渲染前端，前端与 Rust 之间只有 IPC。
**无 Node 运行时**（Node 只在构建期用）。

窗口（`src-tauri/tauri.conf.json`）：

| 项 | 值 | 出处 |
|---|---|---|
| label | `main` | tauri.conf.json:15 |
| 默认尺寸 | 1200 × 800 | tauri.conf.json:17-18 |
| 最小尺寸 | 900 × 600 | tauri.conf.json:19-20 |
| 装饰 | `decorations: false`（自绘标题栏） | tauri.conf.json:21 |
| 托盘 | Rust 侧建（显示窗口 / 退出），关窗 = 隐藏到托盘 | lib.rs:238-259 |

⚠️ **差距**：规范 `02 §1.3` 要求主窗口 `transparent: true`，且 `html, body { background: transparent }`。
现状：`tauri.conf.json` 里没有 `transparent` 字段（默认 false），`src/styles/base.css:12` 是
`background: var(--bg)`（不透明）。**只有 pip 悬浮窗**内部靠 DOM 透明让视频透出，主窗口不是透明窗。

⚠️ **差距**：`pip` 窗口**不在** `tauri.conf.json` 的 `windows[]` 里声明，而是运行期在
`commands/player.rs` 用 `WebviewWindowBuilder` 动态创建，首次 `pip_open` 才建、之后复用实例
（`pip_close` 只 `hide()` 不销毁，见 commands/player.rs:276-280）；`CloseRequested`/`Destroyed` 只
`prevent_close` + `hide()`（lib.rs:68-79）。capabilities 同时覆盖 `main` 与 `pip`
（`capabilities/default.json:5`）。

### Tauri features（Cargo.toml:17-22）

开了 `tray-icon`、`protocol-asset`、`image-png`、`common-controls-v6`。
⚠️ **没有开 `devtools`**（规范 `§1.4` 要求）。

---

## 2. 技术栈与版本（实测，非规范值）

### 2.1 前端（`KX-Player/package.json`）

| 包 | 实际声明 | 规范要求 | 备注 |
|---|---|---|---|
| vue | `^3.5.43` | 3.5.4 | ✅ |
| pinia | **`^4.0.3`** | 2.2.2 | ⚠️ 大版本不同 |
| @tauri-apps/api | `^2.11.1` | 2.11.1 | ✅ |
| vite | **`^8.1.0`** | 5.4.x | ⚠️ 大版本不同（`README.md` 写的「Vite 5.4」也不对） |
| typescript | `^5.9.3` | 5.9.3 | ✅ |
| vue-tsc | `^3.3.11` | 3.x | ✅ |
| vitest | `^5.0.1` | 3.x | ⚠️ |
| @vueuse/core | `^15.0.0` | 12.0.1 | ⚠️ |
| **pinyin-pro** | `^3.28.1` | **规范 §1.6 明确禁止前端引入** | ⚠️ 见 §3.4 |
| **chinese-conv** | `^4.0.0` | **同上禁止** | ⚠️ 同上 |

两个被禁的搜索库是**运行时真在用**（不是残留依赖）：`src/App.vue:140` 首帧后
`loadSearchLibs()`，`src/utils/fuzzy.ts` 消费它们做繁简/拼音匹配；`vite.config.ts:55-58` 还把二者
`optimizeDeps.exclude` 并单独打成 `vendor-search` 分块（`vite.config.ts:78-80`）。
也就是说：**筛选与搜索目前整体在前端内存里做**，Rust 侧没有搜索实现。

### 2.2 后端（`KX-Player/src-tauri/Cargo.toml`）

| crate | 实际 | 规范要求 | 备注 |
|---|---|---|---|
| tauri | `2.11` | 2.11.5 | ✅（feature 见上） |
| tauri-build | `2.6` | — | ✅ |
| libmpv2 | `6.0` | 6.0 | ✅ |
| rusqlite | `0.40`（`bundled`） | 0.32 | ⚠️ 更高 |
| lofty | `0.25` | 0.22 | ⚠️ |
| reqwest | `0.12`（`json`+`blocking`+`native-tls`，关默认特性） | 0.13.5 | ⚠️；且用 `blocking::Client`（ai.rs:5）而非 async |
| tokio | `1.53`（`rt-multi-thread,fs,sync,process,time,macros`） | — | ✅ |
| uuid | `1`（**只开 `v5`**） | 需 v5 + v7 | ⚠️ 没开 v7；任务 ID 用自增 `u64`（system.rs:110） |
| blake3 / parking_lot / crossbeam-channel / base64 / once_cell / thiserror / walkdir / notify / image / encoding_rs | Cargo.toml:26-40 | — | ✅ 均在 |
| **tracing / tracing-subscriber** | **无** | §1.1 要求结构化日志 + 1MB 轮转 | ⚠️ 见 §7.1 |
| **pinyin** | **无** | §1.1 要求（拼音索引下沉 Rust） | ⚠️ 见 §2.1 |
| **tauri-plugin-global-shortcut** | **无** | §1.1 要求（媒体键） | ⚠️ 媒体键未实现 |
| tauri-plugin-single-instance | `2.4` | 要求 | ✅ lib.rs:30 |
| tauri-plugin-dialog | `2.7` | 要求 | ✅ lib.rs:29 |
| windows-sys | `0.59`（6 个 Win32 feature） | 要求 | ✅ 仅 `[target.'cfg(windows)'.dependencies]`，Cargo.toml:44-52 |

`[package]`：`name = "kx-player"`、`version = "2.0.0"`、**`edition = "2021"`**、`rust-version = "1.88"`。
`[lib]` 的 `crate-type = ["staticlib", "cdylib", "rlib"]`。
工具链由 `rust-toolchain.toml` 锁 `1.98.1`。

⚠️ **差距**：规范 `§1.2` 要求 `edition = "2024"`，实际 `2021`。
⚠️ **差距**：`assembly name` 与 `productName` 不一致也没关系，但注意 lib 名是 `kx_player_lib`
（`main.rs` 调 `kx_player_lib::run()`）。

---

## 3. 真实目录结构（按当前代码形状）

```
KX-Player/
├─ index.html
├─ package.json / tsconfig.json / vite.config.ts
├─ scripts/
│  ├─ gen-ipc-doc.mjs        # IPC 四方对账 + 生成 docs/IPC.md
│  ├─ check-ffmpeg.mjs       # ffmpeg 能力校验
│  └─ scan-imports.mjs
├─ docs/                     # IPC.md（脚本生成）
├─ tests/                    # vitest：collections/lyrics/matcher/playback/queue/result
├─ src/                      # 前端（Vue 3 + Pinia）
│  ├─ main.ts / App.vue / env.d.ts
│  ├─ bridge/ipc.ts          # ★ 唯一 invoke / listen 出口（318 行）
│  ├─ contracts/
│  │  ├─ commands.ts         # ★ 命令名 + 参数键 + resolve 类型（编译期约束）
│  │  ├─ events.ts           # ★ 事件名 + 载荷
│  │  ├─ dto.ts              # DTO 单一来源
│  │  ├─ result.ts           # AppError / AppErrorCode 归一化
│  │  ├─ api.ts              # AppApi 方法形状（re-export dto）
│  │  └─ pip.ts              # PIP_GUTTER 前后端共用常量
│  ├─ stores/                # pinia：library / player / playlists / settings / taxonomy / ui
│  ├─ features/
│  │  ├─ shell/              # TitleBar / Sidebar / PlayerBar / QueuePanel
│  │  ├─ library/            # ContentArea / SmartView / FolderView / RecentView / TrackTable
│  │  ├─ inspector/          # InspectorPanel（队列/字幕/信息/书签）
│  │  ├─ stage/              # StageView（影院）
│  │  ├─ lyrics/             # LyricsView
│  │  └─ search/ playlists/ settings/ taxonomy/ convert/ ai/
│  ├─ components/            # VirtualGrid / VirtualList / MediaCover / PipRoot / ToastHost
│  │                         # ContextMenuHost / SelectMenu / ConfirmDialog
│  ├─ composables/           # useKeyboardShortcuts / useThemeEffect / useTrackActions
│  ├─ services/              # confirmHost.ts / aiTranslate.ts（前端 AI 翻译编排）
│  ├─ utils/                 # layout / collections / covers / fuzzy / format / queue
│  │  │                      # playback / speedPresets / folderTree / color / lyricsLoader
│  │  └─ parsers/            # lyrics.ts（LRC/SRT/VTT 解析）/ matcher.ts
│  └─ styles/                # tokens.css / base.css / theme.css / motion.css
└─ src-tauri/
   ├─ Cargo.toml / tauri.conf.json / rust-toolchain.toml
   ├─ capabilities/default.json
   ├─ migrations/            # 001_init.sql / 003_taxonomy.sql
   └─ src/
      ├─ main.rs             # 只调 kx_player_lib::run()
      ├─ lib.rs              # 装配：插件/托盘/单实例/★命令注册表（269 行）
      ├─ model.rs            # 跨 IPC 的 DTO（camelCase serde）
      ├─ error.rs            # IpcError / AppErrorCode
      ├─ state.rs            # AppWindowsState（force_close 等）
      ├─ paths.rs            # 数据目录/日志/ffmpeg 定位
      ├─ settingsio.rs       # 设置 JSON 读写
      ├─ bgimage.rs          # 背景图存取 + 超大图迁移压缩
      ├─ fftools.rs          # ffmpeg 执行器 + 转换队列 + convert:progress
      ├─ ffprobe.rs          # ffprobe 探测
      ├─ ai.rs               # OpenAI 兼容 chat/completions（reqwest blocking）
      ├─ taxonomy.rs         # 分类/标签 SQL + 自动打标词表
      ├─ watcher.rs          # notify 文件监听 → scanner:fsChanged
      ├─ commands/           # dialog_fs / library / player / system / taxonomy + mod.rs
      ├─ db/                 # mod.rs / migrations.rs / library.rs / progress.rs
      ├─ player/             # mod.rs（PlayerCore + mpv 事件泵）/ embed.rs（Windows 覆盖窗口）
      └─ scanner/            # mod.rs（扫描编排）/ meta.rs（元数据）/ covers.rs（封面）
```

`scanner/mod.rs` 是**单个 964 行**的文件，扫描全流程（发现 → 并行元数据 → 分组 → 封面落盘 → 入库）
都在里面。`db/library.rs` 750 行，`player/mod.rs` 577 行，`taxonomy.rs` 426 行。

ℹ️ Electron 版遗留实现（`electron/` 8 个文件、`build/uninstall.nsh`、4 个一次性基准脚本）已移出仓库，
只存在于 git 历史；本机上可能还残留 `dist-electron/`、`dist_electron/`、`test-media/` 等旧输出目录，
它们不参与 Tauri 构建，也已在根 `.gitignore` 里挡住。若去历史里翻旧实现，注意别把 Electron 版当现状。

---

## 4. 与规范 `02 §4` 四层分层（domain / infra / services / commands）的差距

**结论：现在不是四层，是「crate 根平铺模块 + commands/db/player/scanner 四个子目录」。**
`src-tauri/src/` 下**没有** `domain/`、**没有** `infra/`、**没有** `services/` 三个目录。

| 规范层 | 现状 | 证据 |
|---|---|---|
| `domain/`（模型 + 端口 trait + error，无 IO） | **不存在**。等价职责散在三处：`model.rs`（DTO + `normalize_path`）、`error.rs`（`IpcError`/`AppErrorCode`）、`taxonomy.rs`（分类树/标签/自动打标，**直接在函数里开库**） | `taxonomy.rs:53` 起全是 `pub fn ... -> Vec<CategoryDto>` 直接读写 SQLite |
| `infra/`（paths/logging/db/text/media/video 具体实现） | **不存在独立层**。`paths.rs`、`settingsio.rs`、`bgimage.rs`、`db/*`、`scanner/meta.rs`、`scanner/covers.rs`、`fftools.rs`、`ffprobe.rs`、`ai.rs` 各自平铺 | `paths.rs:44` `log_path()`、`db/library.rs` |
| `services/`（编排，只依赖端口 trait） | **不存在独立层**。编排在 `scanner/mod.rs`（扫描）、`player/mod.rs`（播放状态机 + mpv 适配**合体**）、`fftools.rs`（转换队列） | `player/mod.rs:9` 同时 import `libmpv2::{Format, Mpv}` 与 `crate::model` |
| `commands/` | ✅ **存在**，但**不是薄壳**：是「参数校验 + 加锁 + spawn_blocking + 错误收口」的组装点 | `commands/player.rs:15-34`、`commands/mod.rs:11-19` |

**具体违背的红线**（`05 §3` 第 6 条「`services/` 无具体实现类型名」）：

- `player/mod.rs` 里的 `PlayerCore` **同时**是「领域状态机」和「libmpv2 适配器」——文件顶部直接
  `use libmpv2::{Format, Mpv}`（`player/mod.rs:9`），并在 `ensure_started()` 里逐条
  `set_option`（`player/mod.rs:97-105`）。规范要的是 `services/player/PlayerService` 只依赖
  `dyn MpvPort`、由 `infra/mpv/*` 实现端口；现状**没有端口 trait**，无法替换实现，
  也没法给状态机写纯单测。
- `taxonomy.rs` 把「业务词表」和「SQL」放同一文件同一层（`taxonomy.rs:379-414` 是词表，
  `taxonomy.rs:112` 起是建表/增删改），既不是 domain 也不是 infra。
- `commands/` **没有业务逻辑**这半条是**达标的**：在 `commands/` 下 grep
  `SELECT|INSERT|UPDATE|DELETE` **零命中**，所有 SQL 都在 `db/` 与 `taxonomy.rs`。
  子命令确实只转发（例：`commands/library.rs` 只做 `db_path()` + `spawn_blocking` + `join_task`）。

**另一处功能性差距**：规范 `05 §1` 第 4 步要求 `infra/video/ffprobe.rs` + `services/scanner.rs`，
现状是 `ffprobe.rs`（63 行）+ `scanner/meta.rs`（177 行）探测，编排全在 `scanner/mod.rs`（964 行）。

---

## 5. IPC 链路

### 5.1 一次调用的完整路径

```
视图组件 (.vue)
   │  调用 pinia store 的 action / api 方法
   ▼
src/stores/*.ts                       ← 业务状态与编排
   │  api.xxx(...)
   ▼
src/bridge/ipc.ts  invoke(cmd, args)  ← ★ 全应用唯一的 invoke 出口
   │  tauriInvoke(cmd, args)            （@tauri-apps/api/core）
   ▼  [Tauri IPC: JSON]
#[tauri::command] async fn xxx(...)   ← src-tauri/src/commands/*.rs
   │  spawn_blocking + join_task 收口错误
   ▼
db::* / scanner::* / player::PlayerCore / fftools::* / taxonomy::* / ai::*
```

### 5.2 契约的四份来源与「唯一出口」约束

`src/bridge/ipc.ts` 是全应用**唯一**允许出现 `invoke` / `convertFileSrc` 的文件（文件头注释即此约定，
`scripts/scan-imports.mjs` 用于检查裸导入），也是唯一允许 `listen` 的地方
（`bridge/ipc.ts:302-318` 的 `onEvent<T>()`）。

契约分散在 4 个文件，各自单一职责：

| 文件 | 内容 | 作用 |
|---|---|---|
| `contracts/commands.ts` | `CommandSpecs`：命令名 → `{ args, returns }` | **编译期**约束。`bridge/ipc.ts:8-9` 用 `ArgsOf<K>`/`ResolveOf<K>` 取出，键名写错立刻红 |
| `contracts/events.ts` | 事件名 → 载荷类型 | 与 Rust `emit` 名字对账 |
| `contracts/dto.ts` | 所有跨 IPC 结构 | 与 Rust `model.rs` 的 `#[serde(rename_all = "camelCase")]` 对齐 |
| `contracts/result.ts` | `AppError` / `AppErrorCode` | 把 reject 载荷归一化 |

**失败语义**：Rust 返回 `Err(IpcError)` → Tauri **reject** → `invoke()` 的 catch
`throw toAppError(e)`（`bridge/ipc.ts:16-22`）。`error.rs:1-6` 记录了为什么**不能**改成
「resolve 一个 `{ok:false}`」：`bridge/ipc.ts` 里有大量 `.catch(fallback)`，信封一改，
这些 catch 全成死代码、配套 `.then(() => true)` 会把失败当成功 —— 静默反转。

**参数键名**：Tauri v2 把 Rust 的 `snake_case` 形参归一化成 `camelCase` 的 JS 键。
`gen-ipc-doc.mjs:31-32` 的 `toCamel` 就是这条规则的实现。所以 `track_path` → 前端必须传
`trackPath`。`README.md` 的 FAQ 专门记了这个坑（还有个 `base_url` → `baseUrl` 而非
`baseURL` 的例子）。

### 5.3 命令规模

`lib.rs:134-230` 的 `generate_handler!` 注册了 **95 条命令**：

| 模块 | 条数 | 域 |
|---|---|---|
| `commands/dialog_fs.rs` | 12 | 文件对话框、磁盘读写、剪贴板、资源管理器定位 |
| `commands/library.rs` | 21 | 扫描、进度、响度、书签、封面、文件监听 |
| `commands/system.rs` | 19 | 设置、背景图、窗口控制、自检、ffmpeg、转换队列、AI |
| `commands/player.rs` | 24 | 播放遥控、字幕属性、音量设备、舞台矩形、pip 五件套 |
| `commands/taxonomy.rs` | 19 | 分类树、标签、建议打标 |

### 5.4 一致的 `join_task` 收口

`commands/mod.rs:11-19` 把 `spawn_blocking(...).await` 的**双层 Result** 收口：外层是
「后台任务 panic / 被取消」（`JoinError`）→ `IpcError::join_failed` 装进 `detail`，码为
`Internal`；内层业务错误原样穿过。这是新增命令时必须复用的样板。

---

## 6. 事件机制（Rust → 前端）

**单向广播**，前端只 `listen`，没有回调通道。全部 13 个事件（grep `emit("` 实测）：

| 事件 | 载荷 | 生产者 | 消费方 |
|---|---|---|---|
| `player:state` | `PlayerState`（= `MpvPlayerState`） | player/mod.rs:78、:613 | stores/player.ts、PipRoot.vue |
| `player:loading` | `null` | player/mod.rs:557 | stores/player.ts |
| `player:ended` | `{ reason }` | player/mod.rs:573 | stores/player.ts、PipRoot.vue |
| `player:error` | `{ message }` | player/mod.rs:576 | stores/player.ts |
| `scanner:progress` | `{ completed, total }` | scanner/mod.rs:87 | App.vue:151 → library store |
| `scanner:stage` | `string` | scanner/mod.rs:91 | App.vue:155 |
| `scanner:fsChanged` | `null` | watcher.rs:35 | App.vue:159 |
| `convert:progress` | `{...}`（7 处发） | fftools.rs:285-336 | convert 视图 |
| `window:beforeClose` | `null` | lib.rs:262、system.rs:80、player.rs:184 | App.vue:169（落盘设置） |
| `window:maximizeChange` | `boolean` | lib.rs:108 | TitleBar |
| `pip:shown` / `pip:closed` / `pip:restored` / `pip:pinned` | — | player.rs:247-306、lib.rs:73-78 | ui store、PipRoot |

**节流约定**：`player:state` 有 100ms 下限（规范 `02 §2.3` 的 10Hz）——
`PlayerCore::emit_state`（`player/mod.rs:70-79`）用**系统毫秒**，
`emit_state_shared`（`player/mod.rs:613` 附近）用 **thread_local 的 `Instant`**，
**两套独立节流**。前者被 `play_file` 等主动路径用，后者被事件泵用；
两条路径的「上次发送时间」互不可见，所以强制路径与非强制路径混用时会各自放行一次。
⚠️ 这是「同一语义两种实现」的小隐患，合并成一个更好。

**启动自检的时序坑（代码已解决）**：`setup` 阶段 WebView 还没建好，`emit` 会被丢掉。
所以自检告警不靠事件，而是前端首屏主动 `api.startupWarnings()` 取一次（`App.vue:142-147`）。

---

## 7. 数据库与迁移

### 7.1 位置与文件

`paths.rs` 决定一切，**刻意沿用 Electron 版的 userData 目录**，老用户原地继承：

| 用途 | 路径 |
|---|---|
| 数据目录 | `%APPDATA%/kx-music-player/Cache/kx-music-player/`（paths.rs:12-14） |
| 主库 | `kx-player-library.sqlite` |
| 设置 | `kx-player-settings.json` |
| 封面 | `covers/` |
| 背景图 | `kx-player-bg.png` |
| 日志 | `kx-player-log.txt`（paths.rs:96-121） |

⚠️ **日志轮转的现状**：`append_log` 在文件 >1MB 时**只保留尾部 512KB 并原地覆盖**，
不是滚成多个文件；且截断点按**字节**切（`paths.rs:116-117` 用 `content.len()` + 字符串切片）——
日志含中文时理论上可能切在多字节字符中间。`read_to_string` 保证读到的是合法 UTF-8，
但 `&content[keep_from..]` 的 `keep_from` 是纯字节偏移，**这是潜在 panic 点**，建议改为
`char_indices` 取边界。
⚠️ **差距**：规范 `§1.1` 要 `tracing` 结构化日志。现状是自研 `append_log(&str)` + 手写
`epoch_to_utc`（`paths.rs:123-140`，注释写「避免引入 chrono」），无级别、无 span、无 JSON。

### 7.2 迁移机制

`db/migrations.rs`：DDL **外置**在 `src-tauri/migrations/*.sql`，用 `include_str!` 编进二进制
（`migrations.rs:1`），运行时按版本号重放未应用部分，用 `schema_migrations` 记账
（`migrations.rs:41`）。

```rust
pub(crate) const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "001_init.sql",    include_str!("../../migrations/001_init.sql")),
    (3, "003_taxonomy.sql", include_str!("../../migrations/003_taxonomy.sql")),
];
```

⚠️ **差距（有意为之）**：**版本 2 缺席**。规范 `05 §2.1` 要求 `002_search_index.sql`（FTS5，
>20000 条时启用）。`migrations.rs:11` 的注释明确写着「留给筛选下沉那条改动」。
所以现在只有 `001` 与 `003` 两个 SQL 文件。

**老库接管**：`migrations.rs:100-122` 有单测
`apply_adopts_a_preexisting_unversioned_database` —— 没有 `schema_migrations` 但表已存在时
**接管而不是报错**（所有语句都是 `IF NOT EXISTS`）。

**启动时的旧 ID 改写**：`lib.rs:42` `db::migrate_legacy_ids(...)` 把
`md5(路径)前 12 位` 的旧条目 ID 全量改成 **UUIDv5**（含封面文件名与 settings 引用），
改写前把库与设置各备份成 `*.pre-uuid5`。这也是 `Cargo.toml` 只开 `uuid/v5` 的原因。

### 7.3 表结构（`001_init.sql` + `003_taxonomy.sql`）

```
library_meta(key, value)
artists(artist_id, name, root_path)
albums(album_id, artist_id, name, artist_name, cover_path, cover_data)
tracks(id, artist_id, album_id, name, path, duration, artist, album, format,
       is_video, cover_path, cover_data, lyrics_path, file_mtime, file_size,
       meta_title, meta_artist, genre, bitrate, sample_rate, album_cover_data, loudness_lufs)
folder_nodes(path, name, parent_path, track_count, cover_data)
folder_tracks(folder_path, track_id)
play_progress(track_id, position_ms, completed, play_count, played_at, last_speed)
bookmarks(id, track_id, at_ms, label, created_at)
categories(id, parent_id → categories ON DELETE CASCADE, name, icon, color, sort_index, created_at)
category_items(category_id, track_id, added_at)          -- 复合主键
tags(id, name UNIQUE, color, kind DEFAULT 'topic', use_count)
```

**差异写入的实际做法**：没有独立的 diff/delta 层。`play_progress` 走
`INSERT ... ON CONFLICT(track_id) DO UPDATE` 式 upsert，由 `db/progress.rs`（142 行）收口。
`001_init.sql:68` 的注释写明「≥95% 记为听完**由前端判定**，这里只存事实」——
判定逻辑在前端 `src/utils/playback.ts`（`completedOnWrite`）与
`src/utils/collections.ts`（`COMPLETED_RATIO`）。

⚠️ **差距（P0-27）**：规范 `02 §2.4` 要求「字幕偏移随文件记忆（存 `subtitle_offsets` 表，
加载时回填）」。**该表与相关代码完全不存在**：全仓 grep `subtitle_offsets` **零命中**，
`src/stores/player.ts:49` 的 `subtitleDelay` 只是内存 `ref`，切歌即归零。

---

## 8. 播放内核（libmpv2 + mpv 嵌入）

### 8.1 事实源与职责划分

**mpv 是播放状态的唯一事实源**。Rust 侧 `PlayerCore`（`player/mod.rs`，577 行）只做三件事：
转发命令、订阅属性、把事件归一化成 `player:state`。前端 `stores/player.ts` 只**镜像**状态
（`applyMpvState`）。

```
前端 UI（遥控器） → commands/player.rs → PlayerCore → libmpv2 (libmpv-2.dll)
                                             ↓ 事件泵
                    player:state / player:ended / player:error / player:loading
```

### 8.2 mpv 初始化选项（`player/mod.rs:97-105`，逐条实测）

| 选项 | 值 | 说明 |
|---|---|---|
| `wid` | 覆盖窗口 HWND | 嵌入目标 |
| `vo` | `gpu-next` | ✅ 符合规范 |
| `hwdec` | `auto-safe` | ✅ 符合规范（硬解失败回落软解） |
| `gpu-api` | `d3d11` | ✅ 符合规范 |
| `keep-open` | `yes` | |
| `force-window` | **`no`** | ⚠️ 规范要求 `yes` |
| `osc` | `no` | 自绘控制条，禁用 mpv 自带 OSD |
| `input-default-bindings` / `input-vo-keyboard` | `no` | 键盘归前端 |
| `audio-display` | 见源码 | |

⚠️ **差距**：规范要求 `force-window=yes`，实际 `no`。
⚠️ **差距**：规范要求 `no-config=yes`、`no-terminal=yes`，实际**都没设**。
因为没设 `no-config`，用户的 `mpv.conf` 会参与，mpv 自带默认（含 `sub-auto`）也生效 ——
这**恰好在事实上**撑起了「同名外挂字幕自动加载」这条能力（见 §8.4）。
（规范强调 `config=no` 必须写在 `Mpv::with_initializer` 里 **第一个** `set_option`，
加回时留意顺序。）

### 8.3 属性订阅 vs 规范清单

规范 `02 §2.3` 要求观察：`pause` · `time-pos` · `duration` · `filename` · `volume` ·
`mute` · `speed` · `sid` · `secondary-sid` · `track-list/count`。

**实测只订阅 6 个 + 2 个视频尺寸**：`time-pos`、`duration`、`pause`、`speed`、`volume`、
`mute`，外加 `video-params/dw`、`video-params/dh`（给 pip 算宽高比）。
**没订阅** `filename`、`sid`、`secondary-sid`、`track-list/count` —— 这些改成**按需同步查询**
（如 `subtitle_tracks()` 直接读 `track-list`）。

事件处理：`PropertyChange` / `StartFile` / `FileLoaded` / `EndFile` / `Shutdown`。
**没有** `MPV_EVENT_COMMAND_REPLY`（没用异步命令回复，不需要）。

⚠️ **差距（P0-26 双语字幕）**：全仓 grep `secondary-sub` / `secondary_sid` **零命中**。
「同时加载两条轨（`sub` + `secondary-sub`）」**未实现**；`player_set_subtitle_track`
只设单条 `sid`（`player/mod.rs:236`）。随之 `sub-secondary-*` 系列属性与
「mpv 无 `secondary-sub-font-size`、次轨字号只能整体缩放」这个已知限制在代码里**都还不存在**。

### 8.4 字幕：现状与规范的错位

**已实现**（`commands/player.rs:88-110` + `player/mod.rs`）：
`player_subtitle_tracks`（枚举 mpv 的 `track-list`）、`player_set_subtitle_track`（设 `sid`）、
`player_set_subtitle_visible`（`sub-visibility`）、`player_set_subtitle_delay`（`sub-delay`）、
`player_set_sub_style`（映射 `sub-font` / `sub-font-size` / `sub-color` /
`sub-border-color` / `sub-border-size` / `sub-shadow-offset` / `sub-pos`，
加 P0-24 的 `sub-back-color` 与 P0-25 的 `sub-ass-override`）。

**未实现**（规范 `03 §1.3` P0-22~P0-29 中的若干项，代码里查不到对应调用）：

| 规范项 | 现状 |
|---|---|
| 外挂字幕加载 | **没有 `sub-add` 调用**（全仓 grep 零命中）。靠 mpv 未设 `no-config` 时的默认 `sub-auto` 兜底 → **文件名不同名的外挂字幕不会被加载** |
| 字幕自动匹配（同名 → 同目录唯一 → 末尾公共子串 ≥5 → 字幕目录搜索 + 候选选择器） | Rust 侧**不存在**。前端只有歌词用途的 `src/utils/parsers/matcher.ts`（`findBestLyricsMatch`），**不接 mpv** |
| 内封抽取 / 字幕导出 .srt（P0-29） | **不存在** |
| 编码嗅探（GBK/Shift-JIS/EUC-JP…） | Rust 侧**无** `encoding_rs` 使用点（依赖在 `Cargo.toml:33` 声明了，但字幕链路没消费） |
| 拖入字幕（P0-28） | **不存在**（无 `sub-add`） |

**LRC / 歌词走的是另一条完全不同的路**：前端 `src/utils/lyricsLoader.ts` 自己找文件
（同名 .lrc → 目录模糊匹配 → .vtt/.srt 兜底）→ `utils/parsers/lyrics.ts` **在 JS 里解析** →
`features/lyrics/LyricsView.vue` **自绘浮层**。
⚠️ 这与规范「渲染归 libass、前端不做浮层」（`02 §2.4`）**直接冲突**。`README.md` 把它记成
已知限制，但 `05 §2.2` 的表里仍把「字幕由 libass 渲染」当红线 —— **两份文档口径不一致**。

### 8.5 嵌入方案（Windows 专属）

`player/embed.rs`（262 行）用 Win32 覆盖窗口：

- 独立**舞台线程**创建覆盖窗口并跑消息循环（`lib.rs:94-96` 的注释）：
  「窗口操作只在该线程执行，避免跨线程 `SendMessage` 死锁」。
- 命令**异步入队**（`enqueue(StageCmd::Rect/Visible/Attach/Destroy)`），
  `set_stage_rect` 只入队不阻塞（`embed.rs:296-299`）。
- 覆盖窗口是**原生顶层窗口，恒在 WebView 之上** —— 所以前端有个系统性的应对：
  任何要显示在视频上方的浮层（播放栏弹窗、设置模态、右键菜单、确认框）打开前，
  必须先把覆盖窗口藏起来，否则会被视频盖住。这套机制叫**视频遮挡压制**：
  `ui.videoBlocked`（`stores/ui.ts:67-77`，`pbBlockCount` + `confirmOpen` 计数）驱动
  `StageView.vue:73-80` 的 watcher 隐藏/恢复。
- 悬浮窗与舞台**共用同一个覆盖窗口**：pip 激活时把覆盖窗挂到 pip 的 HWND 上
  （`attach_overlay`），关闭时挂回主窗口（`detach_overlay`）。
  因此**同一时刻只能有一个画面出口** —— 这也是 `StageView.vue:82-95` 在离开舞台时
  主动 `setStageRect(visible:false)` 的原因。

⚠️ **差距**：规范 `03 §1.3` P0-20 要求「嵌入失败自动降级为独立 mpv 窗口」。
现状 `MpvEmbedFailed` 错误码在 `error.rs:27` 有定义，`ensure_started` 失败会返回该错
（`player/mod.rs:96`），但**没有自动降级路径**（不重试、不建独立窗口）。

### 8.6 跨平台

**只支持 Windows**。`embed.rs` 的真实实现全部在 `#[cfg(windows)]` 下（`HWND` /
`SetWindowPos`），`windows-sys` 也在 `[target.'cfg(windows)']` 段。
规范提到的「Linux 无合成器 → 降级」在代码里没有对应分支。

---

## 9. AI 子系统现状

**结论：实现的不是规范 `04-ai-subsystem.md` 描述的东西。**

规范 `05 §1` 第 17 步排的是「批量**重命名**」AI（`services/ai/rename/{decide,normalize}.rs` +
`commands/rename.rs` + `rename_history.json` + 本地 Ollama Provider + 缓存 + 后置校验 +
`ai.exportPrompts`/`ai.importTranslations`）。**这些都不存在**：

| 规范要求的 AI 能力 | 现状 |
|---|---|
| `services/ai/rename/decide.rs`（判定分级） | ❌ 不存在 |
| `services/ai/rename/normalize.rs`（前缀切分/规范化） | ❌ 不存在 |
| `commands/rename.rs` + 冲突检测 + 回滚 + `rename_history.json` | ❌ 不存在（只有给分类改名的 `taxonomy_rename_category` 和重命名文件夹的 `rename_dir`） |
| `infra/ai/{credentials,http}.rs` | ❌ 不存在（凭据**只存内存**，见下） |
| `providers/{ollama,openai,anthropic,gemini}` | ❌ 不存在，只有**一个** OpenAI 兼容客户端 |
| `services/ai/{job,cache,prompt}.rs` + 后置校验 | ❌ 不存在（无缓存、无 prompt 版本、无 SYSTEM 前缀哈希） |
| `ai.exportPrompts` / `ai.importTranslations` | ❌ 不存在 |
| 字幕/歌词翻译闭环 | ✅ **存在**，是 AI 域唯一落地的东西 |

**实际实现（三块）**：

1. **Rust 侧传输层** `ai.rs`（222 行）：`POST {base}/v1/chat/completions`，
   `reqwest::blocking`（不是 async）。三个命令：
   - `ai_chat`（只要正文）
   - `ai_ping`（连通性测试，回显 `model`/`latencyMs`/`totalTokens`；失败时这几个键整键缺席，
     见 `model.rs` 的 `#[serde(skip_serializing_if)]`，前端 `AiPingResult` 里注明为可选）
   - `ai_list_models`（解析 `/v1/models`，兼容 Ollama 的列表形状）

   `normalize_base_url` 去尾斜杠、无版本段时补 `/v1`（`ai.rs:9-22`），
   空值时默认 `http://127.0.0.1:11434`（**本地 Ollama 优先**，符合规范第 ③ 步精神）。
   `brief()`（`ai.rs:46-53`）按**字符**截断而非字节 —— 注释记录了原因：`&text[..n]` 落在多字节
   字符中间会 panic，而 panic 在命令线程上会把「HTTP 状态 + 厂商原文」全丢成一句「任务失败」。
   同批把 `ai_ping` 的 `&reply[..60]` 也统一过来。

2. **前端编排层** `src/services/aiTranslate.ts`（264 行）：递归扫 `.lrc/.srt/.vtt`（深度 ≤4、
   数量 ≤500）→ 解析出可翻译段 → 分块送 LLM → 回写**同目录 `{base}.zh.{ext}`**。
   文件写入走 `tools_save_file`（base64，见 `dialog_fs.rs`）。

3. **视图** `src/features/ai/AiTranslateView.vue`（693 行）。

**凭据处理**：`src/stores/settings.ts:56` 的注释写明「**API Key 仅存内存，不落盘**」，
落盘快照 `snapshot()` 里确实只含 `aiBaseURL` / `aiModel`（`settings.ts:79`），
**没有 apiKey**。⚠️ 这意味着**重启后必须重填 Key**。规范要求 `infra/ai/credentials.rs`
（Windows 凭据管理器），现状没有 —— 属于「更保守的实现」，不是漏洞，但与规范的持久化预期不同。

**没有的东西**（AI 域）：无流式（`stream: false` 硬编码，`ai.rs:77`）、无并发/队列、无重试、
无缓存（`blake3` 在依赖里但 AI 链路没用；规范要求的缓存键
`blake3(model + prompt_version + fragment)` 未实现）、无 prompt 模板文件、
无后置校验（数字多重集比对 / 括号数量比对 / 扩展名保护）、无回滚。

---

## 10. 如何新增一条命令（分步清单）

**核心约束：同一命令要在 4 个地方同步出现，漏一处 `npm run check:ipc` 会红。**
`scripts/gen-ipc-doc.mjs:9-13` 明确列了这四个来源，以及各自会踩的坑。

### ① 写 Rust 命令 —— 挑对模块放进 `src-tauri/src/commands/`

| 放哪 | 判据 |
|---|---|
| `dialog_fs.rs` | 文件对话框、磁盘读写、剪贴板、资源管理器 |
| `library.rs` | 曲库/扫描/进度/书签/封面/文件监听 |
| `system.rs` | 设置、背景图、窗口、自检、ffmpeg、转换、AI |
| `player.rs` | 播放遥控、字幕、音量设备、舞台矩形、pip |
| `taxonomy.rs` | 分类、标签 |

样板（照抄 `commands/player.rs:57-60` 的形状）：

```rust
#[tauri::command]
pub async fn player_set_volume(app: AppHandle, vol: f64) -> IpcResult<()> {
    let app2 = app.clone();
    join_task("设置音量", tauri::async_runtime::spawn_blocking(move || {
        app2.state::<PlayerCore>().set_volume(vol.clamp(0.0, 1.0))
    }).await)
}
```

必须遵守的三条规则（都能在现有代码里找到原因注释）：

- **形参全 `snake_case`**，前端按 `toCamel` 归一化（`gen-ipc-doc.mjs:31-32`）。
  `track_path` → 前端键 `trackPath`。
- **可能阻塞的一律 `async` + `spawn_blocking`**。`commands/player.rs:36-39` 的注释：
  同步命令跑在 Tauri 主线程的窗口消息循环上，一阻塞整个 UI 就「未响应」。
- **错误用 `IpcResult<T>`**，不要 `Result<T, String>`。`spawn_blocking` 的双层 Result
  必须经 `join_task`（`commands/mod.rs:11-19`）收口，否则「后端 panic」和「业务报错」
  会混成同一句话。

### ② 在 `src-tauri/src/lib.rs` 的 `generate_handler!` 里注册（`lib.rs:134-230`）

漏了这一步：命令能编译，但前端 `invoke` 报「command not found」。

### ③ 在 `src/contracts/commands.ts` 的 `CommandSpecs` 加一行

```ts
player_set_volume: { args: { vol: number }, returns: void }
```

- `args` 的键名 = Rust 形参的 camelCase，**少一个键或多一个键都会在调用点编译期报错**。
- `returns` 写的是 **resolve 值**类型；Rust 的 `()` 记作 `void`（线上是 JSON `null`，
  前端不该消费它）。
- 失败**不经** `returns`：`Err(IpcError)` 走 reject，由 `contracts/result.ts` 归一化。
- 涉及新结构体时，先在 `contracts/dto.ts` 加接口，并在 Rust `model.rs` 上加对应的
  `#[derive(Serialize, Deserialize)]` + `#[serde(rename_all = "camelCase")]`。

### ④ 在 `src/bridge/ipc.ts` 的 `api` 对象上加实现

```ts
playerSetVolume: (v) => invoke('player_set_volume', { vol: v }).catch(() => {}),
```

兜底的 `catch` 要显式想清楚（报错是静默还是冒泡）。不要在这里写业务逻辑，
它只做「调用 + 边界适配」（如 `decorateScan` 把封面路径转 asset URL）。

### ⑤ 刷新生成物并自检

```bash
npm run gen:ipc        # 重写 docs/IPC.md（脚本生成，别手改）
npm run check:ipc      # 四方对账，不一致 exit 1
```

### ⑥ 新事件同理，但只有两个来源

Rust `emit("ns:event", payload)` + `src/contracts/events.ts` 加一行。
前端消费一律走 `onEvent<T>(name, cb)`（`bridge/ipc.ts:302-318`），
**不要**在组件里直接 `import { listen }`。

### 检查清单

- [ ] `commands/*.rs` 里写了 `#[tauri::command]`，形参 snake_case，用了 `IpcResult` + `join_task`
- [ ] `lib.rs` 的 `generate_handler!` 已注册
- [ ] `src/contracts/commands.ts` 加了 `CommandSpecs` 一行（args 键名 camelCase、returns 是 resolve 值）
- [ ] `src/bridge/ipc.ts` 的 `api` 加了实现
- [ ] 涉及新结构体 → `contracts/dto.ts` + Rust `model.rs`（`rename_all = "camelCase"`）都改了
- [ ] `npm run gen:ipc` && `npm run check:ipc` 通过
- [ ] 长任务不是同步阻塞，而是「命令返回 taskId + 事件推进」（参考 `convert_run` + `convert:progress`）

---

## 11. 与规范的主要差距速查

| # | 项 | 规范 | 现状 | 严重度 |
|---|---|---|---|---|
| 1 | 四层分层 | domain/infra/services/commands | 只有 `commands/` 一层目录，其余平铺 crate 根 | 结构 |
| 2 | `player/mod.rs` | services 只依赖 `dyn MpvPort` | 状态机与 libmpv2 适配合体，无端口 trait | 结构 |
| 3 | `002_search_index.sql` | 必须有 | 缺席（版本号 2 占位，注释写明留给后续） | 功能 |
| 4 | `subtitle_offsets` 表（P0-27） | 字幕偏移随文件记忆 | 完全不存在，偏移只在内存 | **功能缺失** |
| 5 | 双语字幕（P0-26） | `sub` + `secondary-sub` | `secondary-sub` 零命中 | **功能缺失** |
| 6 | 外挂字幕加载 | `sub-add` + 匹配 + 候选选择器 | **无 `sub-add` 调用**，靠 mpv 默认行为 | **功能缺失** |
| 7 | 字幕渲染归属 | libass，前端不做浮层 | LRC 走前端解析 + 自绘浮层 | 冲突 |
| 8 | `force-window` | `yes` | `no` | 配置 |
| 9 | `no-config` / `no-terminal` | 要求设 | 未设 | 配置 |
| 10 | 主窗口透明 | `transparent: true` | 未设 | 配置 |
| 11 | 媒体键 / 全局快捷键 | tauri-plugin-global-shortcut | 插件不存在 | 功能缺失 |
| 12 | 结构化日志 | tracing + 1MB 轮转 | 自研 append_log + 原地截断 512KB | 实现差异 |
| 13 | 搜索下沉 Rust | 前端禁 pinyin-pro/chinese-conv | 两个库都在用，搜索整体在前端 | 红线 |
| 14 | AI 重命名全链路 | rename/decide/normalize + 缓存 + 后置校验 | 完全不存在，只有翻译 | 排期未到 |
| 15 | 嵌入失败降级（P0-20） | 自动降级独立 mpv 窗口 | 只报 `MpvEmbedFailed`，无降级 | 功能缺失 |
| 16 | 属性订阅清单 | 10 个属性 | 订阅 6 个 + 2 个视频尺寸，其余按需查询 | 实现差异 |
| 17 | edition | 2024 | 2021 | 配置 |
| 18 | devtools feature | 要求开 | 未开 | 配置 |
| 19 | `npm run check` | tsc --noEmit + vue-tsc + check:ipc + clippy | vue-tsc + check:ipc + vitest + clippy（无独立 tsc、多了 vitest） | 脚本 |

---

## 12. 一句话总结

代码是一个**能跑的 Tauri 2 单体**：IPC 契约（`contracts/` ↔ `lib.rs` 注册表 ↔ `bridge/ipc.ts`）
做得扎实、有脚本强制对账；播放内核是真 libmpv2 + Win32 覆盖窗口；
数据库用外置 SQL + `schema_migrations` 版本化。
主要欠账是**分层**（domain/infra/services 三层目录都不存在，`player/mod.rs` 与 964 行的
`scanner/mod.rs` 是最大的两块耦合）、**字幕链路**（无 `sub-add`、无双语、偏移不落库）、
和**AI 重命名全链路**。
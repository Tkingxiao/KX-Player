# KX 音乐播放器

本地音视频播放器，面向大容量 ASMR 曲库。Tauri 2 桌面应用，播放内核为 libmpv。

## 技术栈

| 层 | 选型 |
|---|---|
| 桌面框架 | Tauri 2（WebView2 渲染，Rust 主进程） |
| 播放内核 | libmpv（`libmpv2` crate），`vo=gpu-next` + `hwdec=auto-safe` |
| 后端 | Rust 1.98.1 / edition 2021 |
| 前端 | Vue 3.5 + Pinia 4 + TypeScript 5.9，Vite 8 构建，无 UI 组件库 |
| 存储 | SQLite（`rusqlite`，WAL）+ 整文档 JSON 设置 |
| 元数据 | lofty；编码嗅探 encoding_rs |
| 音频处理 | ffmpeg（可选外部依赖，用于转换/提取/响度分析） |

## 环境要求

- Windows 10/11（依赖 WebView2；窗口嵌入用 Win32 API）
- Rust 1.98.1（`rust-toolchain.toml` 已锁定，`rustup` 自动安装）。`npm run check` 需要 clippy，它**不随 toolchain 自动装**：`rustup component add --toolchain 1.98.1-x86_64-pc-windows-msvc clippy`
- Node.js ≥ 18、npm ≥ 9
- mpv-dev：解压到仓库根的 `.toolchain/mpv-dev/`（要含 `libmpv-2.dll` 与 `mpv.lib`）。运行期那份 dll 由 `build.rs` 从这儿自动铺到 `src-tauri/lib/` 和 exe 同目录，不入库
- `src-tauri/.cargo/config.toml`：复制同目录的 `config.example.toml` 过去，把 `native` 改成自己机器上 `.toolchain/mpv-dev` 的绝对路径。缺它链接期报 `cannot find mpv.lib`；它含机器专属路径，同样不入库
- ffmpeg（**可选，且不随安装包分发** —— 这是 2026-09-22 的决定，不是待办：安装包与仓库都不带它，`tauri.conf.json` 也不写 `bundle.externalBin`）。只有格式转换 / 音频提取 / 响度分析 / 抽帧需要它，谁用谁本机装一份：
  - 要**单文件静态构建**（gyan.dev 的 `ffmpeg-release-essentials.zip` 那一类）。共享构建的 `ffmpeg.exe` 只有 ~0.5 MB、运行时拖着 ~210 MB 的 `av-*.dll`，不能用
    - 落点（`paths::locate_ffmpeg()` 按序找）：`KX_FFMPEG` 环境变量 → exe 同目录 `ffmpeg.exe`（若你手动放了一份）→ 同目录 `ffmpeg/ffmpeg.exe` → `C:\ffmpeg\bin` → PATH。日常最省事的是加进 PATH
  - 装完跑一次 `npm run check:ffmpeg`：它按 `02 §1.2` 的能力清单核 `-codecs`/`-formats`，会打印「找到了哪个 exe、版本、是否 GPL、缺哪些编解码器」，任一项缺失即 `exit 1`
  - 没装也能正常用：应用启动与播放完全不受影响，转换页会显示一条警告并置灰「开始转换」（`ffmpeg_probe` 探测到 `available:false`）

## 开发

```bash
cd KX-Player
npm install
npm run dev          # Vite dev server + Rust 后端，前端热更新、Rust 改动自动重编译
```

本地想双击启动可自行放一个等价 `npm run dev` 的脚本；仓库不代管，因为它总要写死本机路径。

dev server 固定 `127.0.0.1:5173`——不要改回 `localhost`，WebView2 的 IPv6 解析会导致启动时「拒绝连接」。

## 校验与构建

```bash
npm run check:types    # vue-tsc 类型检查
npm run check:ipc      # 命令/事件四方对账（Rust ⇄ generate_handler ⇄ contracts ⇄ ipc.ts）
npm run check:columns  # 列名对账：contracts/columns.ts ↔ DDL ↔ Rust SQL ↔ contracts/dto.ts
npm run check:colors   # 色值对账：DESIGN.md 的 Color 段 ↔ theme.css ↔ 组件里的字面色值
npm run gen:ipc        # 重新生成 docs/IPC.md（改了命令或事件就要跑）
npm run check          # 一条跑全：上面三段对账 + vitest + cargo clippy -D warnings
npm run test           # vitest 单测 + cargo test（Rust 侧用例）
npm run check:ffmpeg   # 校验本机 ffmpeg 是否具备转换页所需编解码能力（环境自检，缺失即 exit 1）
npm run build        # 前端生产构建 → KX-Player/dist

cd src-tauri && cargo check && cargo test

# 出安装包（唯一正确方式）
npm run tauri build -- --bundles nsis
```

产物：

- `KX-Player/src-tauri/target/release/kx-player.exe`
- `KX-Player/src-tauri/target/release/bundle/nsis/KX-Player_2.0.0_x64-setup.exe`

> ⚠️ **不要用 `cargo build --release` 出分发包。** Tauri 用 `custom-protocol` feature 区分生产/开发：`npm run tauri build` 会把 `dist/` 内嵌进 exe（加载 `http://tauri.localhost/`）；`cargo build --release` 不启用该 feature，产出的 exe 仍去请求 devUrl，单独双击报「127.0.0.1 拒绝连接」。判别方法：
>
> ```powershell
> $t=[Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes(".../kx-player.exe"))
> $t -match 'index-[A-Za-z0-9_-]{6,}\.(js|css)'   # True = 生产产物
> ```
>
> 若 `cargo` 不在 PATH，先 `%USERPROFILE%\.cargo\bin` 加入 PATH，否则报 `failed to run 'cargo metadata'`。

## 项目结构

```
KX-Player/                      仓库根
├── KX-Player/                  应用根（前端 + 后端同层）
│   ├── src/                    Vue 渲染进程
│   │   ├── bridge/ipc.ts       ★ 唯一 IPC 出口（所有 invoke/listen 集中于此）
│   │   ├── contracts/          api（AppApi 形状）· dto（结构体镜像）· commands · events
│   │   ├── features/           shell / library / stage / inspector / lyrics / ai / tools
│   │   ├── stores/             Pinia：library player ui settings playlists taxonomy collections
│   │   ├── components/         SelectMenu、PipWindow 等通用组件
│   │   ├── composables/        useKeyboardShortcuts、useThemeEffect 等
│   │   ├── utils/              parsers（歌词/字幕匹配）、fuzzy、covers、playback、queue
│   │   └── styles/             tokens.css（常量）theme.css（色板）base.css（骨架）
│   └── src-tauri/              Rust 主进程
│       ├── src/
│       │   ├── lib.rs          装配：插件、窗口、托盘、命令注册
│       │   ├── commands/       Tauri 命令入口（library / player / system / dialog_fs）
│       │   ├── db/             SQLite 访问层（library / progress）
│       │   ├── scanner/        目录扫描、并行元数据解析、封面提取、文件监听
│       │   ├── player/         libmpv 封装、HWND 窗口嵌入、设备枚举、响度
│       │   ├── fftools.rs      ffmpeg 转换/提取/响度任务队列（可取消）
│       │   ├── ffprobe.rs      能力探测
│       │   ├── ai.rs           OpenAI 兼容 chat/completions（网络只在此处）
│       │   ├── taxonomy.rs     分类树与标签
│       │   ├── bgimage.rs      背景图压缩
│       │   ├── paths.rs        数据目录、日志轮转
│       │   └── model.rs        DTO
│       ├── lib/                构建时由 build.rs 从 .toolchain 铺 libmpv-2.dll，不入库
│       └── tauri.conf.json
└── .ulpi/design/DESIGN.md      设计语言
```

## 功能

**播放**：libmpv 直解音视频、仅音频模式（`vid=no` 且位置不丢）、四种播放模式、进度记忆与完听判定、淡入淡出、变速、睡眠定时、书签、输出设备选择、响度均衡（ffmpeg ebur128，默认关闭）。

**曲库**：文件夹导入、全量/增量/静默扫描、启动离线同步、文件监听、并行元数据解析、乱码修复、长路径与遍历上限保护、封面三级回退（内嵌 → 同目录外挂 → 子孙目录 BFS）、SQLite 持久化。

**界面**：三栏骨架（侧栏可折叠可拖拽）、智能集合、分级分类树、标签系统、组合筛选、文件夹视图（网格/列表 + 虚拟滚动）、视频舞台、独立悬浮播放窗（拖拽/缩放/吸附/钉住）、歌词视图、检查器面板（队列/字幕/信息/书签）、深浅色与主题色、背景图编辑器、无边框窗口 + 托盘。

**工具**：音频格式转换、视频提取音频、转换队列（可取消、同名加序号、保持源声道与采样率）。

**AI**：字幕/歌词批量翻译（LRC 保留时间戳，SRT/VTT 保留 cue）、模型列表拉取、连通性测试。

## 已知限制

- **ffmpeg 不随包（决定，非欠账）**：理由与口径见上文「环境要求」的 ffmpeg 条与 `docs-vibecoding/02 §1.2`。应用侧已具备能力检测（`ffmpeg_probe`）与降级（转换页警告条 + 置灰执行按钮），缺它只影响转换类功能。因为不分发二进制，带 `libx264` 的 GPL v2+ 源码获取义务也不落在本项目上。
- **筛选与搜索在前端执行**：组合筛选/搜索目前是全库载入后 JS 过滤，未下沉为 SQL；大库（万条级以上）的启动与搜索响应会退化。
- 字幕目前由前端解析并自绘浮层，未统一交由 libass 渲染。

以上限制连同其余偏差的完整清单、证据定位与整改顺序，记录在逐批整改账本 `ALIGNMENT-AUDIT-*.md` 与交接笔记 `HANDOVER*.md` 里；这两类是开发过程的本地笔记（含机器路径与逐批决策往来），不随仓库分发。

## 故障排查

| 现象 | 处理 |
|---|---|
| 启动白屏 / 「127.0.0.1 拒绝连接」 | 见上文「不要用 cargo build --release」；确认 dev server 已在 `127.0.0.1:5173` 监听 |
| 视频黑屏但有声 | 确认 exe 同目录有 `libmpv-2.dll`。它由 `build.rs` 从 `.toolchain/mpv-dev` 铺设，构建日志里出现「缺少 libmpv-2.dll」warning 就是没铺成 |
| 极少数情况启动白屏 | 清除 `%LOCALAPPDATA%\com.kxplayer.music*` 缓存后重试 |
| 转换/响度分析报「ffmpeg.exe 未找到」 | 安装 ffmpeg 并加入 PATH（或设 `KX_FFMPEG` 指向 exe）—— 它**不随安装包分发**，见上文「环境要求」；`npm run check:ffmpeg` 能一次报清「找到了哪个、缺哪些编解码器」 |
| 新增 Rust 命令前端调不到 | Tauri 按 camelCase 归一化参数名：`base_url` → 前端键名必须是 `baseUrl`（不是 `baseURL`）；同时确认已在 `lib.rs` 的 `invoke_handler` 注册 |
| 升级后进度/收藏对不上 | 条目 ID 已由 `md5(路径)前 12 位` 改为 UUIDv5，启动时由 `db::migrate_legacy_ids` 全量改写（含封面文件名与 settings）。改写前会把 `library.db` 与 `settings.json` 各复制为 `*.pre-uuid5`，异常时用同目录备份整对回滚 |

数据与日志位置由 `src-tauri/src/paths.rs` 决定（Windows 下位于 `%APPDATA%` / `%LOCALAPPDATA%`），日志按 1 MB 轮转。

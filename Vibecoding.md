# KX 音乐播放器 — AI 协作提示词模板

## 项目概述

基于 Electron + Vite 的本地桌面音乐播放器，使用 vanilla JS/CSS，无前端框架。

## 技术栈

| 层 | 技术 | 版本 |
|---|---|---|
| 桌面框架 | Electron | 42 |
| 构建工具 | Vite + vite-plugin-electron | 8 |
| 前端 | vanilla JS/CSS (无框架) | — |
| 数据库 | sql.js (SQLite WASM) | 1.14.1 |
| 图片处理 | sharp | 0.35.2 |
| 元数据 | music-metadata | 11.0.2 |
| 文件监听 | chokidar | 5.0.0 |
| 搜索 | pinyin-pro + chinese-conv | 3.28.1 / 4.0.0 |
| 字符编码 | iconv-lite | 0.7.2 |
| 字体 | @fontsource/inter + @fontsource/noto-sans-sc | — |
| 打包 | electron-builder (NSIS) | 26.15.3 |

## 项目结构

```
KX-Player/
├── electron/                    # Electron 主进程
│   ├── main.ts                 # 主进程入口、IPC 处理器、系统托盘、ffmpeg 集成
│   ├── fileScanner.ts          # 文件夹扫描（Worker 线程池）、文件监听、增量扫描
│   ├── libraryDb.ts            # SQLite 音乐库数据库（sql.js）
│   ├── coverService.ts         # 封面文件服务（sharp 压缩、文件系统存储）
│   ├── preload.ts              # 上下文桥接（main ↔ renderer），暴露 electronAPI
│   └── workers/
│       └── metadata-worker.ts  # Worker 线程（元数据解析、Shift-JIS 修复）
├── KX-Player/                  # 渲染器
│   ├── script.js               # 前端主逻辑（播放、UI、搜索、渲染 ~2954 行）
│   ├── api.js                  # IPC 封装（renderer → electronAPI）
│   ├── utils.js                # 工具函数（DOM 辅助、路径、模糊搜索、时间格式化）
│   ├── virtual-list.js         # 高性能虚拟滚动列表
│   ├── cover.js                # 封面加载与缓存（renderer 侧，批量 IPC，LRU 缓存）
│   ├── style.css               # 全局样式、CSS 自定义属性主题
│   ├── fonts.css               # 字体声明（Inter + Noto Sans SC）
│   ├── index.html              # 前端入口
│   ├── vite.config.ts          # Vite + Electron 构建配置
│   ├── public/                 # 静态资源（favicon、icon）
│   └── package.json
```

## 核心架构

### 状态管理 (S 对象)

```javascript
const S = {
  // === 曲目数据 ===
  all: [],           // 全部曲目数组 (TrackRecord[])
  playingTid: null,  // 正在播放的曲目 ID
  playing: false,    // 是否正在播放
  cTime: 0,          // 当前播放时间（秒）
  dur: 0,            // 当前曲目时长（秒）

  // === 播放列表 ===
  tI: -1,            // 当前曲目在 pl[] 中的索引
  mode: 0,           // 播放模式: 0=顺序 1=随机 2=单曲循环 3=播放完停止

  // === 音量 ===
  vol: 50,           // 音量 0-100
  pVol: 50,          // 上次音量（取消静音时恢复）
  muted: false,      // 是否静音

  // === 播放列表/收藏夹 ===
  favs: [],          // 收藏夹数组 [{id, name, trackIds[], isDefault}]
  pls: [],           // 播放列表数组 [{id, name, trackIds[], coverData}]
  recents: [],       // 最近播放曲目 ID 数组

  // === 视图状态 ===
  view: 'all',       // 当前视图: 'all'|'recent'|'tools'|'lyrics'
  aF: null,          // 当前活跃收藏夹 ID
  aPl: null,         // 当前活跃播放列表 ID
  aI: -1,            // 活跃专辑索引（已废弃）
  alI: -1,           // 活跃列表索引（已废弃）

  // === 文件夹 ===
  folderTree: [],    // 文件夹树结构
  folderStack: [],   // 面包屑导航路径栈
  activeFp: null,    // 当前活跃文件夹路径
  folderSort: 'name',// 文件夹排序: 'name'|'time'|'tracks'
  folderView: 'grid',// 文件夹视图: 'grid'|'list'
  _folderMeta: null, // 预计算的文件夹元数据
  _searchFolders: [],// 搜索结果中的文件夹

  // === 主题 ===
  theme: 'light',    // 主题: 'light'|'dark'
  clr: '#E63A2E',    // 主题色（十六进制）
  ovl: 72,           // 背景图不透明度 0-100
  bgBlur: 0,         // 背景模糊度 0-40
  bgData: null,      // 背景图 base64 数据
  bgPath: null,      // 背景图文件路径
  bgSize: 'cover',   // 背景图尺寸模式

  // === 透明度 ===
  sidebarOpacity: 100,  // 侧边栏透明度
  titlebarOpacity: 100, // 标题栏透明度
  playerOpacity: 100,   // 播放栏透明度

  // === 其他 ===
  q: '',             // 搜索查询字符串
  selMode: false,    // 多选模式
  devId: '',         // 音频输出设备 ID
  listTextColor: null,     // 列表文字颜色
  listTextColorsCached: null, // 缓存的列表文字颜色
  _imgEditState: null,     // 背景图编辑状态
  _syncingView: false,     // 是否正在同步视图
}
```

### 全局变量

```javascript
let fp = []                    // 已导入的文件夹路径数组
let audio = new Audio()        // HTML5 Audio 元素
let lrc = []                   // 当前歌词数组 [{time, text}]
let pl = []                    // 当前播放列表（当前视图的曲目数组）
let nI = 0                     // 当前播放索引

// DSD 播放状态
let dsdState = {
  active: false,      // 是否正在播放 DSD
  path: null,         // DSD 文件路径
  context: null,      // AudioContext 实例
  gainNode: null,     // GainNode 实例
  buffer: null,       // AudioBuffer 实例
  source: null,       // AudioBufferSourceNode 实例
  startedAt: 0,       // 开始播放时间（AudioContext.currentTime）
  pausedAt: 0,        // 暂停时的时间位置
  duration: 0,        // 音频总时长
  raf: 0,             // requestAnimationFrame ID
}

// 扫描状态
let _scanRunning = false    // 是否正在扫描
let _pendingRescan = false  // 是否有待处理的重新扫描
let _loadTGeneration = 0    // 加载曲目代数（用于取消过时的加载）
```

### 播放模式

| 值 | 模式 | 图标 | 说明 |
|---|---|---|---|
| 0 | 顺序循环 | ↻ | 播放完下一首，最后一首播完回到第一首 |
| 1 | 随机播放 | 🔀 | 随机选择下一首 |
| 2 | 单曲循环 | 🔁 | 播放完重新播放当前曲目 |
| 3 | 播放完停止 | ⏹ | 播放完最后一首后停止 |

### IPC 通道 (main.ts ↔ preload.ts ↔ api.js)

#### 对话框类
| 通道 | 方向 | 说明 |
|---|---|---|
| `dialog:openFolder` | renderer → main | 打开文件夹选择器，返回路径数组 |
| `dialog:openImageFile` | renderer → main | 打开图片文件选择器 |
| `dialog:openAudioFiles` | renderer → main | 打开音频/视频文件选择器（多选） |
| `dialog:selectBgImage` | renderer → main | 选择背景图片并读取为 dataURL |

#### 扫描/库类
| 通道 | 方向 | 说明 |
|---|---|---|
| `scanner:scanFoldersWithProgress` | renderer → main | 全量扫描文件夹，返回完整库数据 |
| `library:scanIncremental` | renderer → main | 增量扫描（仅解析新增/修改文件） |
| `library:load` | renderer → main | 加载完整库数据（含封面） |
| `library:loadFast` | renderer → main | 快速加载库数据（不含封面） |
| `library:getCovers` | renderer → main | 批量获取曲目封面（文件系统） |
| `library:loadFolderCovers` | renderer → main | 加载所有文件夹封面 |
| `library:removeFolder` | renderer → main | 从库中移除文件夹（无需全量重扫） |
| `scanner:startWatching` | renderer → main | 启动文件监听（chokidar） |
| `scanner:stopWatching` | renderer → main | 停止文件监听 |

#### 文件类
| 通道 | 方向 | 说明 |
|---|---|---|
| `file:readAsDataURL` | renderer → main | 读取文件为 dataURL |
| `file:readTextFile` | renderer → main | 读取文本文件（支持 UTF-8/GBK 自动检测） |
| `file:exists` | renderer → main | 检查文件是否存在 |

#### 设置类
| 通道 | 方向 | 说明 |
|---|---|---|
| `settings:load` | renderer → main | 加载用户设置（JSON） |
| `settings:save` | renderer → main | 保存用户设置（异步） |
| `settings:syncSave` | renderer → main | 同步保存用户设置（使用 ipcRenderer.send） |

#### 媒体类
| 通道 | 方向 | 说明 |
|---|---|---|
| `media:getAudioDevices` | renderer → main | 获取音频输出设备列表 |
| `media:setAudioDevice` | renderer → main | 设置音频输出设备 |

#### 封面/背景图类
| 通道 | 方向 | 说明 |
|---|---|---|
| `bgImage:load` | renderer → main | 加载背景图（从 userData） |
| `bgImage:save` | renderer → main | 保存背景图（到 userData） |
| `bgImage:remove` | renderer → main | 删除背景图 |

#### ffmpeg/DSD 类
| 通道 | 方向 | 说明 |
|---|---|---|
| `ffmpeg:exec` | renderer → main | 执行 ffmpeg 命令 |
| `dsd:decodePcm` | renderer → main | 解码 DSD 文件为 PCM（返回 base64） |
| `dsd:getTempPath` | renderer → main | 获取 DSD 临时目录路径 |
| `tools:saveFile` | renderer → main | 保存转换后的音频文件 |

#### 窗口控制类
| 通道 | 方向 | 说明 |
|---|---|---|
| `window:minimize` | renderer → main | 最小化窗口 |
| `window:maximize` | renderer → main | 切换最大化/还原 |
| `window:close` | renderer → main | 隐藏窗口到托盘 |
| `window:forceClose` | renderer → main | 强制关闭窗口 |
| `window:isMaximized` | renderer → main | 查询窗口是否最大化 |

#### 工具类
| 通道 | 方向 | 说明 |
|---|---|---|
| `clipboard:writeText` | renderer → main | 写入文本到剪贴板 |
| `shell:showItemInFolder` | renderer → main | 在文件资源管理器中显示文件 |

#### 事件监听类
| 通道 | 方向 | 说明 |
|---|---|---|
| `scanner:progress` | main → renderer | 扫描进度 {completed, total, stage} |
| `scanner:stage` | main → renderer | 扫描阶段文本 |
| `scanner:fsChanged` | main → renderer | 文件系统变化通知 |
| `window:maximizeChange` | main → renderer | 窗口最大化状态变化 |
| `window:beforeClose` | main → renderer | 窗口即将关闭（保存状态） |

### 数据库架构 (SQLite via sql.js)

#### 曲目表 (tracks)
```sql
CREATE TABLE tracks (
  id TEXT PRIMARY KEY,
  name TEXT,
  path TEXT UNIQUE,
  duration REAL,
  artist TEXT,
  album TEXT,
  format TEXT,
  isVideo INTEGER,
  coverPath TEXT,
  coverData TEXT,          -- 不使用，封面存储在文件系统
  lyricsPath TEXT,
  fileMtime REAL,
  fileSize REAL,
  metaTitle TEXT,
  metaArtist TEXT,
  genre TEXT,
  bitrate REAL,
  sampleRate REAL
);
```

#### 封面存储系统
```
userData/
├── covers/
│   ├── {trackId}.jpg           -- 曲目封面（压缩后 ~400px）
│   └── folder_{md5(path)}.jpg  -- 文件夹封面
└── kx-player-library.sqlite    -- 音乐库数据库
```

### 渲染流程

```
初始化:
  init()
    → loadS()                    加载用户设置
    → apTh()                     应用主题
    → apThBg()                   应用背景图
    → loadLibraryData()          加载库数据
      → api.loadLibraryFast()    快速加载（不含封面）
      → applyScanResult()        应用扫描结果
      → api.loadFolderCovers()   加载文件夹封面
      → _preloadVisibleCovers()  预加载可见曲目封面
      → restartWatching()        启动文件监听
    → renderAll()                渲染全部 UI

扫描流程:
  importFolder() / rescan()
    → api.scanFoldersIncremental() / api.scanFoldersWithProgress()
      → IPC → main.ts → fileScanner.ts
        → Worker 线程池解析元数据
        → 返回 FolderNode 树
      → applyScanResult()        应用扫描结果
      → api.loadFolderCovers()   恢复文件夹封面
      → restartWatching()        重启文件监听
      → renderAll()              渲染
```

### 视图系统

| 视图 | view 值 | 说明 |
|---|---|---|
| 全部音乐 | `'all'` | 文件夹卡片/列表视图，支持面包屑导航 |
| 最近播放 | `'recent'` | 最近播放的 200 首曲目 |
| 工具 | `'tools'` | 视频提取音频、音频格式转换 |
| 歌词 | `'lyrics'` | 全屏歌词视图（自动滚动） |

#### 文件夹导航

```javascript
S.folderStack  // 面包屑路径栈，例如 ['D:/Music', 'D:/Music/流行', 'D:/Music/流行/周杰伦']
S.activeFp     // 当前活跃文件夹路径
```

导航函数:
- `navigateFolder(path)` — 进入文件夹
- `navigateFolderUp()` — 返回上级
- `navigateFolderTo(path)` — 跳转到指定层级

## 功能模块详解

### 1. 音频播放

#### 普通音频播放
使用 HTML5 Audio 元素:
```javascript
audio.src = 'file:///' + track.path.replace(/\\/g, '/')
audio.play()
```

#### DSD 播放
DSD 文件 (DSF/DFF) 需要特殊处理:
1. 通过 IPC 调用 `dsd:decodePcm` 解码为 PCM
2. 创建 `AudioContext` + `BufferSource`
3. 手动管理播放状态

关键函数:
- `playDsdTrack(track, seekTime)` — 播放 DSD 曲目
- `stopDsdPlayback(resetTime)` — 停止 DSD 播放
- `syncDsdVolume()` — 同步音量到 DSD GainNode
- `startDsdProgressLoop()` — 启动进度更新循环

### 2. 歌词系统

支持格式:
- **LRC** — 标准歌词格式 `[mm:ss.xx]歌词内容`
- **VTT** — WebVTT 字幕格式
- **SRT** — SRT 字幕格式

歌词加载:
```javascript
async function loadLrcForTrack(t) {
  // 1. 尝试同名 .lrc 文件
  // 2. 尝试 .mp3.vtt 或 .vtt 文件
  // 3. 尝试 .mp3.srt 或 .srt 文件
}
```

歌词渲染:
- 左侧: 封面图
- 右侧: 歌词/信息标签切换
- 自动滚动到当前播放行
- 支持手动滚动暂停（2.2秒）

### 3. 搜索功能

模糊搜索支持:
- 子字符串匹配
- 拼音首字母匹配（如 "zjl" 匹配 "周杰伦"）
- 简繁体自动转换

```javascript
function fuzzyMatch(text, query) {
  // 1. 归一化文本（简繁转换）
  // 2. 直接子字符串匹配
  // 3. 拼音首字母匹配
}
```

### 4. 封面系统

#### 存储
- 曲目封面: `covers/{trackId}.jpg`（压缩后 ~400px，quality=70）
- 文件夹封面: `covers/folder_{md5(path)}.jpg`
- 外部封面文件: 自动发现 cover.jpg/png/webp

#### 加载
```javascript
// renderer 侧 (cover.js)
_getCoverData(track)           // 获取曲目封面（优先内存 → 缓存 → IPC）
_loadCoversForTrackIds(ids)    // 批量加载封面（50ms 延迟批量）
_preloadVisibleCovers(tracks)  // 预加载可见曲目封面
```

### 5. 主题系统

CSS 自定义属性:
```css
:root {
  --bg, --bg-card, --bg-sidebar, --bg-player, --bg-input,
  --bg-hover, --bg-active, --text, --text-sub, --text-muted,
  --border, --accent, --accent-light, --accent-rgb, --accent-bg,
  --modal-bg, --modal-overlay, --radius
}
```

深色/浅色主题通过 `apTh()` 函数动态切换。

背景图编辑:
- 缩放: 10%-400%
- 透明度: 5%-95%
- 模糊度: 0-40px
- 位置偏移: 拖拽调整

### 6. 虚拟列表

高性能滚动列表，仅渲染可见行:
```javascript
virtualList(containerId, items, rowHeight, renderItem, onClick)
// rowHeight: 46px
// buffer: 10 行
```

### 7. 工具模块

#### 视频提取音频
从视频文件提取音轨:
- 输入: MP4/MKV/AVI/MOV/WEBM/FLV/WMV
- 输出: MP3/AAC/FLAC/WAV/OGG
- 使用 ffmpeg `-vn` 参数

#### 音频格式转换
批量转换音频格式:
- 输入: 所有支持的音频格式
- 输出: MP3/WAV/FLAC/AAC/OGG/M4A

## 开发指南

### 关键原则

1. **保持简单** — vanilla JS，不引入框架或构建抽象
2. **直接修改** — 编辑现有文件，不创建新文件除非必要
3. **删除死代码** — 不用的函数/变量/样式直接删除
4. **不引入抽象** — 三行相似代码优于半成品封装
5. **无注释** — 除非 WHY 不明显，否则不写注释

### 常见陷阱

- CSS `background` 简写会重置所有子属性（包括 `background-image`），如需只改颜色用 `background-color`
- Electron 文件操作时进程可能锁定文件（如 app.asar），`rm`/`mv` 可能失败
- `script.js` 中的 `S` 对象是响应式的但不自动触发重渲染，修改后需要手动调用 `renderAll()` 或具体渲染函数
- Worker 线程中的 `require()` 使用 `createRequire` 从 `module.createRequire` 导入
- sql.js WASM 文件需要通过 `extraResources` 打包到 `resources/sqljs/sql-wasm.wasm`
- ffmpeg.exe 需要通过 `extraResources` 打包到 `resources/ffmpeg/ffmpeg.exe`
- 元数据 Worker 有智能解析策略: 大文件 (>10MB) 跳过完整解析，使用文件名信息

### IPC 开发模式

主进程 IPC 处理器在 `electron/main.ts` 中定义，使用 `ipcMain.handle` 注册异步处理器。

渲染进程通过 `preload.ts` 暴露的 `window.electronAPI` 调用，`api.js` 是其封装。

### 状态持久化

用户设置保存到 `userData/kx-player-settings.json`:
- 使用 `schedSave()` 防抖保存（300ms）
- 关闭窗口时同步保存
- 支持 `settings:syncSave` 同步通道（立即写入磁盘）

### 构建命令

```bash
npm run dev          # 开发模式（热重载）
npm run build        # 构建前端
npm run electron:build  # 打包 NSIS 安装程序
```

安装程序输出: `dist_electron/KX音乐播放器 Setup 1.0.4.exe`

### 修改检查清单

改完代码后:
1. `npm run dev` 验证热重载正常
2. 检查受影响的功能路径（如修改了搜索→测试搜索+播放状态同步）
3. `npm run build` 确保构建通过

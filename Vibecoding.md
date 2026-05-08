# KX 音乐播放器 — AI 协作提示词模板

## 项目概述

基于 Electron + Vite 的本地桌面音乐播放器，使用 vanilla JS/CSS，无前端框架。

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面框架 | Electron 28 |
| 构建工具 | Vite 5 + vite-plugin-electron |
| 前端 | vanilla JS/CSS (无框架) |
| 元数据 | music-metadata |
| 文件监听 | chokidar |
| 打包 | electron-builder (NSIS) |

## 项目结构

```
KX-Player/
├── electron/           # Electron 主进程
│   ├── main.ts         # 主进程入口、IPC 处理器
│   ├── fileScanner.ts  # 音乐文件夹扫描逻辑
│   ├── preload.ts      # 上下文桥接
│   └── workers/        # Worker 线程（元数据解析）
├── public/             # 静态资源
│   ├── favicon.ico     # 应用图标
│   └── icon.svg        # SVG 图标（构建用）
├── api.js              # IPC 封装（渲染器进程）
├── script.js           # 前端主逻辑（播放、UI、搜索等）
├── style.css           # 全局样式
├── fonts.css           # 字体声明
├── index.html          # 前端入口
├── vite.config.ts      # Vite + Electron 构建配置
└── package.json        # 项目清单
```

## 核心架构

### 状态管理 (S 对象)

```javascript
const S = {
  af: [],        // 全部音乐数组
  all: [],       // 全部曲目数组
  aI: -1,        // 活跃专辑索引
  alI: -1,       // 活跃列表索引
  tI: -1,        // 当前曲目索引
  playing: false, cTime: 0, dur: 0,
  vol: 50,       // 音量 0-100
  pVol: 50,      // 播放栏音量 0-100
  muted: false,
  mode: 0,       // 播放模式 0=顺序 1=随机 2=单曲循环
  playingTid: null,  // 正在播放的曲目 ID
  favs: [],      // 收藏曲目 ID 数组
  recents: [],   // 最近播放 {id, time}
  view: 'all',   // 当前视图 'all'|'fav'|'folder'|'pl'
  q: '',         // 搜索查询
  theme: 'light', clr: '#E63A2E',  // 主题色
  ovl: 72,       // 背景图不透明度 0-100
  devId: '',     // 音频输出设备 ID
  bgData: null,  // 背景图 base64 数据
  bgPath: null,  // 背景图文件路径
  bgSize: 'cover',
  pls: [],       // 播放列表数组
  aPl: null,     // 当前活跃播放列表
  aF: null,      // 当前活跃文件夹节点
  selMode: false, // 多选模式
  bgBlur: 0,     // 背景模糊度
  listTextColor: null,  // 列表文字颜色
  folderTree: [], folderStack: [],
  _syncingView: false
}
```

所有状态通过 `localStorage` 持久化，使用 `schedSave()` 防抖保存。

### IPC 通道 (main.ts ↔ preload.ts ↔ api.js)

| 通道 | 方向 | 说明 |
|---|---|---|
| `select-folder` | renderer → main | 打开文件夹选择器 |
| `select-file` | renderer → main | 打开文件选择器 |
| `scan-music-folders` | renderer → main | 扫描指定文件夹 |
| `remove-folder` | renderer → main | 移除已扫描的文件夹 |
| `rescan-folders` | renderer → main | 重新扫描所有文件夹 |
| `get-music-folders` | renderer → main | 获取已保存的文件夹列表 |
| `parse-metadata` | renderer → main | 解析音频文件元数据 |
| `parse-metadata-worker` | renderer → main | 使用 Worker 批量解析 |
| `load-background-image` | renderer → main | 加载背景图 |
| `delete-file` | renderer → main | 删除文件 |
| `save-file` | renderer → main | 保存文件 |
| `select-audio-output` | renderer → main | 选择音频输出设备 |

### 渲染流程

```
scan() → IPC scan-music-folders → main.ts → fileScanner.ts
       → 返回 FolderNode 树
       → 解析元数据 (Worker 线程)
       → 构建 pl[] 播放列表
       → renderAll() → 渲染当前视图
```

### 视图系统

- `all` — 全部音乐（列表视图 / 文件夹卡片视图）
- `fav` — 收藏夹
- `folder` — 文件夹浏览（使用 folderStack 管理面包屑导航）
- `pl` — 播放列表
- `search` — 搜索结果

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

### 修改检查清单

改完代码后：
1. `npm run dev` 验证热重载正常
2. 检查受影响的功能路径（如修改了搜索→测试搜索+播放状态同步）
3. `npm run build` 确保构建通过

### 构建命令

```bash
npm run dev          # 开发模式（热重载）
npm run build        # 构建前端
npm run electron:build  # 打包 NSIS 安装程序
```

安装程序输出：`dist_electron/KX音乐播放器 Setup 1.0.0.exe`

# KX 音乐播放器

基于 Electron + Vite 的本地桌面音乐播放器。

## 技术栈

- **Electron 42** - 桌面应用框架
- **Vite 8** - 前端构建工具
- **vanilla JS/CSS** - 无框架前端
- **sql.js** - SQLite 浏览器端数据库（音乐库持久化）
- **sharp** - 图片压缩（封面 JPEG 生成）
- **music-metadata** - 音频元数据解析
- **chokidar** - 文件系统监听
- **pinyin-pro** - 中文拼音（搜索功能）
- **chinese-conv** - 中文简繁转换
- **iconv-lite** - 字符编码修复（Shift-JIS）
- **ffmpeg** (可选) - 音频提取/格式转换

## 开发环境要求

- Node.js >= 18
- npm >= 9

## 安装与构建

```bash
# 进入项目目录
cd KX-Player

# 安装依赖
npm install

# 开发模式（热重载）
npm run dev

# 构建前端资源（生产模式）
npm run build

# 打包为安装程序 (Windows NSIS)
npm run electron:build
```

## 打包安装程序

运行 `npm run electron:build` 后，安装程序会输出到 `dist_electron/` 目录：

- `KX音乐播放器 Setup 1.0.4.exe` - NSIS 安装程序
- `KX音乐播放器 Setup 1.0.4.exe.blockmap` - 更新用块映射

安装程序支持自定义安装路径、创建桌面快捷方式和开始菜单快捷方式。

## 项目结构

```
KX-Player/
├── electron/                 # Electron 主进程
│   ├── main.ts              # 主进程入口、IPC 处理器、系统托盘、ffmpeg 集成
│   ├── fileScanner.ts       # 文件夹扫描（Worker 线程池）、文件监听、增量扫描
│   ├── libraryDb.ts         # SQLite 音乐库数据库（sql.js）
│   ├── coverService.ts      # 封面文件服务（sharp 压缩、文件系统存储）
│   ├── preload.ts           # 上下文桥接（main ↔ renderer）
│   └── workers/
│       └── metadata-worker.ts  # Worker 线程（元数据解析、Shift-JIS 修复）
├── KX-Player/               # 渲染器
│   ├── script.js            # 前端主逻辑（播放、UI、搜索、渲染 ~2757 行）
│   ├── api.js               # IPC 封装（renderer → electronAPI）
│   ├── utils.js             # 工具函数（DOM 辅助、路径、模糊搜索、时间格式化）
│   ├── virtual-list.js      # 高性能虚拟滚动列表
│   ├── cover.js             # 封面加载与缓存（renderer 侧，批量 IPC，LRU 缓存）
│   ├── style.css            # 全局样式、CSS 自定义属性主题
│   ├── fonts.css            # 字体声明（Inter + Noto Sans SC）
│   ├── index.html           # 前端入口
│   ├── vite.config.ts       # Vite + Electron 构建配置
│   ├── public/              # 静态资源（favicon、icon）
│   ├── build/               # NSIS 安装脚本
│   ├── find-db.js           # 调试工具：搜索 kx-player 数据库文件
│   ├── ffmpeg.exe           # ffmpeg 可执行文件（开发用）
│   └── package.json
```

构建产物：
- `dist/` - Vite 前端构建输出
- `dist-electron/` - Electron 主进程编译输出
- `dist_electron/` - electron-builder 安装程序

## 功能特性

- 文件夹扫描与增量扫描（仅解析新增/修改文件）
- 音乐元数据解析（封面、歌词、专辑信息）
- 收藏夹与播放列表管理
- 歌词同步显示
- 自定义主题色与背景图（支持缩放、透明度、模糊度调节）
- 侧边栏、顶栏、播放栏透明度调节
- 音频输出设备选择
- 音频提取与格式转换工具
- 搜索功能（支持拼音首字母搜索、简繁体匹配）
- SQLite 音乐库持久化（快速启动，无需重新扫描）
- 封面文件系统存储（JPEG 压缩，按需加载，LRU 缓存）
- 虚拟滚动列表（高性能渲染大量歌曲）
- 系统托盘（最小化到托盘）
- 文件夹文件监听（chokidar）
- 自定义字体（Inter + Noto Sans SC）

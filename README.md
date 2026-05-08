# KX 音乐播放器

基于 Electron + Vite 的本地桌面音乐播放器。

## 技术栈

- **Electron 28** - 桌面应用框架
- **Vite 5** - 前端构建工具
- **vanilla JS/CSS** - 无框架前端
- **music-metadata** - 音频元数据解析
- **chokidar** - 文件系统监听
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

- `KX音乐播放器 Setup 1.0.0.exe` - NSIS 安装程序
- `KX音乐播放器 Setup 1.0.0.exe.blockmap` - 更新用块映射

安装程序支持自定义安装路径、创建桌面快捷方式和开始菜单快捷方式。

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

构建产物：
- `dist/` - Vite 前端构建输出
- `dist-electron/` - Electron 主进程编译输出
- `dist_electron/` - electron-builder 安装程序

## 功能特性

- 文件夹扫描与文件监听
- 音乐元数据解析（封面、歌词、专辑信息）
- 收藏夹与播放列表管理
- 歌词同步显示
- 自定义主题色与背景图
- 侧边栏、顶栏、播放栏透明度调节
- 音频输出设备选择
- 音频提取与格式转换工具
- 搜索功能
- DSD 格式支持

# ffmpeg —— 故意不放这里（不随安装包分发）

**2026-09-22 用户裁决**：ffmpeg **不进安装包、不进仓库**，它是「环境需求」而不是「随包资产」。
所以 `tauri.conf.json` 里**没有** `bundle.externalBin`，这个目录里也**只有本文件**。
用户视角的写法在根 `README.md`「环境要求」；规范侧同一口径写在 `docs-vibecoding/02 §1.2`、`§1.5.3`（该目录是本地规范副本，**不入库**，看不到属正常）。

## 为什么不是「还没做」

- 缺文件却写了 `externalBin`，`build.rs` 的 `tauri_build::build()` 会当场失败 —— `cargo check` / `cargo test` / `tauri dev` 全挂。把 ~100 MB 二进制变成所有人开发的前置条件，代价先落在仓库内，收益只在分发端。
- 带 `libx264` 的构建是 **GPL v2+**，分发二进制要一并给出源码获取方式。不分发，这条义务就不落在本项目上。
- 缺它的后果已经可控：运行期 `fftools::probe_ffmpeg()`（命令 `ffmpeg_probe`）探测可用性，转换页拿到 `available:false` 时显示警告条并置灰「开始转换」，播放与曲库完全不受影响。即 `02 §1.5.3` 表里第三行要求的「能力检测 + 降级」。
- `02` 原先倾向的「按需下载可选组件」（P2-6，应用内下载器 + 哈希校验）**不实现**：检测 + 降级已经让缺失状态可理解、可自救，再加一个下载器是多出来的供应链面。

## 要用转换类功能，装一份到自己的机器上

单文件静态构建（gyan.dev 的 `ffmpeg-release-essentials.zip` 那一类）。共享构建的 `ffmpeg.exe` 只有 ~0.5 MB，运行时拖着 ~210 MB 的 `av-*.dll`，不适合当独立分发物。

`paths::locate_ffmpeg()` 的查找顺序（放到任一处即可，日常最省事的是加进 PATH）：

1. 环境变量 `KX_FFMPEG` 指向的 exe
2. `<主程序同目录>/ffmpeg.exe`
3. `<主程序同目录>/ffmpeg/ffmpeg.exe`
4. `C:\ffmpeg\bin\ffmpeg.exe`、`C:\Program Files\ffmpeg\bin\ffmpeg.exe`
5. PATH

装完跑一次 `npm run check:ffmpeg`：按 `02 §1.2` 的能力清单核 `-codecs` / `-formats`
（libx264、libmp3lame、ape/wavpack/wmav2、subrip/ass/webvtt/mov_text/dvd_subtitle、matroska/webm …），
并打印「找到了哪个 exe、版本、是否 GPL」。任一项缺失即 `exit 1`，清单唯一来源是 `scripts/check-ffmpeg.mjs`。

## 如果将来改成随包（目前不做，别顺手加）

两处改动，缺一不可：

1. 把静态构建改名成 `binaries/ffmpeg-<TRIPLE>.exe`（如 `ffmpeg-x86_64-pc-windows-msvc.exe`，`TRIPLE` 见 `tauri build --target`），提交或放入构建产物目录
2. `tauri.conf.json` → `bundle` 加 `"externalBin": ["binaries/ffmpeg"]`，然后 `npm run tauri build`（Tauri 会把它改名成 `ffmpeg.exe` 放到 exe 同目录，上面第 2 条查找顺序就会命中）

运行期不需要任何改动。同时要一并处理 GPL：安装包与「关于」页附许可证与源码获取方式。

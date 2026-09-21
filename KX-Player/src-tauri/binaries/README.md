# ffmpeg sidecar（可选，转换页专用）

放这里：`binaries/ffmpeg-x86_64-pc-windows-msvc.exe`（`TRIPLE` 见 `tauri build --target`，改名规则由 Tauri 规定）。
必须是**单文件静态构建**（gyan.dev 的 `ffmpeg-release-essentials.zip` 那一类）：
共享构建的 `ffmpeg.exe` 只有 ~0.5 MB，运行时要拖着 `avcodec-*.dll` 等 ~210 MB，不能当 sidecar 用。

## 打包前才需要做的两处改动

1. `tauri.conf.json` → `bundle` 加 `"externalBin": ["binaries/ffmpeg"]`
2. `npm run tauri build`（Tauri 会把 sidecar 改名成 `ffmpeg.exe` 放到 exe 同目录）

`externalBin` 平时**不加**：`build.rs` 里的 `tauri_build::build()` 会校验该文件存在，
缺文件时 `cargo check` / `cargo test` / `tauri dev` 会一起失败 —— 不该让没下载 sidecar 的人连开发都跑不起来。

运行期不需要任何改动：`paths::locate_ffmpeg()` 的查找顺序是
`KX_FFMPEG` → `<exe 同目录>/ffmpeg.exe`（sidecar 落点）→ `<exe 同目录>/ffmpeg/ffmpeg.exe` → `C:\ffmpeg\bin` → PATH。

## 校验

```bash
npm run check:ffmpeg
```

按 `02-architecture.md §1.2` 的能力清单核对 `-codecs` / `-formats`（libx264、libmp3lame、
ape/wavpack/wmav2/…、subrip/ass/webvtt/mov_text/dvd_subtitle、matroska/webm/…）。
任一项缺失即 `exit 1`。清单的唯一来源是 `scripts/check-ffmpeg.mjs`。

## 许可证

带 `libx264` 的构建是 **GPL v2+**：分发安装包时必须同时给出该 ffmpeg 版本的**源码获取方式**
（`02 §1.5.3` 更倾向「按需下载的可选组件」，即安装包不带 ffmpeg、首次进转换页再拉）。

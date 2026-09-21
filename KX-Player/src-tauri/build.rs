use std::path::Path;

const MPV_DLL: &str = "libmpv-2.dll";

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // OUT_DIR = target/<profile>/build/kx-player-*/out → 向上三级即 target/<profile>/
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let profile_dir = Path::new(&out_dir).ancestors().nth(3).unwrap();
    stage_mpv_dll(Path::new(&manifest), profile_dir);

    tauri_build::build();

    // ffmpeg sidecar（仅打包期需要；dev 直接找系统 ffmpeg）
    let ffmpeg_src = Path::new(&manifest).join("binaries/ffmpeg-x86_64-pc-windows-msvc.exe");
    if ffmpeg_src.exists() {
        println!("cargo:rerun-if-changed=binaries/ffmpeg-x86_64-pc-windows-msvc.exe");
    }
}

// libmpv-2.dll 不入库（115 MB > GitHub 单文件 100 MB 上限），从 .toolchain/mpv-dev 铺到
// lib/（tauri.conf.json 的 bundle.resources 落点）和 exe 同目录（运行期加载点）。
// 缺文件只警告不失败，与 binaries/README.md 的 ffmpeg sidecar 策略一致：
// 不该让没下载 mpv-dev 的人连 cargo check / cargo test 都跑不起来。
fn stage_mpv_dll(manifest: &Path, profile_dir: &Path) {
    let src = manifest
        .ancestors()
        .nth(2)
        .unwrap_or(manifest)
        .join(".toolchain/mpv-dev")
        .join(MPV_DLL);
    if !src.exists() {
        println!("cargo:warning=缺少 {MPV_DLL}（{}），跳过铺设", src.display());
        return;
    }
    for dest in [manifest.join("lib").join(MPV_DLL), profile_dir.join(MPV_DLL)] {
        copy_if_stale(&src, &dest);
    }
    // 故意不声明 lib/ 为 rerun 依赖：build.rs 自己往里写，声明了就成了重建循环
    println!("cargo:rerun-if-changed=../../.toolchain/mpv-dev/{MPV_DLL}");
}

// 115 MB，按尺寸 + mtime 判断是否已同步，避免每次 build.rs 都重铺两遍
fn copy_if_stale(src: &Path, dest: &Path) {
    let in_sync = match (src.metadata(), dest.metadata()) {
        (Ok(s), Ok(d)) => s.len() == d.len() && d.modified().ok() >= s.modified().ok(),
        _ => false,
    };
    if in_sync {
        return;
    }
    if let Some(dir) = dest.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::copy(src, dest) {
        println!("cargo:warning=铺设 {} 失败：{e}", dest.display());
    }
}

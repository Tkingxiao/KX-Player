fn main() {
    tauri_build::build();

    // 把 libmpv-2.dll 复制到可执行文件旁边，保证运行期可加载（dev 与 release 通用）。
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dll = std::path::Path::new(&manifest).join("lib").join("libmpv-2.dll");
    if dll.exists() {
        // OUT_DIR = target/<profile>/build/kx-player-*/out → 向上三级即 target/<profile>/
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let target_profile = std::path::Path::new(&out_dir)
            .ancestors()
            .nth(3)
            .map(|p| p.to_path_buf());
        if let Some(profile_dir) = target_profile {
            let dest = profile_dir.join("libmpv-2.dll");
            let _ = std::fs::copy(&dll, &dest);
        }
    }

    // ffmpeg sidecar（仅打包期需要；dev 直接找系统 ffmpeg）
    let ffmpeg_src = std::path::Path::new(&manifest).join("binaries/ffmpeg-x86_64-pc-windows-msvc.exe");
    if ffmpeg_src.exists() {
        println!("cargo:rerun-if-changed=binaries/ffmpeg-x86_64-pc-windows-msvc.exe");
    }
    println!("cargo:rerun-if-changed=lib/libmpv-2.dll");
}

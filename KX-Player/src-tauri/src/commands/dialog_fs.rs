//! 命令层：对话框与文件系统。
//!
//! 对话框是**唯一**的授权来路（P0-1）：用户在原生对话框里点出来的路径当场登记进
//! [`PathScope`]，之后 `read_*` / `file_exists` / `list_dir` / `show_item_in_folder`
//! 才认它；前端直接递进来的路径一律先过 [`PathScope::resolve`]，过不了就当没这回事
//! （返回类型不变，拒绝写日志 —— 改成错误信封是 P1-6 那一批的事）。

use base64::Engine;
use std::path::Path;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::scope::PathScope;

/// 被拒的调用留一条记录：不写的话，前端只会看到「读不到」，排查时分不清是路径不对还是权限不对
fn deny(cmd: &str, path: &str, why: crate::scope::Denied) {
    crate::paths::append_log(&format!("[scope] {cmd} 拒绝 {path}（{why:?}）"));
}

#[tauri::command]
pub async fn open_folder(app: AppHandle) -> Option<Vec<String>> {
    let app2 = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app2.dialog()
            .file()
            .set_title("选择音乐文件夹")
            .blocking_pick_folder()
            .map(|p| vec![p.to_string()])
    })
    .await
    .unwrap_or(None);
    if let Some(paths) = &picked {
        let scope = app.state::<PathScope>();
        for p in paths {
            let _ = scope.allow_root(Path::new(p));
        }
    }
    picked
}

#[tauri::command]
pub async fn open_image_file(app: AppHandle) -> Option<String> {
    let app2 = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app2.dialog()
            .file()
            .set_title("选择图片")
            .add_filter("图片", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
            .blocking_pick_file()
            .map(|p| p.to_string())
    })
    .await
    .unwrap_or(None);
    if let Some(p) = &picked {
        let _ = app.state::<PathScope>().allow_file(Path::new(p));
    }
    picked
}

/// 转换页的选文件入口（`kind` = audio | video）。走这里而不是前端 `<input type=file>`：
/// 只有对话框选中的路径才会登记进 [`PathScope`]，`convert_run` 的输入校验只认这个来路。
/// 除了文件本身，**连它们的所在目录一起授权** —— 转换的输出固定落在源目录（产品规则），
/// 前端还要靠 `file_exists` 在源目录里探「同名加序号」，不授权目录这两件事都做不了；
/// 粒度与 AI 翻译页的「选一个目录」（`open_folder`）一致。
/// 与 `open_*` 其余命令同口径：取消就是空数组，不用错误信封。
#[tauri::command]
pub async fn open_media_files(app: AppHandle, kind: String) -> Vec<String> {
    let video = kind == "video";
    let app2 = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let (title, filter) = if video {
            ("选择视频", ["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv"].to_vec())
        } else {
            ("选择音频", ["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "ape", "wv", "aiff"].to_vec())
        };
        app2.dialog()
            .file()
            .set_title(title)
            .add_filter(if video { "视频" } else { "音频" }, &filter)
            .blocking_pick_files()
            .map(|list| list.iter().map(|p| p.to_string()).collect::<Vec<_>>())
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let scope = app.state::<PathScope>();
    for p in &picked {
        let path = Path::new(p);
        let _ = scope.allow_file(path);
        if let Some(dir) = path.parent() {
            let _ = scope.allow_root(dir);
        }
    }
    picked
}

/// AI-16 导入用的多选文本框：一批可以拆给几台机器跑，导回来合并（对齐靠序号，不靠顺序）。
/// 与 `open_audio_files` 同口径：取消就是空数组，不用错误信封。
#[tauri::command]
pub async fn open_text_files(app: AppHandle) -> Vec<String> {
    let app2 = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app2.dialog()
            .file()
            .set_title("选择译文文件")
            .add_filter("文本", &["txt"])
            .blocking_pick_files()
            .map(|list| list.iter().map(|p| p.to_string()).collect::<Vec<_>>())
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let scope = app.state::<PathScope>();
    for p in &picked {
        let _ = scope.allow_file(Path::new(p));
    }
    picked
}

#[tauri::command]
pub async fn select_bg_image(app: AppHandle) -> Option<crate::model::BgImageData> {
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .set_title("选择背景图片")
            .add_filter("图片文件", &["jpg", "jpeg", "png", "bmp", "webp", "gif"])
            .blocking_pick_file()?;
        let path = picked.to_string();
        // 路径来自上面的对话框，不经过前端；落盘目标固定是数据目录，不需要再走 [PathScope]
        crate::bgimage::select_and_save(&path)
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub fn read_as_data_url(scope: tauri::State<'_, PathScope>, path: String) -> Option<String> {
    let path = match scope.resolve(&path) {
        Ok(p) => p,
        Err(why) => {
            deny("read_as_data_url", &path, why);
            return None;
        }
    };
    let bytes = match crate::limits::read_capped(&path, crate::limits::MAX_IMAGE_BYTES) {
        Ok(b) => b,
        Err(why) => {
            crate::paths::append_log(&format!("[limit] read_as_data_url 拒绝 {path:?}（{why}）"));
            return None;
        }
    };
    // 解码前先量像素：前端拿到 data URL 就会画，图片炸弹在这一步之前就得拦下
    if let Err(why) = crate::limits::check_image_pixels(&bytes, crate::limits::MAX_IMAGE_PIXELS) {
        crate::paths::append_log(&format!("[limit] read_as_data_url 拒绝 {path:?}（{why}）"));
        return None;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "jpeg",
        "png" => "png",
        "bmp" => "bmp",
        "webp" => "webp",
        "gif" => "gif",
        other => other,
    };
    Some(format!("data:image/{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[tauri::command]
pub fn read_text_file(scope: tauri::State<'_, PathScope>, path: String) -> Option<String> {
    let path = match scope.resolve(&path) {
        Ok(p) => p,
        Err(why) => {
            deny("read_text_file", &path, why);
            return None;
        }
    };
    let bytes = match crate::limits::read_capped(&path, crate::limits::MAX_TEXT_BYTES) {
        Ok(b) => b,
        Err(why) => {
            crate::paths::append_log(&format!("[limit] read_text_file 拒绝 {path:?}（{why}）"));
            return None;
        }
    };
    // BOM 处理
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(&bytes[3..]).into_owned());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (s, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return Some(s.into_owned());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return Some(s.into_owned());
    }
    // UTF-8 严格解码
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Some(s.to_string());
    }
    // 日文/中文启发：Shift-JIS 前导字节特征优先
    let has_sjis_lead = bytes.iter().any(|b| (0x81..=0x9F).contains(b) || (0xE0..=0xEF).contains(b));
    let mut candidates: Vec<&encoding_rs::Encoding> = Vec::new();
    if has_sjis_lead {
        candidates.push(encoding_rs::SHIFT_JIS);
        candidates.push(encoding_rs::GBK);
    } else {
        candidates.push(encoding_rs::GBK);
        candidates.push(encoding_rs::SHIFT_JIS);
    }
    for enc in candidates {
        let (s, _, had_errors) = enc.decode(&bytes);
        if !had_errors {
            return Some(s.into_owned());
        }
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// 存在性探测：**放行范围**先判，再看盘上有没有 —— 不许用它当目录探针
/// （「根外有没有 X 这个文件」本身也是信息）。
#[tauri::command]
pub fn file_exists(scope: tauri::State<'_, PathScope>, path: String) -> bool {
    if !scope.allows(&path) {
        deny("file_exists", &path, crate::scope::Denied::Outside);
        return false;
    }
    Path::new(&path).exists()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    pub name: String,
    pub is_file: bool,
    pub is_directory: bool,
}

#[tauri::command]
pub fn list_dir(scope: tauri::State<'_, PathScope>, path: String) -> Vec<DirEntryInfo> {
    let path = match scope.resolve(&path) {
        Ok(p) => p,
        Err(why) => {
            deny("list_dir", &path, why);
            return vec![];
        }
    };
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .take(crate::limits::MAX_DIR_ENTRIES + 1)
            .enumerate()
            .filter_map(|(i, e)| {
                if i == crate::limits::MAX_DIR_ENTRIES {
                    // 截断要说一声：歌词同名文件查找会在这份列表里找 `.lrc`，静默少一半
                    // 会表现成「这首没有歌词」，比报一条日志难查得多。
                    crate::paths::append_log(&format!(
                        "[limit] list_dir 截断到 {} 条（目录条目更多）",
                        crate::limits::MAX_DIR_ENTRIES
                    ));
                    return None;
                }
                Some(DirEntryInfo {
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_file: e.file_type().map(|t| t.is_file()).unwrap_or(false),
                    is_directory: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
                })
            })
            .collect(),
        Err(_) => vec![],
    }
}

#[tauri::command]
pub fn clipboard_write_text(text: String) -> bool {
    #[cfg(windows)]
    {
        set_clipboard_win(&text)
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        false
    }
}

#[cfg(windows)]
fn set_clipboard_win(text: &str) -> bool {
    unsafe {
        if windows_sys::Win32::System::DataExchange::OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        let mut ok = false;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        let handle = windows_sys::Win32::System::Memory::GlobalAlloc(
            windows_sys::Win32::System::Memory::GMEM_MOVEABLE,
            bytes,
        );
        if !handle.is_null() {
            let ptr = windows_sys::Win32::System::Memory::GlobalLock(handle) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                windows_sys::Win32::System::Memory::GlobalUnlock(handle);
                if windows_sys::Win32::System::DataExchange::SetClipboardData(
                    13, // CF_UNICODETEXT
                    handle as _,
                )
                .is_null()
                {
                    ok = true;
                }
            }
        }
        windows_sys::Win32::System::DataExchange::CloseClipboard();
        ok
    }
}

/// 资源管理器中打开文件或目录。
///
/// 文件要用一个 `/select,<路径>` 参数定位；目录不能也走 `/select`，否则资源管理器只会
/// 打开它的父目录并选中目录，用户会误以为「打开所在文件夹」失效。
///
/// 这里用 `std::process::Command` 而不是 `tokio::process`：只是把进程拉起来就走，
/// 不需要 reactor，也就不该因为取不到运行时（`fftools::runtime()` 失败）而误报 false。
/// 含空格的路径由 `Command` 按 CreateProcess 规则补引号，explorer 能正确解析。
#[tauri::command]
pub fn show_item_in_folder(scope: tauri::State<'_, PathScope>, path: String) -> bool {
    if path.is_empty() {
        return false;
    }
    let resolved = match scope.resolve(&path) {
        Ok(p) => p,
        Err(why) => {
            deny("show_item_in_folder", &path, why);
            return false;
        }
    };
    #[cfg(windows)]
    {
        let args = explorer_args(&resolved, resolved.is_dir());
        match std::process::Command::new("explorer.exe").args(args).spawn() {
            Ok(_) => true,
            Err(e) => {
                crate::paths::append_log(&format!("[fs] 打开资源管理器失败：{path}（{e}）"));
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn explorer_args(path: &Path, is_dir: bool) -> Vec<std::ffi::OsString> {
    if is_dir {
        vec![path.as_os_str().to_os_string()]
    } else {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        vec![arg]
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn explorer_uses_a_directory_as_the_location_but_selects_a_file() {
        use super::explorer_args;
        use std::path::Path;

        let dir = explorer_args(Path::new(r"C:\Music\Album"), true);
        assert_eq!(dir, vec![std::ffi::OsString::from(r"C:\Music\Album")]);

        let file = explorer_args(Path::new(r"C:\Music\Album\song.lrc"), false);
        assert_eq!(file, vec![std::ffi::OsString::from(r"/select,C:\Music\Album\song.lrc")]);
    }
}

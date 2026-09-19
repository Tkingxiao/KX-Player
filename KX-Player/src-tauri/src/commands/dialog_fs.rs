//! 命令层：对话框与文件系统。

use base64::Engine;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn open_folder(app: AppHandle) -> Option<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择音乐文件夹")
            .blocking_pick_folder()
            .map(|p| vec![p.to_string()])
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub async fn open_image_file(app: AppHandle) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择图片")
            .add_filter("图片", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
            .blocking_pick_file()
            .map(|p| p.to_string())
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub async fn open_audio_files(app: AppHandle) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择音频/视频")
            .add_filter(
                "音频/视频",
                &["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "ape", "wv", "aiff", "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv"],
            )
            .blocking_pick_files()
            .map(|list| list.iter().map(|p| p.to_string()).collect())
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
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
        crate::bgimage::select_and_save(&path)
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub fn read_as_data_url(path: String) -> Option<String> {
    let bytes = std::fs::read(&path).ok()?;
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
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
pub fn read_text_file(path: String) -> Option<String> {
    let bytes = std::fs::read(&path).ok()?;
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

#[tauri::command]
pub fn file_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    pub name: String,
    pub is_file: bool,
    pub is_directory: bool,
}

#[tauri::command]
pub fn list_dir(path: String) -> Vec<DirEntryInfo> {
    match std::fs::read_dir(&path) {
        Ok(entries) => entries
            .flatten()
            .map(|e| DirEntryInfo {
                name: e.file_name().to_string_lossy().into_owned(),
                is_file: e.file_type().map(|t| t.is_file()).unwrap_or(false),
                is_directory: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            })
            .collect(),
        Err(_) => vec![],
    }
}

#[tauri::command]
pub fn rename_dir(old_path: String, new_path: String) -> Result<(), String> {
    crate::ai::rename_dir(&old_path, &new_path)
}

#[tauri::command]
pub fn tools_save_file(path: String, base64_data: String) -> bool {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
        Ok(b) => b,
        Err(e) => {
            crate::paths::append_log(&format!("[tools:saveFile] 解码失败: {e}"));
            return false;
        }
    };
    std::fs::write(&path, bytes).is_ok()
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
    use std::ffi::CString;
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
                if !windows_sys::Win32::System::DataExchange::SetClipboardData(
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
        let _ = CString::new("clipboard");
        ok
    }
}

/// 资源管理器中定位文件
#[tauri::command]
pub fn show_item_in_folder(path: String) -> bool {
    let arg = format!("/select,{path}");
    let _ = crate::fftools::runtime().block_on(async {
        #[cfg(windows)]
        {
            let _ = tokio::process::Command::new("explorer.exe").args(["/select,", &arg]).spawn();
        }
        #[allow(unused_must_use)]
        {
            ()
        }
    });
    true
}

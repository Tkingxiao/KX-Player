//! 设置 JSON 读写（与 Electron 版同一文件、同一格式）。

use std::path::Path;

pub fn load(settings_path: &Path) -> serde_json::Value {
    match std::fs::read_to_string(settings_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        Err(_) => serde_json::Value::Object(serde_json::Map::new()),
    }
}

pub fn save(settings_path: &Path, settings: &serde_json::Value) -> bool {
    if let Some(dir) = settings_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let pretty = serde_json::to_string_pretty(settings).unwrap_or_default();
    // tmp + rename 原子替换
    let tmp = settings_path.with_extension("json.tmp");
    if std::fs::write(&tmp, pretty).is_ok() {
        return std::fs::rename(&tmp, settings_path).is_ok();
    }
    false
}

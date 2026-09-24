//! 设置 JSON 读写（与 Electron 版同一文件、同一格式）。

use std::path::Path;

pub fn load(settings_path: &Path) -> serde_json::Value {
    match std::fs::read_to_string(settings_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        Err(_) => serde_json::Value::Object(serde_json::Map::new()),
    }
}

pub fn save(settings_path: &Path, settings: &serde_json::Value) -> bool {
    let pretty = serde_json::to_string_pretty(settings).unwrap_or_default();
    // 唯一临时名 + sync + 原子替换：设置写坏一次的代价是「所有偏好回到默认」，
    // 而且用户不会知道是这一次退出时坏的
    crate::atomic::write_str(settings_path, &pretty).is_ok()
}

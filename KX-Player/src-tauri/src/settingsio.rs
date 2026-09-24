//! 设置 JSON 读写（与 Electron 版同一文件、同一格式）。

use std::path::Path;

pub fn load(settings_path: &Path) -> serde_json::Value {
    match std::fs::read_to_string(settings_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        Err(_) => serde_json::Value::Object(serde_json::Map::new()),
    }
}

pub fn save(settings_path: &Path, settings: &serde_json::Value) -> bool {
    // 写盘这一层是「秘密不外泄」的最后一道闸：设置文件可能被同步盘、旧版本或
    // 手改动过，把 API Key 之类的字段塞了进来（P0-3）。剥掉再写，并留一条日志 ——
    // 前端本来就只把 Key 放内存，真命中说明有别处在往里写。
    let mut cleaned = settings.clone();
    let removed = strip_secrets(&mut cleaned);
    if !removed.is_empty() {
        crate::paths::append_log(&format!("[settings] 拒绝持久化秘密字段：{}", removed.join("、")));
    }
    let pretty = serde_json::to_string_pretty(&cleaned).unwrap_or_default();
    // 唯一临时名 + sync + 原子替换：设置写坏一次的代价是「所有偏好回到默认」，
    // 而且用户不会知道是这一次退出时坏的
    crate::atomic::write_str(settings_path, &pretty).is_ok()
}

/// 秘密字段名（比较时去掉 `_`/`-` 并转小写，所以 `apiKey`、`api_key`、`Api-Key` 都算）。
/// 只做名字匹配、不看值 —— 值长得像不像 Key 是猜的，名字是确定的。
const SECRET_KEYS: [&str; 12] = [
    "apikey",
    "key",
    "token",
    "accesstoken",
    "refreshtoken",
    "password",
    "passwd",
    "secret",
    "clientsecret",
    "privatekey",
    "credential",
    "credentials",
];

/// 递归剥掉秘密字段，返回被剥掉的字段名（去重前的顺序保留，便于日志对账）。
pub fn strip_secrets(value: &mut serde_json::Value) -> Vec<String> {
    let mut removed = Vec::new();
    walk(value, &mut removed);
    removed
}

fn walk(value: &mut serde_json::Value, removed: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|k, v| {
                if is_secret_key(k) {
                    removed.push(k.clone());
                    return false;
                }
                walk(v, removed);
                true
            });
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                walk(item, removed);
            }
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    SECRET_KEYS.contains(&normalized.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("测试 JSON 应当能解析")
    }

    #[test]
    fn secrets_are_stripped_from_top_level_and_nested_places() {
        let mut v = json(
            r#"{"theme":"dark","apiKey":"sk-live-1","ai":{"api_key":"sk-2","model":"m"},
                "favs":[{"name":"a","password":"p"}],"tokens":[{"token":"t"}]}"#,
        );
        let removed = strip_secrets(&mut v);
        assert_eq!(removed.len(), 4, "{removed:?}");
        assert!(v.get("apiKey").is_none());
        assert!(v["ai"].get("api_key").is_none());
        assert!(v["favs"][0].get("password").is_none());
        assert!(v["tokens"][0].get("token").is_none());
        assert_eq!(v["theme"], "dark", "非秘密字段一个都不能动");
        assert_eq!(v["ai"]["model"], "m");
        assert_eq!(v["favs"][0]["name"], "a");
    }

    #[test]
    fn real_settings_keys_are_not_secret_looking() {
        // 现有快照里的键名都过一遍：剥错了等于用户设置丢一块
        let mut v = json(
            r#"{"theme":"dark","clr":"x","ovl":0.5,"bgPath":"p","bgMtime":1,"bgSize":"cover",
                "keyboardEnabled":true,"recents":["a"],"folderSort":"name","aiBaseURL":"https://api.x/v1",
                "aiModel":"m","aiModelKind":"remote","aiMergeLines":4,"_v":2,"_imgEditState":{}}"#,
        );
        assert!(strip_secrets(&mut v).is_empty(), "现有设置键不该命中秘密清单");
    }

    #[test]
    fn name_matching_ignores_case_and_separators() {
        for k in ["apiKey", "API_KEY", "Api-Key", "apikey", "accessToken", "Private-Key"] {
            assert!(is_secret_key(k), "{k} 应当命中");
        }
        for k in ["keyboardEnabled", "keynote", "monkey", "tokenizer", "secretSauce"] {
            assert!(!is_secret_key(k), "{k} 不该命中");
        }
    }
}

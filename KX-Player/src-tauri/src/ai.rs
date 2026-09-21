//! AI 翻译服务：OpenAI 兼容 /chat/completions（网络只发生在 Rust 侧；API Key 不落盘）。

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::{AiChatPayload, AiModelsResult, AiPingResult};
use reqwest::blocking::Client;
use std::time::Duration;

/// 规范化 baseURL：去尾部斜杠，无 /v1 时自动补（Ollama 原生地址兼容）
pub fn normalize_base_url(raw: &str) -> String {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        u = "http://127.0.0.1:11434".into();
    }
    // 仅接受 http/https
    if !u.starts_with("http://") && !u.starts_with("https://") {
        u = format!("http://{u}");
    }
    if !has_version_segment(&u) {
        u.push_str("/v1");
    }
    u
}

/// 等价 TS 正则 /\/v\d+(\/|$)/：末段或任一段为 v+数字
fn has_version_segment(u: &str) -> bool {
    u.split('/').any(|seg| {
        seg.strip_prefix('v')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

fn client(timeout: Duration) -> Client {
    Client::builder().timeout(timeout).build().unwrap_or_default()
}

fn chat_url(base_url: &str) -> String {
    format!("{}/chat/completions", normalize_base_url(base_url))
}

pub fn chat_completion(opts: &AiChatPayload, timeout_ms: u64) -> Result<String, String> {
    let url = chat_url(&opts.base_url);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    if !opts.api_key.is_empty() {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", opts.api_key)) {
            headers.insert("Authorization", v);
        }
    }
    let body = serde_json::json!({
        "model": opts.model,
        "messages": opts.messages,
        "temperature": opts.temperature.unwrap_or(0.3),
        "stream": false,
    });
    let resp = client(Duration::from_millis(timeout_ms))
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(format!("HTTP {status}: {}", &text[..text.len().min(300)]));
    }
    let data: serde_json::Value = resp.json().map_err(|e| format!("解析失败: {e}"))?;
    let content = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or("响应缺少 choices[0].message.content")?;
    Ok(content.to_string())
}

pub fn ping(opts: &AiChatPayload) -> AiPingResult {
    let minimal = AiChatPayload {
        base_url: opts.base_url.clone(),
        api_key: opts.api_key.clone(),
        model: opts.model.clone(),
        messages: vec![crate::model::ChatMessage { role: "user".into(), content: "ping".into() }],
        temperature: Some(0.0),
    };
    match chat_completion(&minimal, 15_000) {
        // 按字符截：`&reply[..60]` 落在多字节中间会 panic，而这条在命令线程上。
        Ok(reply) => AiPingResult { ok: true, message: format!("连接成功：{}", reply.chars().take(60).collect::<String>()) },
        Err(e) => AiPingResult { ok: false, message: e },
    }
}

pub fn list_models(base_url: &str, api_key: &str) -> AiModelsResult {
    let url = format!("{}/models", normalize_base_url(base_url));
    let mut req = client(Duration::from_secs(15)).get(url);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    match req.send() {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().unwrap_or_default();
                return AiModelsResult { ok: false, models: vec![], error: Some(format!("HTTP {status}: {}", &text[..text.len().min(200)])) };
            }
            let data: serde_json::Value = match resp.json() {
                Ok(d) => d,
                Err(e) => return AiModelsResult { ok: false, models: vec![], error: Some(format!("解析失败: {e}")) },
            };
            let mut models: Vec<String> = data["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            models.sort();
            AiModelsResult { ok: true, models, error: None }
        }
        Err(e) => AiModelsResult { ok: false, models: vec![], error: Some(format!("{e}")) },
    }
}

/// 目录重命名（同父目录内；目标已存在则拒绝）
pub fn rename_dir(old_path: &str, new_path: &str) -> IpcResult<()> {
    if old_path.is_empty() || new_path.is_empty() {
        return Err(IpcError::invalid_argument("参数错误"));
    }
    let old = std::path::Path::new(old_path);
    let new = std::path::Path::new(new_path);
    if !old.is_dir() {
        return Err(IpcError::new(AppErrorCode::NotFound, "原目录不存在"));
    }
    if new.exists() {
        return Err(IpcError::invalid_argument("目标目录已存在"));
    }
    let old_parent = old.parent().unwrap_or(old);
    let new_parent = new.parent().unwrap_or(new);
    if old_parent != new_parent {
        return Err(IpcError::invalid_argument("只允许同父目录内重命名"));
    }
    std::fs::rename(old, new).map_err(IpcError::io)
}

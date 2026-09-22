//! AI 翻译服务：OpenAI 兼容 /chat/completions（网络只发生在 Rust 侧；API Key 不落盘）。

use crate::error::{AppErrorCode, IpcError, IpcResult};
use crate::model::{AiChatPayload, AiModelsResult, AiPingResult};
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

/// 共享 **async** 客户端（连接池复用）。
///
/// B3：这里曾是 `reqwest::blocking::Client`，而 `ai_chat` 是同步命令 —— 同步命令跑在
/// 主线程上，一次 120 s 超时的补全会让整个窗口冻住（用户在等，且无法取消）。
/// 现在只留 async 客户端，`Cargo.toml` 里连 `blocking` feature 都删掉了：
/// 谁再写回阻塞调用，编译直接失败。
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().build().unwrap_or_default())
}

fn chat_url(base_url: &str) -> String {
    format!("{}/chat/completions", normalize_base_url(base_url))
}

/// 按**字符**截断，绝不按字节。
///
/// `&text[..n]` 在 `n` 落在多字节字符中间时会直接 panic（上游返回中文错误页或中文回复时
/// 相当常见：UTF-8 汉字 3 字节，字节边界的 2/3 概率踩在字符中间），而这个函数跑在命令
/// 线程上 —— 一次 panic 会把整条调用变成兜底的「任务失败」，HTTP 状态与厂商原文全丢。
/// 同批把 `ai_ping` 的 `&reply[..60]` 也统一到这里，杜绝复发。
fn brief(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

/// 一次补全的结果：正文 + 计费信息（`ai_ping` 顺带报 token 数，AI-5）。
struct ChatOutcome {
    content: String,
    total_tokens: Option<u64>,
}

/// 补全失败的原因分类。批量任务必须分得开「端点连不上」和「这句没翻好」：
/// 前者接着往下跑只是把几百个文件白烧一遍，后者换下一个文件仍可能成功。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// 连接被拒 / DNS 失败 / TLS 握手不来：端点级，重试同一批没有意义
    Transport(String),
    /// 请求发出去了但没在时限内回：本地模型加载权重时很常见，不算端点已死
    Timeout(String),
    /// 非 2xx、响应结构不对、正文缺失：请求级
    Reply(String),
}

impl AiError {
    pub fn text(&self) -> String {
        match self {
            Self::Transport(m) | Self::Timeout(m) | Self::Reply(m) => m.clone(),
        }
    }

    /// 只有「根本连不上」才够格判定端点已死
    pub fn is_dead_endpoint(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    fn from_reqwest(e: &reqwest::Error, text: String) -> Self {
        if e.is_connect() {
            Self::Transport(text)
        } else if e.is_timeout() {
            Self::Timeout(text)
        } else {
            Self::Reply(text)
        }
    }
}

async fn chat_completion_full(opts: &AiChatPayload, timeout_ms: u64) -> Result<ChatOutcome, AiError> {
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
    let resp = client()
        .post(url)
        .timeout(Duration::from_millis(timeout_ms))
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::from_reqwest(&e, brief(&format!("请求失败: {e}"), 300)))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AiError::Reply(format!("HTTP {status}: {}", brief(&text, 300))));
    }
    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AiError::Reply(format!("解析失败: {e}")))?;
    let content = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AiError::Reply("响应缺少 choices[0].message.content".into()))?;
    Ok(ChatOutcome {
        content: content.to_string(),
        total_tokens: data.pointer("/usage/total_tokens").and_then(|v| v.as_u64()),
    })
}

/// 「只要正文、不要计费」的入口：`ai_chat` 命令与 `ai_job` 的逐块翻译都在用。
/// 错误保留分类（`AiError`），批量任务靠它判断端点是否已经失联。
pub async fn chat_completion(opts: &AiChatPayload, timeout_ms: u64) -> Result<String, AiError> {
    chat_completion_full(opts, timeout_ms).await.map(|out| out.content)
}

/// 连通性测试：回显延迟、模型名与 token 数（AI-5），失败时回原文。
pub async fn ping(opts: &AiChatPayload) -> AiPingResult {
    let minimal = AiChatPayload {
        base_url: opts.base_url.clone(),
        api_key: opts.api_key.clone(),
        model: opts.model.clone(),
        messages: vec![crate::model::ChatMessage { role: "user".into(), content: "ping".into() }],
        temperature: Some(0.0),
    };
    let started = std::time::Instant::now();
    match chat_completion_full(&minimal, 15_000).await {
        Ok(out) => AiPingResult {
            ok: true,
            message: format!("连接成功：{}", brief(&out.content, 60)),
            model: Some(opts.model.clone()),
            latency_ms: Some(started.elapsed().as_millis() as u64),
            total_tokens: out.total_tokens,
        },
        Err(e) => AiPingResult {
            ok: false,
            message: e.text(),
            model: Some(opts.model.clone()),
            latency_ms: None,
            total_tokens: None,
        },
    }
}

/// OpenAI 兼容的模型清单端点：`{base}/models`。
fn models_url(base_url: &str) -> String {
    format!("{}/models", normalize_base_url(base_url))
}

/// Ollama **原生**端点：`/api/tags` 挂在站点根上，**不在 `/v1` 之下** ——
/// 老版本 Ollama 没有 `/v1/models`，只打 `/models` 会取不到任何模型（AI-2）。
fn ollama_tags_url(base_url: &str) -> String {
    let base = normalize_base_url(base_url);
    let origin = base.strip_suffix("/v1").unwrap_or(&base);
    format!("{origin}/api/tags")
}

/// 两种线上形状都吃：OpenAI `{"data":[{"id":"…"}]}` 与 Ollama `{"models":[{"name":"…"}]}`。
fn parse_model_list(data: &serde_json::Value) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    if let Some(arr) = data["data"].as_array() {
        models.extend(arr.iter().filter_map(|m| m["id"].as_str()).filter(|s| !s.is_empty()).map(str::to_string));
    }
    if let Some(arr) = data["models"].as_array() {
        models.extend(
            arr.iter()
                .filter_map(|m| m["name"].as_str().or_else(|| m["model"].as_str()))
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    models.sort();
    models.dedup();
    models
}

async fn fetch_models(url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let mut req = client().get(url).timeout(Duration::from_secs(15));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    let resp = req.send().await.map_err(|e| format!("{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", brief(&text, 200)));
    }
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("解析失败: {e}"))?;
    Ok(parse_model_list(&data))
}

/// 先打 OpenAI 兼容端点，空手而归再退到 Ollama 原生端点；两条都失败时把两条原因一起回给用户。
pub async fn list_models(base_url: &str, api_key: &str) -> AiModelsResult {
    let mut errors: Vec<String> = Vec::new();
    for url in [models_url(base_url), ollama_tags_url(base_url)] {
        match fetch_models(&url, api_key).await {
            Ok(models) if !models.is_empty() => return AiModelsResult { ok: true, models, error: None },
            Ok(_) => errors.push(format!("{url} 没有返回任何模型")),
            Err(e) => errors.push(format!("{url}：{e}")),
        }
    }
    AiModelsResult { ok: false, models: vec![], error: Some(errors.join("；")) }
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
#[cfg(test)]
mod tests {
    use super::*;

    /// 老代码 `&text[..text.len().min(300)]`：UTF-8 汉字 3 字节，第 300 字节大概率
    /// 落在字符中间 → 命令线程 panic，真实原因被兜底的「任务失败」顶掉。
    #[test]
    fn brief_counts_characters_not_bytes() {
        let cn = "错误信息".repeat(100);
        let cut = brief(&cn, 300);
        assert_eq!(cut.chars().count(), 301, "300 个字符 + 省略号");
        assert!(cut.ends_with('…'));
        assert_eq!(brief("短", 10), "短", "没超长就原样返回");
        assert_eq!(brief("", 0), "");
    }

    /// 快速失败只认 Transport：本地模型首次加载权重动辄几分钟，一次 Timeout 就停整条队列
    /// 会把「其实还能继续」的任务误杀。
    #[test]
    fn only_transport_errors_mark_the_endpoint_dead() {
        assert!(AiError::Transport("连接被拒绝".into()).is_dead_endpoint());
        assert!(!AiError::Timeout("operation timed out".into()).is_dead_endpoint());
        assert!(!AiError::Reply("HTTP 401: unauthorized".into()).is_dead_endpoint());
        assert_eq!(AiError::Reply("HTTP 401: unauthorized".into()).text(), "HTTP 401: unauthorized");
    }

    #[test]
    fn ollama_endpoint_is_derived_from_the_origin() {
        assert_eq!(ollama_tags_url("http://127.0.0.1:11434"), "http://127.0.0.1:11434/api/tags");
        assert_eq!(ollama_tags_url("http://127.0.0.1:11434/v1/"), "http://127.0.0.1:11434/api/tags");
        assert_eq!(ollama_tags_url("https://api.deepseek.com"), "https://api.deepseek.com/api/tags");
        assert_eq!(models_url("http://127.0.0.1:11434"), "http://127.0.0.1:11434/v1/models");
    }

    #[test]
    fn parse_model_list_reads_both_wire_shapes() {
        let openai: serde_json::Value =
            serde_json::from_str(r#"{"data":[{"id":"gpt-4o"},{"id":"b"},{"id":""}]}"#).unwrap();
        assert_eq!(parse_model_list(&openai), vec!["b".to_string(), "gpt-4o".to_string()]);

        let ollama: serde_json::Value = serde_json::from_str(
            r#"{"models":[{"name":"qwen2.5:7b"},{"model":"llama3"},{"name":"qwen2.5:7b"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_model_list(&ollama), vec!["llama3".to_string(), "qwen2.5:7b".to_string()]);

        let none: serde_json::Value = serde_json::from_str(r#"{"object":"list"}"#).unwrap();
        assert!(parse_model_list(&none).is_empty());
    }

    #[test]
    fn ping_result_serializes_as_camel_case() {
        let ok = AiPingResult { ok: true, message: "连接成功：pong".into(), model: Some("m".into()), latency_ms: Some(12), total_tokens: Some(3) };
        let j = serde_json::to_value(&ok).unwrap();
        assert_eq!(j["latencyMs"], 12);
        assert_eq!(j["totalTokens"], 3);
        assert_eq!(j["model"], "m");

        let bad = AiPingResult { ok: false, message: "网络不可达".into(), model: Some("m".into()), latency_ms: None, total_tokens: None };
        let j2 = serde_json::to_value(&bad).unwrap();
        assert!(j2.get("latencyMs").is_none(), "失败时只留 message");
        assert!(j2.get("totalTokens").is_none());
    }
}

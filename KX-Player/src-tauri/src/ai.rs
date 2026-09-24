//! AI 翻译服务：OpenAI 兼容 /chat/completions（网络只发生在 Rust 侧；API Key 不落盘）。

use crate::ai_url::Target;
use crate::model::{AiChatPayload, AiModelsResult, AiPingResult};
use std::time::Duration;

/// 日志与调试输出里的秘密表示：只报有没有、多长，**永不回显内容**。
pub fn redact(secret: &str) -> String {
    if secret.is_empty() {
        "未设置".to_string()
    } else {
        format!("已设置（{} 字符）", secret.chars().count())
    }
}

/// 端点校验与规范化都在 [`crate::ai_url`]（P0-3）：scheme 白名单、默认 https、
/// 明文只给本机与私网、公网只走 443。这里只负责把它归到错误分类上。
fn resolve(raw: &str) -> Result<Target, AiError> {
    crate::ai_url::check(raw).map_err(AiError::Config)
}

fn chat_url(base: &str) -> String {
    format!("{base}/chat/completions")
}

/// 共享 **async** 客户端（连接池复用）。
///
/// B3：这里曾是 `reqwest::blocking::Client`，而 `ai_chat` 是同步命令 —— 同步命令跑在
/// 主线程上，一次 120 s 超时的补全会让整个窗口冻住（用户在等，且无法取消）。
/// 现在只留 async 客户端，`Cargo.toml` 里连 `blocking` feature 都删掉了：
/// 谁再写回阻塞调用，编译直接失败。
///
/// 重定向改成逐跳复核的自定义策略（P0-3）：跨源一律拒，理由见 `ai_url::redirect_verdict`。
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(crate::ai_url::redirect_policy())
            .build()
            .unwrap_or_default()
    })
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
    /// 接口地址本身就不合规（P0-3）：整批都不可能成功，与「连不上」同样归为端点没救
    Config(String),
}

impl AiError {
    pub fn text(&self) -> String {
        match self {
            Self::Transport(m) | Self::Timeout(m) | Self::Reply(m) | Self::Config(m) => m.clone(),
        }
    }

    /// 「根本连不上」或「地址根本不合规」才够格判定端点已死：这两种情况接着往下跑
    /// 只是把剩下的文件白烧一遍。
    pub fn is_dead_endpoint(&self) -> bool {
        matches!(self, Self::Transport(_) | Self::Config(_))
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

/// 带上限地读响应体：`resp.text()` 是「对端给多少就吃多少」—— 一个坏掉的端点用一条
/// 超大响应就能把进程内存顶穿。先看 `Content-Length`（有就早退），再按块读、过上限即停
/// —— 流式响应没有长度，只靠前者等于没拦。
async fn read_body_capped(resp: &mut reqwest::Response, cap: u64) -> Result<String, AiError> {
    if let Some(n) = resp.content_length() {
        if n > cap {
            return Err(AiError::Reply(format!("响应体 {n} 字节，超过上限 {cap}")));
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| AiError::Reply(format!("读取响应失败: {e}")))? {
        push_capped(&mut buf, &chunk, cap)?;
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 单独一个函数是为了能测：不发真请求也能钉住「到上限就停」。
fn push_capped(buf: &mut Vec<u8>, chunk: &[u8], cap: u64) -> Result<(), AiError> {
    if buf.len() as u64 + chunk.len() as u64 > cap {
        return Err(AiError::Reply(format!("响应体超过上限 {cap} 字节")));
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

async fn chat_completion_full(opts: &AiChatPayload, timeout_ms: u64) -> Result<ChatOutcome, AiError> {
    let target = resolve(&opts.base_url)?;
    crate::ai_url::note_endpoint(&target, &opts.api_key);
    let url = chat_url(&target.base);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    if !opts.api_key.is_empty() {
        // 拼出来的 `Bearer …` 也是一份 Key：用完就清，别留在堆上等下一次分配
        let bearer = zeroize::Zeroizing::new(format!("Bearer {}", opts.api_key));
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&bearer) {
            headers.insert("Authorization", v);
        }
    }
    let mut body = serde_json::json!({
        "model": opts.model,
        "messages": opts.messages,
        "temperature": opts.temperature.unwrap_or(0.3),
        "stream": false,
    });
    // 只在软件侧给了上限时才写这个字段：各家 OpenAI 兼容接口对 `max_tokens: null` 的容忍度不一，
    // 少发一个键比猜对方怎么解释 null 稳。
    if let Some(n) = opts.max_tokens {
        body["max_tokens"] = serde_json::json!(n);
    }
    let resp = client()
        .post(url)
        .timeout(Duration::from_millis(timeout_ms))
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::from_reqwest(&e, brief(&format!("请求失败: {e}"), 300)))?;
    let status = resp.status();
    let mut resp = resp;
    if !status.is_success() {
        let text = read_body_capped(&mut resp, crate::limits::MAX_AI_RESPONSE_BYTES).await.unwrap_or_default();
        return Err(AiError::Reply(format!("HTTP {status}: {}", brief(&text, 300))));
    }
    let text = read_body_capped(&mut resp, crate::limits::MAX_AI_RESPONSE_BYTES).await?;
    let data: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| AiError::Reply(format!("解析失败: {e}")))?;
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
        max_tokens: None,
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
fn models_url(base: &str) -> String {
    format!("{base}/models")
}

/// Ollama **原生**端点：`/api/tags` 挂在站点根上，**不在 `/v1` 之下** ——
/// 老版本 Ollama 没有 `/v1/models`，只打 `/models` 会取不到任何模型（AI-2）。
fn ollama_tags_url(base: &str) -> String {
    let origin = base.strip_suffix("/v1").unwrap_or(base);
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
        let bearer = zeroize::Zeroizing::new(format!("Bearer {api_key}"));
        req = req.header("Authorization", bearer.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("{e}"))?;
    let status = resp.status();
    let mut resp = resp;
    let text = read_body_capped(&mut resp, crate::limits::MAX_AI_RESPONSE_BYTES)
        .await
        .map_err(|e| e.text())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", brief(&text, 200)));
    }
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("解析失败: {e}"))?;
    Ok(parse_model_list(&data))
}

/// 先打 OpenAI 兼容端点，空手而归再退到 Ollama 原生端点；两条都失败时把两条原因一起回给用户。
pub async fn list_models(base_url: &str, api_key: &str) -> AiModelsResult {
    let target = match crate::ai_url::check(base_url) {
        Ok(t) => t,
        Err(e) => return AiModelsResult { ok: false, models: vec![], error: Some(e) },
    };
    crate::ai_url::note_endpoint(&target, api_key);
    let mut errors: Vec<String> = Vec::new();
    for url in [models_url(&target.base), ollama_tags_url(&target.base)] {
        match fetch_models(&url, api_key).await {
            Ok(models) if !models.is_empty() => return AiModelsResult { ok: true, models, error: None },
            Ok(_) => errors.push(format!("{url} 没有返回任何模型")),
            Err(e) => errors.push(format!("{url}：{e}")),
        }
    }
    AiModelsResult { ok: false, models: vec![], error: Some(errors.join("；")) }
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

    /// 快速失败只认「不可恢复」：本地模型首次加载权重动辄几分钟，一次 Timeout 就停整条队列
    /// 会把「其实还能继续」的任务误杀；地址不合规则反过来 —— 整批都跑不动，越早停越好。
    #[test]
    fn only_unrecoverable_errors_mark_the_endpoint_dead() {
        assert!(AiError::Transport("连接被拒绝".into()).is_dead_endpoint());
        assert!(AiError::Config("公网主机必须用 https".into()).is_dead_endpoint());
        assert!(!AiError::Timeout("operation timed out".into()).is_dead_endpoint());
        assert!(!AiError::Reply("HTTP 401: unauthorized".into()).is_dead_endpoint());
        assert_eq!(AiError::Reply("HTTP 401: unauthorized".into()).text(), "HTTP 401: unauthorized");
    }

    #[test]
    fn an_unusable_base_url_is_a_config_error() {
        let e = resolve("http://api.deepseek.com/v1").expect_err("公网明文地址应当被拒");
        assert!(e.is_dead_endpoint(), "地址不合规整批都跑不动");
        assert!(e.text().contains("必须用 https"), "{}", e.text());
        assert!(resolve("http://127.0.0.1:11434/v1").is_ok(), "本机 Ollama 不受影响");
    }

    /// 响应体上限到点就停，不是「先全收下再判断」—— 后者对坏端点毫无意义。
    #[test]
    fn response_body_cap_stops_at_the_limit() {
        let mut buf: Vec<u8> = Vec::new();
        push_capped(&mut buf, b"1234", 8).expect("未超限");
        push_capped(&mut buf, b"5678", 8).expect("正好到上限");
        let e = push_capped(&mut buf, b"9", 8).expect_err("多一个字节就该拒");
        assert!(e.text().contains("上限"), "{}", e.text());
        assert_eq!(buf.len(), 8, "被拒的那一块不该进缓冲");
    }

    /// 流式响应没有 `Content-Length`，只查头等于没拦：分块累计到上限也得停下。
    #[test]
    fn response_body_cap_counts_chunk_by_chunk() {
        let mut buf: Vec<u8> = Vec::new();
        for i in 0..4 {
            assert!(push_capped(&mut buf, &[b'x'; 100], 400).is_ok(), "第 {i} 块仍在限内");
        }
        assert!(push_capped(&mut buf, b"x", 400).is_err(), "第 401 个字节必须被拦");
    }

    #[test]
    fn ollama_endpoint_is_derived_from_the_origin() {
        let base = |raw: &str| resolve(raw).expect("测试地址应当通过").base;
        assert_eq!(ollama_tags_url(&base("http://127.0.0.1:11434")), "http://127.0.0.1:11434/api/tags");
        assert_eq!(ollama_tags_url(&base("http://127.0.0.1:11434/v1/")), "http://127.0.0.1:11434/api/tags");
        assert_eq!(ollama_tags_url(&base("https://api.deepseek.com")), "https://api.deepseek.com/api/tags");
        assert_eq!(models_url(&base("http://127.0.0.1:11434")), "http://127.0.0.1:11434/v1/models");
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

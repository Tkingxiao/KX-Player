//! AI 翻译服务：三种接口协议可切换（OpenAI Chat Completions / OpenAI Responses /
//! Anthropic Messages；网络只发生在 Rust 侧；API Key 不落盘）。

use crate::ai_url::Target;
use crate::model::{AiChatPayload, AiModelsResult, ApiStyle, AiPingResult};
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
/// 明文只给本机与私网、公网只走 443，外加用户显式放行的明文端点（`check_for_app`）。
/// 这里只负责把它归到错误分类上。
fn resolve(raw: &str) -> Result<Target, AiError> {
    crate::ai_url::check_for_app(raw).map_err(AiError::Config)
}

fn chat_url(base: &str) -> String {
    format!("{base}/chat/completions")
}

/// 三种协议都把版本段交给 `ai_url` 的规范化（无版本段时补 `/v1`），差别只在末段路径：
/// OpenAI 两代是 `{base}/chat/completions` 与 `{base}/responses`，Anthropic 是 `{base}/messages`。
fn endpoint_url(style: ApiStyle, base: &str) -> String {
    match style {
        ApiStyle::OpenAiChat => chat_url(base),
        ApiStyle::OpenAiResponses => format!("{base}/responses"),
        ApiStyle::Anthropic => format!("{base}/messages"),
    }
}

/// Anthropic 的版本头是固定日期，不是「最新」可以猜的 —— 官方按这个值保证兼容。
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 各协议的鉴权头（不含 Content-Type，POST 的 `.json()` 自己带）。
/// OpenAI 两代都走 `Authorization: Bearer`；Anthropic 是 `x-api-key` + 版本头。
/// 拼出来的 `Bearer …` 也是一份 Key：用完就清，别留在堆上等下一次分配。
fn auth_headers(style: ApiStyle, api_key: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if api_key.is_empty() {
        return headers;
    }
    match style {
        ApiStyle::OpenAiChat | ApiStyle::OpenAiResponses => {
            let bearer = zeroize::Zeroizing::new(format!("Bearer {api_key}"));
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&bearer) {
                headers.insert("Authorization", v);
            }
        }
        ApiStyle::Anthropic => {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(api_key) {
                headers.insert("x-api-key", v);
            }
            headers.insert("anthropic-version", reqwest::header::HeaderValue::from_static(ANTHROPIC_VERSION));
        }
    }
    headers
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
#[derive(Debug)]
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
            // 超时和「连不上」是两回事：请求已经在路上，多半是推理模型在思考或本地模型在
            // 加载权重。把这条人话给出去，别让用户对着一句 "operation timed out" 猜。
            Self::Timeout("请求超时：模型可能正在思考或加载权重，可减小一次发送的行数或改用本地档（长超时）".into())
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

/// OpenAI Chat Completions 的请求体。
fn chat_body(opts: &AiChatPayload) -> serde_json::Value {
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
    body
}

/// OpenAI Responses 的请求体：messages 原样映射成 `input`（system/user/assistant 都是
/// 合法角色），输出上限换名 `max_output_tokens`；`stream:false` 与这里从不流式的口径一致。
fn responses_body(opts: &AiChatPayload) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": opts.model,
        "input": opts.messages,
        "temperature": opts.temperature.unwrap_or(0.3),
        "stream": false,
    });
    if let Some(n) = opts.max_tokens {
        body["max_output_tokens"] = serde_json::json!(n);
    }
    body
}

/// Anthropic Messages 的请求体。两处形状差异：
/// - `max_tokens` **必填** —— 软件侧没给上限时用 4096（老款 opus 的输出上限是最小公共值），
///   缺席这个键整条请求会被 400 拒掉；
/// - system 消息不在 messages 里，挪到顶层 `system` 字段；相邻同角色合并成一条
///   （Anthropic 要求 user/assistant 严格交替）。
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 4096;

fn anthropic_body(opts: &AiChatPayload) -> serde_json::Value {
    let mut system: Vec<&str> = Vec::new();
    let mut turns: Vec<(&str, String)> = Vec::new();
    for m in &opts.messages {
        match m.role.as_str() {
            "system" => system.push(m.content.as_str()),
            role => match turns.last_mut() {
                Some((prev, content)) if *prev == role => {
                    content.push_str("\n\n");
                    content.push_str(&m.content);
                }
                _ => turns.push((role, m.content.clone())),
            },
        }
    }
    let mut body = serde_json::json!({
        "model": opts.model,
        "max_tokens": opts.max_tokens.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS),
        "messages": turns
            .into_iter()
            .map(|(role, content)| serde_json::json!({ "role": role, "content": content }))
            .collect::<Vec<_>>(),
    });
    if let Some(t) = opts.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if !system.is_empty() {
        body["system"] = serde_json::json!(system.join("\n\n"));
    }
    body
}

async fn chat_completion_full(opts: &AiChatPayload, timeout_ms: u64) -> Result<ChatOutcome, AiError> {
    let target = resolve(&opts.base_url)?;
    crate::ai_url::note_endpoint(&target, &opts.api_key, opts.api_style);
    let url = endpoint_url(opts.api_style, &target.base);
    let body = match opts.api_style {
        ApiStyle::OpenAiChat => chat_body(opts),
        ApiStyle::OpenAiResponses => responses_body(opts),
        ApiStyle::Anthropic => anthropic_body(opts),
    };
    let mut headers = auth_headers(opts.api_style, &opts.api_key);
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
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
    parse_outcome(opts.api_style, &data)
}

/// 各协议的正文与计费信息各在哪儿。解析只在分派点做一次 —— 批量侧只认 `ChatOutcome`。
fn parse_outcome(style: ApiStyle, data: &serde_json::Value) -> Result<ChatOutcome, AiError> {
    match style {
        ApiStyle::OpenAiChat => {
            let content = data
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AiError::Reply("响应缺少 choices[0].message.content".into()))?;
            Ok(ChatOutcome {
                content: content.to_string(),
                total_tokens: data.pointer("/usage/total_tokens").and_then(|v| v.as_u64()),
            })
        }
        ApiStyle::OpenAiResponses => parse_responses(data),
        ApiStyle::Anthropic => parse_anthropic(data),
    }
}

/// Responses 的正文在 `output[]` 里：`type == "message"` 条目的 `content[]` 中
/// `type == "output_text"` 块的 text 相连；reasoning（思考摘要）与工具条目跳过 ——
/// 推理模型的输出数组里混着它们，一并拼进正文会把思考过程当译文。有些网关
/// 直接给顶层 `output_text` 字符串，先吃它。
fn parse_responses(data: &serde_json::Value) -> Result<ChatOutcome, AiError> {
    let content = if let Some(s) = data["output_text"].as_str() {
        s.to_string()
    } else {
        let mut parts: Vec<&str> = Vec::new();
        if let Some(items) = data["output"].as_array() {
            for item in items {
                match item["type"].as_str() {
                    Some("message") | None => match item["content"].as_array() {
                        Some(blocks) => {
                            for b in blocks {
                                match b["type"].as_str() {
                                    Some("output_text") | None => {
                                        if let Some(t) = b["text"].as_str() {
                                            parts.push(t);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        None => {
                            if let Some(t) = item["content"].as_str() {
                                parts.push(t);
                            }
                        }
                    },
                    _ => {}
                }
            }
        }
        if parts.is_empty() {
            return Err(AiError::Reply("响应缺少 output[].content[].text".into()));
        }
        parts.concat()
    };
    let usage = &data["usage"];
    let total = usage["total_tokens"].as_u64().or_else(|| {
        usage["input_tokens"].as_u64().map(|i| i + usage["output_tokens"].as_u64().unwrap_or(0))
    });
    Ok(ChatOutcome { content, total_tokens: total })
}

/// Anthropic 的正文在 `content[]` 里：`type == "text"` 块直接相连（一个回复的多块是
/// 连续段落）；usage 只有 input/output 两个计数，合计即 total。`stop_reason == "max_tokens"`
/// 是**截断**：剩下的正文多半不完整，当作请求级失败报出去，批量侧的重试/对半拆开才有机会救回。
fn parse_anthropic(data: &serde_json::Value) -> Result<ChatOutcome, AiError> {
    if data["stop_reason"].as_str() == Some("max_tokens") {
        return Err(AiError::Reply(
            "输出在 max_tokens 处被截断（可调大输出上限或减少一次合并的行数）".into(),
        ));
    }
    let mut parts: Vec<&str> = Vec::new();
    if let Some(blocks) = data["content"].as_array() {
        for b in blocks {
            match b["type"].as_str() {
                Some("text") | None => {
                    if let Some(t) = b["text"].as_str() {
                        parts.push(t);
                    }
                }
                _ => {}
            }
        }
    }
    if parts.is_empty() {
        return Err(AiError::Reply("响应缺少 content[].text".into()));
    }
    let total = data["usage"]["input_tokens"].as_u64().unwrap_or(0)
        + data["usage"]["output_tokens"].as_u64().unwrap_or(0);
    Ok(ChatOutcome { content: parts.concat(), total_tokens: (total > 0).then_some(total) })
}

/// 「只要正文、不要计费」的入口：`ai_chat` 命令与 `ai_job` 的逐块翻译都在用。
/// 错误保留分类（`AiError`），批量任务靠它判断端点是否已经失联。
pub async fn chat_completion(opts: &AiChatPayload, timeout_ms: u64) -> Result<String, AiError> {
    chat_completion_full(opts, timeout_ms).await.map(|out| out.content)
}

/// 测试连接的超时也分档：推理型外接模型对一个 "ping" 也要先产出长思考（实测 12 s 起步），
/// 15 秒的旧预算形同「测试必挂」；本地模型首次加载权重更是分钟级。缺省按外接档。
pub fn ping_timeout_ms(kind: Option<crate::ai_strategy::ModelKind>) -> u64 {
    match kind {
        Some(crate::ai_strategy::ModelKind::Local) => 300_000,
        _ => 60_000,
    }
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
        model_kind: None,
        // 测试连接必须走用户选的那条协议：换协议后能不能通，就是在这里见分晓
        api_style: opts.api_style,
    };
    let started = std::time::Instant::now();
    match chat_completion_full(&minimal, ping_timeout_ms(opts.model_kind)).await {
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

async fn fetch_models(url: &str, api_key: &str, style: ApiStyle) -> Result<Vec<String>, String> {
    let req = client().get(url).timeout(Duration::from_secs(15)).headers(auth_headers(style, api_key));
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

/// 先打模型清单端点，空手而归再退到 Ollama 原生端点；两条都失败时把两条原因一起回给用户。
/// 三种协议共用 `{base}/models`（OpenAI 与 Anthropic 的清单都是 `data[].id`，解析同形），
/// 鉴权头跟着协议走；Ollama 原生 `/api/tags` 只对 chat/completions 有意义 ——
/// Ollama 不说 Responses 与 Anthropic 方言，给它列模型只会把人往跑不通的组合上引。
pub async fn list_models(base_url: &str, api_key: &str, style: ApiStyle) -> AiModelsResult {
    let target = match crate::ai_url::check_for_app(base_url) {
        Ok(t) => t,
        Err(e) => return AiModelsResult { ok: false, models: vec![], error: Some(e) },
    };
    crate::ai_url::note_endpoint(&target, api_key, style);
    let mut errors: Vec<String> = Vec::new();
    let mut urls = vec![models_url(&target.base)];
    if style == ApiStyle::OpenAiChat {
        urls.push(ollama_tags_url(&target.base));
    }
    for url in urls {
        match fetch_models(&url, api_key, style).await {
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

    /// 测试连接的超时跟档位走：外接档实测 ping 12 s（15 s 旧预算只剩 2.8 s 余量），本地档要留足加载时间
    #[test]
    fn ping_timeout_follows_the_model_kind() {
        use crate::ai_strategy::ModelKind;
        assert_eq!(ping_timeout_ms(None), 60_000);
        assert_eq!(ping_timeout_ms(Some(ModelKind::Remote)), 60_000);
        assert_eq!(ping_timeout_ms(Some(ModelKind::Local)), 300_000);
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

    // ── 三种协议的线形状 ─────────────────────────────────────────

    use crate::model::{ApiStyle, ChatMessage};

    fn opts(style: ApiStyle) -> AiChatPayload {
        AiChatPayload {
            base_url: "https://api.example.com".into(),
            api_key: "k".into(),
            model: "m".into(),
            messages: vec![
                ChatMessage { role: "system".into(), content: "sys".into() },
                ChatMessage { role: "user".into(), content: "hello".into() },
            ],
            temperature: Some(0.3),
            max_tokens: None,
            model_kind: None,
            api_style: style,
        }
    }

    /// 版本段交给 ai_url 的规范化（补 `/v1`），三种协议只差末段路径
    #[test]
    fn each_protocol_appends_its_own_path() {
        let base = "https://api.example.com/v1";
        assert_eq!(endpoint_url(ApiStyle::OpenAiChat, base), "https://api.example.com/v1/chat/completions");
        assert_eq!(endpoint_url(ApiStyle::OpenAiResponses, base), "https://api.example.com/v1/responses");
        assert_eq!(endpoint_url(ApiStyle::Anthropic, base), "https://api.example.com/v1/messages");
    }

    /// Anthropic 用 x-api-key + 版本头；OpenAI 两代都是 Bearer
    #[test]
    fn auth_headers_follow_the_protocol() {
        let openai = auth_headers(ApiStyle::OpenAiChat, "k");
        assert!(openai.get("Authorization").unwrap().to_str().unwrap().ends_with(" k"));
        let responses = auth_headers(ApiStyle::OpenAiResponses, "k");
        assert!(responses.get("Authorization").is_some());
        let anthropic = auth_headers(ApiStyle::Anthropic, "k");
        assert_eq!(anthropic.get("x-api-key").unwrap().to_str().unwrap(), "k");
        assert_eq!(anthropic.get("anthropic-version").unwrap().to_str().unwrap(), ANTHROPIC_VERSION);
        // 没 Key 就没有鉴权头：本地 Ollama 不需要
        assert!(auth_headers(ApiStyle::OpenAiChat, "").get("Authorization").is_none());
    }

    /// Anthropic：system 挪到顶层、max_tokens 必填（缺省 4096）、温度可选
    #[test]
    fn the_anthropic_body_lifts_system_and_fills_max_tokens() {
        let mut o = opts(ApiStyle::Anthropic);
        let b = anthropic_body(&o);
        assert_eq!(b["system"], "sys");
        assert_eq!(b["max_tokens"], ANTHROPIC_DEFAULT_MAX_TOKENS, "软件侧没给上限也要填，缺席会被 400 拒掉");
        assert_eq!(b["messages"].as_array().unwrap().len(), 1, "system 不留在 messages 里");
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["temperature"], 0.3);

        o.max_tokens = Some(77);
        o.temperature = None;
        let b = anthropic_body(&o);
        assert_eq!(b["max_tokens"], 77);
        assert!(b.get("temperature").is_none(), "温度缺席就不写键");
    }

    /// Anthropic 要求 user/assistant 严格交替：相邻同角色合并成一条
    #[test]
    fn the_anthropic_body_merges_consecutive_same_roles() {
        let mut o = opts(ApiStyle::Anthropic);
        o.messages.push(ChatMessage { role: "user".into(), content: "second".into() });
        let b = anthropic_body(&o);
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "hello\n\nsecond");
    }

    #[test]
    fn the_responses_body_renames_the_output_cap() {
        let b = responses_body(&opts(ApiStyle::OpenAiResponses));
        assert_eq!(b["input"].as_array().unwrap().len(), 2, "messages 原样映射成 input");
        assert_eq!(b["stream"], false);
        assert!(b.get("max_output_tokens").is_none(), "没给上限就不写键");
        let mut o = opts(ApiStyle::OpenAiResponses);
        o.max_tokens = Some(64);
        assert_eq!(responses_body(&o)["max_output_tokens"], 64);
    }

    #[test]
    fn responses_reply_reads_message_items_and_skips_reasoning() {
        let data: serde_json::Value = serde_json::from_str(
            r#"{"output":[
                {"type":"reasoning","content":[{"type":"summary_text","text":"thinking"}]},
                {"type":"message","role":"assistant","content":[
                    {"type":"output_text","text":"1. 你好"},{"type":"output_text","text":"2. 再见"}]}],
              "usage":{"input_tokens":10,"output_tokens":20}}"#,
        )
        .unwrap();
        let out = parse_responses(&data).unwrap();
        assert_eq!(out.content, "1. 你好2. 再见");
        assert_eq!(out.total_tokens, Some(30), "没有 total_tokens 时按 input+output 合计");

        // 网关直接给顶层 output_text 的形状也吃
        let shortcut: serde_json::Value = serde_json::from_str(r#"{"output_text":"hi"}"#).unwrap();
        assert_eq!(parse_responses(&shortcut).unwrap().content, "hi");

        let empty: serde_json::Value = serde_json::from_str(r#"{"output":[{"type":"message"}]}"#).unwrap();
        assert!(parse_responses(&empty).is_err(), "没有正文要当请求级失败");
    }

    #[test]
    fn anthropic_reply_joins_text_blocks_and_reports_truncation() {
        let data: serde_json::Value = serde_json::from_str(
            r#"{"content":[{"type":"text","text":"1. 你好"},{"type":"text","text":"2. 再见"}],
                "stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":20}}"#,
        )
        .unwrap();
        let out = parse_anthropic(&data).unwrap();
        assert_eq!(out.content, "1. 你好2. 再见");
        assert_eq!(out.total_tokens, Some(30));

        let truncated: serde_json::Value =
            serde_json::from_str(r#"{"content":[{"type":"text","text":"半截"}],"stop_reason":"max_tokens"}"#).unwrap();
        let e = parse_anthropic(&truncated).expect_err("截断的正文不该被当成完整译文");
        assert!(e.text().contains("max_tokens"), "{}", e.text());

        let thinking: serde_json::Value = serde_json::from_str(
            r#"{"content":[{"type":"thinking","thinking":"…"},{"type":"text","text":"答案"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_anthropic(&thinking).unwrap().content, "答案", "思考块不进正文");
    }

    /// parse_outcome 只是个分派点：chat/completions 的旧路径原样保留
    #[test]
    fn parse_outcome_dispatches_by_protocol() {
        let chat: serde_json::Value = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"hi"}}],"usage":{"total_tokens":5}}"#,
        )
        .unwrap();
        assert_eq!(parse_outcome(ApiStyle::OpenAiChat, &chat).unwrap().content, "hi");
        assert_eq!(parse_outcome(ApiStyle::OpenAiChat, &chat).unwrap().total_tokens, Some(5));
        let responses: serde_json::Value =
            serde_json::from_str(r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"yo"}]}]}"#)
                .unwrap();
        assert_eq!(parse_outcome(ApiStyle::OpenAiResponses, &responses).unwrap().content, "yo");
    }
}

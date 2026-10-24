//! 契约层 DTO：与前端 src/contracts/api.ts 逐字段对齐（serde camelCase）。

use serde::{Deserialize, Serialize};

pub fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    while s.ends_with('/') {
        s.pop();
    }
    s
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub path: String,
    pub duration: f64,
    pub artist: String,
    pub album: String,
    pub format: String,
    pub is_video: bool,
    pub cover_path: Option<String>,
    #[serde(default)]
    pub cover_data: Option<String>,
    pub lyrics_path: Option<String>,
    pub file_mtime: f64,
    pub file_size: i64,
    pub meta_title: Option<String>,
    pub meta_artist: Option<String>,
    pub genre: Option<String>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    #[serde(default)]
    pub loudness_lufs: Option<f64>,
    #[serde(default)]
    pub album_cover_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub name: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_data: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub name: String,
    pub path: String,
    pub albums: Vec<Album>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub name: String,
    pub path: String,
    pub children: Vec<FolderNode>,
    pub tracks: Vec<Track>,
    pub track_count: i64,
    pub cover_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub folder_paths: Vec<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub folder_tree: Vec<FolderNode>,
    pub all_tracks: Vec<Track>,
    pub file_count: i64,
    #[serde(skip)]
    pub scanned_at: i64,
    #[serde(default)]
    pub scan: ScanStatus,
}

/// 本轮扫描的收口状态（P0-4）。只有 `complete` 才允许「本轮没见到就当删除」，
/// 其余三种都必须保留旧条目 —— 前端也据此决定要不要用这份结果替换曲库。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    /// `complete` / `partial` / `cancelled` / `failed`
    pub outcome: String,
    /// 给人看的原因（UI 直接显示），`complete` 时为空
    pub reasons: Vec<String>,
}

pub const SCAN_COMPLETE: &str = "complete";
pub const SCAN_PARTIAL: &str = "partial";
pub const SCAN_CANCELLED: &str = "cancelled";
pub const SCAN_FAILED: &str = "failed";

impl ScanStatus {
    pub fn complete() -> Self {
        Self { outcome: SCAN_COMPLETE.into(), reasons: Vec::new() }
    }

    pub fn new(outcome: &str, reasons: Vec<String>) -> Self {
        Self { outcome: outcome.into(), reasons }
    }

    pub fn is_complete(&self) -> bool {
        self.outcome == SCAN_COMPLETE
    }
}

impl Default for ScanStatus {
    fn default() -> Self {
        Self::complete()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayProgress {
    pub track_id: String,
    pub position_ms: i64,
    pub completed: bool,
    pub play_count: i64,
    pub played_at: i64,
    pub last_speed: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub track_id: String,
    pub at_ms: i64,
    pub label: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BgImageData {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<f64>,
    /// 背景图的 rec.601 平均感知亮度（0–255）。解码失败时为 None，前端据此回落到主题底色。
    /// 在 Rust 侧算而不是前端 canvas：asset 协议跨源会 taint canvas，`getImageData` 必抛。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub luma: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub device_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 接口协议：同一份接入参数（地址 / Key / 模型名）按三种线形状之一发。
/// 版本段都由 `ai_url` 的规范化统一补成 `/v1`，三种协议的差别只在末段路径、
/// 鉴权头与请求/响应的 JSON 形状 —— 缺省 `OpenAiChat` = 既有行为，老前端与
/// 老设置不带这个字段时照旧走 chat/completions。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ApiStyle {
    /// OpenAI Chat Completions：`POST {base}/chat/completions`，Bearer 鉴权（历史缺省）
    #[default]
    #[serde(rename = "openai-chat")]
    OpenAiChat,
    /// OpenAI Responses（新版）：`POST {base}/responses`，同为 Bearer 鉴权
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    /// Anthropic Messages：`POST {base}/messages`，`x-api-key` + `anthropic-version` 鉴权
    #[serde(rename = "anthropic")]
    Anthropic,
}

impl ApiStyle {
    /// 设置与日志里的稳定名（与 serde 线值一致；derive 的 Debug 名是实现细节）
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai-chat",
            Self::OpenAiResponses => "openai-responses",
            Self::Anthropic => "anthropic",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatPayload {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// `ai_ping` 只带三个凭证字段过来，messages 由 [`crate::ai::ping`] 自己补。
    /// 没有 `default` 时 serde 会因缺字段直接拒绝整个请求，测试连接必然失败。
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f64>,
    /// 输出上限（`04 §7.6.4`）：按原文长度收紧，防本地模型进入复读或长篇解释。
    /// `None` = 不由软件侧限制（字幕那条链路的既有行为，参数是第 11 批定的）。
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// 模型档位，只有 `ai_ping` 消费：测试连接的超时跟着档位走（本地模型加载权重
    /// 与推理模型的长思考都不是 15 秒级的事）。`ai_chat` 与批量任务不看它。
    #[serde(default)]
    pub model_kind: Option<crate::ai_strategy::ModelKind>,
    /// 接口协议，缺省 `openai-chat`（历史行为）。请求的 URL/鉴权头/线形状与响应解析都跟着它走。
    #[serde(default)]
    pub api_style: ApiStyle,
}

/// API Key 只该活在这条请求里。`String` 被丢弃不会擦掉堆上的字节 —— 那块内存会被
/// 下一个分配者拿到（P0-3：任务线程里的秘密要有等价于 `SecretString` 的生命周期控制）。
impl Drop for AiChatPayload {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
    }
}

/// 手写而非 derive：`{:?}` 一份请求就是一条带 Key 的日志，**输出里永远只有长度**。
impl std::fmt::Debug for AiChatPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiChatPayload")
            .field("base_url", &self.base_url)
            .field("api_key", &crate::ai::redact(&self.api_key))
            .field("model", &self.model)
            .field("messages", &self.messages.len())
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("model_kind", &self.model_kind)
            .field("api_style", &self.api_style.name())
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct AiChatResult {
    pub ok: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPingResult {
    pub ok: bool,
    pub message: String,
    /// 连通性测试顺带回显的档位信息（AI-5）：模型名、本次往返毫秒数、上游报的 token 数。
    /// 三个都是 `Option` + `skip_serializing_if`：失败时只留 `message` 原文。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct AiModelsResult {
    pub ok: bool,
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `ai_endpoint_check` 的回执：地址现状 + 被拦原因 + 「放行即可用」的端点。
/// 给界面的显式放行按钮供靶 —— 公网/私网判断只活在 Rust 一处，前端不复刻规则。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiEndpointPolicy {
    pub ok: bool,
    /// ok=false 时的原因原文（与测试连接看到的一致）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 被拦、但显式放行这个 `host:port` 就能用；缺席 = 放行也救不了（地址本身不合规/本机被拦）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// 当前放行名单，随每条回执带上（前端免一次专门查询）
    pub allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    /// mpv 轨 id（<=0 表示「关闭字幕」）
    pub id: i64,
    pub title: String,
    pub lang: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub volume: f64,
    pub muted: bool,
    pub track_path: Option<String>,
    pub video_active: bool,
    /// 画面显示尺寸（mpv video-params dwidth/dheight，含像素长宽比）。
    /// 0 = 尚未拿到（无视频轨或还在加载）。悬浮窗据此把窗口锁成同一比例，
    /// 避免 mpv 在窗口内留 letterbox 黑边。
    #[serde(default)]
    pub video_w: f64,
    #[serde(default)]
    pub video_h: f64,
    /// 加载代际（P0-5）：前端每次 `player_play` 自增一个数随命令带下来。
    /// 状态广播带上它，前端才能认出「这是上一首的迟到状态」并丢弃。
    /// Rust 侧只做单调记账（`max`），不生成代际。
    #[serde(default)]
    pub generation: u64,
}

/// 字幕样式（mpv sub-* 属性遥控）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubStyle {
    pub font: Option<String>,
    pub font_size: Option<f64>,
    pub color: Option<String>,
    pub border_color: Option<String>,
    pub border_size: Option<f64>,
    pub shadow_offset: Option<f64>,
    pub pos: Option<f64>,
    /// 背景条基色 `#RRGGBB`（P0-24）。与 `back_opacity` 一起由 Rust 合成 mpv 的
    /// `#AARRGGBB`：`<input type="color">` 只能给 6 位十六进制，透明度必须另传。
    pub back_color: Option<String>,
    /// 背景条不透明度 0–100；0 = 完全透明（mpv 默认，即「没有背景条」）。
    pub back_opacity: Option<f64>,
    /// `sub-ass-override`（P0-25）：`no`(保留 ASS 样式) / `yes`(本面板样式覆盖) /
    /// `force` / `scale` / `strip`。非法值由 mpv 自己拒绝，这里不猜。
    pub ass_override: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::AiChatPayload;

    /// 线格式回归守卫：`ai_ping` 只发三个凭证字段（见 `bridge/ipc.ts`），
    /// `messages` 一旦回到必填，serde 会拒掉整个请求，测试连接静默失败。
    #[test]
    fn ai_chat_payload_accepts_a_ping_shaped_request() {
        let p: AiChatPayload =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m"}"#).unwrap();
        assert!(p.messages.is_empty());
        assert!(p.temperature.is_none());
        assert_eq!(p.model_kind, None, "档位缺席 = 按外接档给测试超时");
        let with_kind: AiChatPayload =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","modelKind":"local"}"#).unwrap();
        assert_eq!(with_kind.model_kind, Some(crate::ai_strategy::ModelKind::Local));
    }

    /// 接口协议是可选的：老前端不带 `apiStyle` 时必须照旧走 chat/completions，
    /// 不能因为加了新协议就让存量请求反序列化失败。
    #[test]
    fn ai_chat_payload_defaults_to_the_legacy_protocol() {
        use super::ApiStyle;
        let p: AiChatPayload =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m"}"#).unwrap();
        assert_eq!(p.api_style, ApiStyle::OpenAiChat);
        let responses: AiChatPayload =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","apiStyle":"openai-responses"}"#).unwrap();
        assert_eq!(responses.api_style, ApiStyle::OpenAiResponses);
        let anthropic: AiChatPayload =
            serde_json::from_str(r#"{"baseUrl":"u","apiKey":"k","model":"m","apiStyle":"anthropic"}"#).unwrap();
        assert_eq!(anthropic.api_style, ApiStyle::Anthropic);
        // 稳定名与 serde 线值一致：日志与设置里出现的是这一个形状
        assert_eq!(ApiStyle::default().name(), "openai-chat");
    }
}

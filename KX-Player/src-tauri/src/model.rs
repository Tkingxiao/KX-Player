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

#[derive(Debug, Deserialize)]
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
    }
}

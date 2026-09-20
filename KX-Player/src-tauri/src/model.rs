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
pub struct AiPingResult {
    pub ok: bool,
    pub message: String,
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
    pub is_video: bool,
    pub video_active: bool,
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
}

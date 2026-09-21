//! IPC 错误信封（`02 §6.1`）。
//!
//! 形状上仍是 `Result<T, IpcError>`：Tauri 对 `Err` 走 **reject**，前端 `bridge/ipc.ts` 里
//! 48 处 `.catch(fallback)` 正是靠这个语义工作。若把信封改成「正常 resolve 一个 {ok:false}」，
//! 这些 catch 会全部变成死代码，而配套的 `.then(() => true)` 会把失败当成功 —— 静默反转。
//! 因此本模块只负责把 reject 的载荷从「一坨字符串」升级成「{code, message, detail}」。

use std::fmt::Display;

use serde::Serialize;

/// 错误码。序列化为 SCREAMING_SNAKE_CASE，前端 `contracts/result.ts` 的 `AppErrorCode` 与之对齐。
///
/// 按 `02 §6.1` 全量定义，本批次只有部分码有构造点 —— `MpvEmbedFailed`/`SubtitleParseFailed`/
/// `RemuxFailed` 等要等 B4/B3 才用得上，故不裁成「只留用得上的」。
#[allow(dead_code)] // 码表是契约的一部分，未构造 ≠ 可删
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppErrorCode {
    NotFound,
    PermissionDenied,
    IoFailed,
    DbFailed,
    DecodeFailed,
    UnsupportedFormat,
    MpvInitFailed,
    MpvEmbedFailed,
    RemuxFailed,
    SubtitleParseFailed,
    EncodingUnknown,
    Busy,
    InvalidArgument,
    Cancelled,
    Internal,
}

/// 穿越 IPC 的错误载荷。`message` 面向用户（中文，可直接进 Toast），
/// `detail` 面向日志（驱动器原始报错、panic 线索），前端默认不展示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: AppErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub type IpcResult<T> = Result<T, IpcError>;

/// `Debug` 形状与 `Display` 一致，日志里直接 `{e}` 或 `{e:?}` 都是同一行可读文本。
impl Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(d) => write!(f, "[{:?}] {}；detail: {d}", self.code, self.message),
            None => write!(f, "[{:?}] {}", self.code, self.message),
        }
    }
}

impl std::error::Error for IpcError {}

impl IpcError {
    pub fn new(code: AppErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: None }
    }

    pub fn with_detail(code: AppErrorCode, message: impl Into<String>, detail: impl Display) -> Self {
        Self { code, message: message.into(), detail: Some(detail.to_string()) }
    }

    /// `spawn_blocking` 的任务 panic 或被取消。原始 `JoinError` 只能给出「task panicked」这类
    /// 英文线索，所以进 `detail`，`message` 留给用户看得懂的话。
    pub fn join_failed(task: &str, e: impl Display) -> Self {
        Self::with_detail(AppErrorCode::Internal, format!("后台任务「{task}」异常终止"), e)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(AppErrorCode::InvalidArgument, message)
    }

    /// `PlayerCore` 的句柄还没建起来就收到播放指令。与「mpv 初始化失败」同码，
    /// 因为对用户的处置一样：重新触发一次起播。
    pub fn mpv_not_started() -> Self {
        Self::new(AppErrorCode::MpvInitFailed, "播放器尚未初始化")
    }

    pub fn db(e: impl Display) -> Self {
        Self::with_detail(AppErrorCode::DbFailed, "数据库操作失败", e)
    }

    pub fn io(e: impl Display) -> Self {
        Self::with_detail(AppErrorCode::IoFailed, "文件操作失败", e)
    }
}

impl From<rusqlite::Error> for IpcError {
    fn from(e: rusqlite::Error) -> Self {
        Self::db(e)
    }
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppErrorCode, IpcError};
    use crate::commands::join_task;

    /// 线上形状是 Rust 与 `contracts/result.ts` 之间唯一的约定，serde 属性写错不会编译失败，
    /// 只会在前端静默变成 `undefined` —— 所以钉在这里。
    #[test]
    fn wire_shape_is_camel_case_with_screaming_snake_code() {
        let v = serde_json::to_value(IpcError::with_detail(AppErrorCode::InvalidArgument, "名称不能为空", "UNIQUE constraint failed"))
            .expect("IpcError 必然可序列化");
        assert_eq!(v["code"], "INVALID_ARGUMENT");
        assert_eq!(v["message"], "名称不能为空");
        assert_eq!(v["detail"], "UNIQUE constraint failed");

        // detail 为 None 时整键缺席，前端的 `detail?: string` 才成立
        assert_eq!(
            serde_json::to_string(&IpcError::invalid_argument("参数错误")).unwrap(),
            r#"{"code":"INVALID_ARGUMENT","message":"参数错误"}"#
        );
    }

    /// `join_task` 的两层语义：内层业务错误原样穿过，外层「任务崩了」才换成 Internal。
    /// 旧写法两者都压成「任务失败」，用户和日志都分不开。
    #[test]
    fn join_task_preserves_inner_error_and_only_wraps_the_panic() {
        let inner: super::IpcResult<i64> = Err(IpcError::invalid_argument("已存在同名标签"));
        let e = join_task::<i64, &str>("保存标签", Ok(inner)).expect_err("内层错误必须穿过");
        assert_eq!(e.code, AppErrorCode::InvalidArgument);
        assert_eq!(e.message, "已存在同名标签");

        let e = join_task::<i64, _>("切换播放/暂停", Err("task 7 panicked")).expect_err("外层错误必须换码");
        assert_eq!(e.code, AppErrorCode::Internal);
        assert_eq!(e.message, "后台任务「切换播放/暂停」异常终止");
        assert_eq!(e.detail.as_deref(), Some("task 7 panicked"));
    }
}

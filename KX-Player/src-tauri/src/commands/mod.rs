//! 命令层聚合。

use crate::error::{IpcError, IpcResult};

pub mod dialog_fs;
pub mod library;
pub mod player;
pub mod rename;
pub mod system;
pub mod taxonomy;

/// 收口 `spawn_blocking(..).await` 的双层 `Result`：外层是「后台任务 panic / 被取消」，
/// 内层才是业务错误。
///
/// 旧写法 `.map_err(|_| "任务失败".to_string())?` 把两者混成同一句话，用户和日志都分不清
/// 「后端崩了」和「这个名字已存在」。这里前者进 `detail`（`Internal` 码），后者原样穿过。
/// 错误类型用泛型而非具体名，是因为 `JoinError` 属 tokio 重导出，命令层不该依赖它。
pub fn join_task<T, E: std::fmt::Display>(what: &str, joined: Result<IpcResult<T>, E>) -> IpcResult<T> {
    joined.map_err(|e| IpcError::join_failed(what, e))?
}

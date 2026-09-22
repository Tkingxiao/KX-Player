//! 改名命令层（`04 §7`）。目前只有一条**只读**命令：干跑预览。
//!
//! 应用与回滚（`04 §7.8`）刻意留到下一批：先让用户看见「规则会改哪些、哪些要问模型、哪里撞名」，
//! 确认判定没有跑偏，再给写盘的按钮 —— 中间没有第二次机会的那一步不该和它的检查同批上线。

use crate::commands::join_task;
use crate::error::{IpcError, IpcResult};
use crate::rename::preview::Preview;

/// 改名干跑预览：把选中的目录各往下看一层，返回档位分布 + 会变动的条目 + 冲突。
/// **一个字节都不写**：只 `read_dir`，不改名、不建批次、不写 history。
/// 目录列不出来时不静默当空目录，路径进 `unreadable` 如实上报（`04 §7.1` 第 5 条的边界）。
#[tauri::command]
pub async fn rename_preview(paths: Vec<String>) -> IpcResult<Preview> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中要预览的目录"));
    }
    join_task(
        "预览改名",
        tauri::async_runtime::spawn_blocking(move || Ok(crate::rename::listing::preview_roots(&paths)))
            .await,
    )
}

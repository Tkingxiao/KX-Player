//! 一次写盘的**汇总形状**，与 `apply` / `rollback` 共用的两件机械：当项撤销 `undo`、
//! history 那侧的记账 `sync`。
//!
//! 之所以不从 `apply.rs` 里引用：两边都要用，而「谁共用谁」这种方向一旦写进 import 就会骗人
//! （回滚并不是应用的下游）。汇总与撤销是**结果形状**，与方向无关。

use crate::rename::batch::{self, Pair};
use crate::rename::paths::move_one;

/// 一次执行的汇总。`failed` 是**明细**而不是一个数 —— 三百个文件里失败两个，只报数字没法查。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub batch_id: String,
    /// 进了队列的条数（不含跳过项）
    pub planned: usize,
    pub applied: usize,
    /// 没进队列的条数：要问模型的 + 撞名的；回滚时是「盘上已经不是新名」的条数
    pub skipped: usize,
    pub failed: Vec<String>,
    /// 追加进了哪份 `rename_history.json`；`None` = 那一层没有这个文件（不是失败，但要如实显示）
    pub history_file: Option<String>,
    pub history_written: usize,
}

impl Report {
    pub(crate) fn new(batch_id: String, planned: usize, skipped: usize) -> Self {
        Self {
            batch_id,
            planned,
            applied: 0,
            skipped,
            failed: Vec::new(),
            history_file: None,
            history_written: 0,
        }
    }
}

/// history 那一侧的账。找不到文件不是失败（`04 §7.1` 只说「有就追加」），但读写出问题必须报。
pub(crate) fn sync(rep: &mut Report, roots: &[String], done: &[Pair], undo: bool) {
    if done.is_empty() {
        return;
    }
    match batch::sync_history(roots, done, undo) {
        Ok((file, n)) => {
            rep.history_file = file;
            rep.history_written = n;
        }
        Err(e) => rep.failed.push(format!("rename_history.json：{e}")),
    }
}

/// 撤销一次已经成功的改名。撤销也失败就是最坏的情况：盘上改了名而库里没账，**必须报出来**，
/// 不能 `let _ =` 咽掉 —— 否则现象是「那个文件怎么也回不去」而日志一个字都没有。
pub(crate) fn undo(rep: &mut Report, from: &str, to: &str) {
    if let Err(e) = move_one(from, to) {
        rep.failed.push(format!("{from}：撤销失败，名字停在改名后（{e}）"));
    }
}

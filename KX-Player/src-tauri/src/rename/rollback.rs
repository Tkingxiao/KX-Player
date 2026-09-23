//! **反向执行**（`04 §7.8`）：整批回滚，以及「回滚的回滚」= 重做。
//!
//! 与 `apply.rs` 共用一台机器：`report.rs` 的汇总形状 `Report`、当项撤销 `undo`、history 那侧的
//! `sync`，以及 `paths.rs` 的顺序与换算。真正的区别只有三条 —— 挑哪些行（`applied` 旗子的正反）、
//! 往哪个方向搬、以及顺序反过来（`sort_pairs` 的 `undo` 参数）。
//!
//! 依据是**库里当时落下的映射**，不是重新预览：盘后来被谁动过，逐条复核并把失败明细报出来，
//! 不静默、也不替用户猜「大概已经改回去了」。

use std::path::Path;

use rusqlite::Connection;

use crate::rename::batch::{self, Pair};
use crate::rename::paths::{move_one, parents_of, resolve, sort_pairs};
use crate::rename::report::{sync, undo, Report};

/// 整批回滚（`04 §7.8`）：文件夹自顶向下先回、文件最后回；动手前逐条复核现名还在不在。
pub fn rollback(conn: &Connection, batch_id: &str) -> Result<Report, String> {
    reverse(conn, batch_id, false)
}

/// 回滚的回滚：把这一批再执行一遍（`04 §7.8` 最后一条）。账还在，所以不需要重新判定。
pub fn redo(conn: &Connection, batch_id: &str) -> Result<Report, String> {
    reverse(conn, batch_id, true)
}

/// 回滚与重做共用一台机器：都是「把一条映射照库里记的样子再搬一次」，区别只在挑哪些行、
/// 往哪个方向搬、执行顺序，以及批次标记。
fn reverse(conn: &Connection, batch_id: &str, forward: bool) -> Result<Report, String> {
    let label = if forward { "重做" } else { "回滚" };
    if !batch::exists(conn, batch_id)? {
        return Err(format!("找不到批次 {batch_id}"));
    }
    // 回滚只回「现在盘上是新名」的行（applied=1）；重做正好相反，补上当初没生效的那些（applied=0）
    let mut pairs = batch::pairs(conn, batch_id, i64::from(!forward))?;
    if pairs.is_empty() {
        return Err(format!("批次 {batch_id} 没有可{label}的条目"));
    }
    // 重做就是再应用一遍，顺序与 `04 §7.7` 相同（深目录先）；回滚把方向反过来：**文件夹自顶向下
    // 先回，文件最后回**，这样每一行动手时它存的两端都是**当时真实**的路径。
    sort_pairs(&mut pairs, !forward);

    let mut rep = Report::new(batch_id.to_string(), pairs.len(), 0);
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut done: Vec<Pair> = Vec::new();
    for p in &pairs {
        // `applied` 说的就是「盘上现在是不是 new」，所以现名/目标名由方向决定
        let (cur, want) = if forward { (&p.old, &p.new) } else { (&p.new, &p.old) };
        let from = resolve(cur, &moved);
        let to = resolve(want, &moved);
        if !Path::new(&from).exists() {
            // 重做时「这条当年就没改成」是常态，算跳过；回滚时它就是盘被动过了，要报
            if forward {
                rep.skipped += 1;
            } else {
                rep.failed.push(format!("{from}：现名已不存在，未改动"));
            }
            continue;
        }
        if let Err(e) = move_one(&from, &to) {
            rep.failed.push(format!("{from}：{e}"));
            continue;
        }
        match batch::set_item(conn, batch_id, p, forward) {
            Ok(()) => {
                rep.applied += 1;
                moved.push((cur.clone(), want.clone()));
                done.push(p.clone());
            }
            Err(e) => {
                rep.failed.push(format!("{from}：{e}，已撤销"));
                undo(&mut rep, &to, &from);
            }
        }
    }
    let note = format!(
        "{label} · 计划 {} · 成功 {} · 跳过 {} · 失败 {}",
        rep.planned,
        rep.applied,
        rep.skipped,
        rep.failed.len()
    );
    batch::mark_rolled_back(conn, batch_id, !forward, &note)?;
    let roots = parents_of(&done);
    sync(&mut rep, &roots, &done, true);
    Ok(rep)
}

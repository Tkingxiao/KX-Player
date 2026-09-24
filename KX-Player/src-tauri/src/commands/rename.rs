//! 改名命令层（`04 §7`）。七条命令共用**同一份输入**：用户勾出来的目录，或库里已有的批次号。
//!
//! 预览与落地之间不传条目：`rename_apply` 收到 `paths` 后自己重算一遍预览，与屏幕上那次跑的是
//! 同一个纯函数。前端回传「要改哪些」等于把判定权交给一份可能已经过期的列表 —— 列表一变，
//! 手上那份映射就指向别的文件（接力棒坑 33）。模型译名同理：`rename_suggest_names` 只把名字
//! **写进库**，落地的仍然是 `rename_apply` 按片段查出来的那一份；`rename_reject_suggestion` 也只是
//! 往拒绝表里写一行，显示侧与落地侧共用同一条判据（`ai_ledger::ledger_state`，`04 §7.7` 第 9 条）——
//! 预览说「跳过」的那一条，落地时确实一个字都不会动它。
//!
//! 会写盘的是 `rename_apply` / `rename_rollback` / `rename_redo` 三条，它们只动**已经定稿**的那部分
//! （本地规则改完就算定稿，要问模型的得先在译名库里过完九道校验；撞名的都不碰），
//! 每改成一项立刻落库一行（`04 §7.7` 第 3 条）。

use rusqlite::Connection;

use crate::ai_strategy::ModelKind;
use crate::commands::join_task;
use crate::error::{IpcError, IpcResult};
use crate::rename::ai_ask::{self, NameReport};
use crate::rename::ai_ledger;
use crate::rename::batch::Batch;
use crate::rename::judge::Skipped;
use crate::rename::preview::Preview;
use crate::rename::report::Report;
use crate::scope::PathScope;

/// 三条会走盘的入口共用的闸门（P0-1）：目录必须落在授权根里。
/// 授权只有两个来路 —— 对话框选中的文件夹、库里已登记的根目录；前端换一条路径递进来，
/// 这里给 `INVALID_ARGUMENT`（`PermissionDenied` 那条码留给「命令自己能读、但对象不该被碰」的场景）。
fn guard_scope(scope: &PathScope, paths: &[String]) -> IpcResult<()> {
    scope.check_all(paths).map_err(|(bad, why)| {
        crate::paths::append_log(&format!("[scope] 改名拒绝 {bad}（{why}）"));
        IpcError::outside_scope(&bad, why)
    })
}

/// 改名干跑预览：把选中的目录各往下看一层，返回档位分布 + 会变动的条目 + 冲突。
/// **一个字节都不写**：只 `read_dir` 加读一次译名库（给已经问到、且没被拒过的名字盖章），
/// 不改名、不建批次、不写 history。
/// 目录列不出来时不静默当空目录，路径进 `unreadable` 如实上报（`04 §7.1` 第 5 条的边界）。
///
/// 判定树手里没有库，所以「锁定 / 已拒绝」（`04 §7.2` 第 1 条）靠一个探针按库里的事实回答：
/// 这里给的就是 `apply` 后来会用的那条判据（被拒过**那个名字**才跳过），预览列出的档位因此
/// 与落地做的事对得上 —— 不会出现「预览说跳过、落地照改名」。
#[tauri::command]
pub async fn rename_preview(scope: tauri::State<'_, PathScope>, paths: Vec<String>) -> IpcResult<Preview> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中要预览的目录"));
    }
    guard_scope(&scope, &paths)?;
    join_task(
        "预览改名",
        tauri::async_runtime::spawn_blocking(move || {
            in_db(|conn| {
                let mut p = crate::rename::listing::preview_roots_with(&paths, &|it| {
                    ai_ledger::blocked_by_refusal(conn, it).then_some(Skipped::Rejected)
                });
                ai_ledger::annotate(conn, &mut p);
                Ok(p)
            })
        })
        .await,
    )
}

/// 应用这一批已经定稿的改名。返回值里的 `applied` 才是真改了多少条 ——
/// `planned` 是队列长度，`skipped` 是「模型译名还没生成 / 撞名」而**这一批没替用户做**的条数。
#[tauri::command]
pub async fn rename_apply(scope: tauri::State<'_, PathScope>, paths: Vec<String>) -> IpcResult<Report> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中要应用的目录"));
    }
    guard_scope(&scope, &paths)?;
    join_task(
        "应用改名",
        tauri::async_runtime::spawn_blocking(move || in_db(|conn| crate::rename::apply::apply(conn, &paths)))
            .await,
    )
}

/// 生成模型译名（`04 §7.6~7.7`）：清点要问模型的片段 → 一个片段一次请求 → 过九道校验 →
/// 把核准的名字写进 `rename_name_cache`。**一个字节都不改盘**，改盘仍然是 `rename_apply` 的事。
///
/// 这是一条会长跑的命令（外接模型几十秒、本地模型可能几分钟），但它是 `async` 的：请求在
/// tokio 上跑，主线程与窗口不因此停顿（第 12 批那次卡死是**同步**命令里做重活）。
/// 代价是这一条没有取消按钮，所以 `ai_ledger::MAX_NAMES` 把一次的上限压到 200 个名字。
#[tauri::command]
pub async fn rename_suggest_names(
    scope: tauri::State<'_, PathScope>,
    paths: Vec<String>,
    base_url: String,
    api_key: String,
    model: String,
    model_kind: Option<ModelKind>,
) -> IpcResult<NameReport> {
    if paths.is_empty() {
        return Err(IpcError::invalid_argument("没有选中要生成译名的目录"));
    }
    guard_scope(&scope, &paths)?;
    // 地址不合规就别起任务：这一条没有取消按钮，等它把 200 个名字逐个撞一遍
    // 再报「全部失败」是最差的一种反馈（P0-3）
    crate::ai_url::check(&base_url).map_err(IpcError::invalid_argument)?;
    // Tauri 交过来的 String 用完就擦：这条链路里 Key 会活到整批问完为止（P0-3）
    let api_key = zeroize::Zeroizing::new(api_key);
    let kind = model_kind.unwrap_or_default();
    let plan = join_task(
        "清点待生成译名",
        tauri::async_runtime::spawn_blocking(move || in_db(|conn| ai_ledger::plan(conn, &paths))).await,
    )?;
    // 全部命中缓存是常态（同一批名字再生成一次不该再花钱），这一步一次请求都不发
    let outcomes = if plan.missing.is_empty() {
        Vec::new()
    } else {
        ai_ask::translate(&base_url, &api_key, &model, kind, &plan.missing).await
    };
    let rep = ai_ask::report(plan.needed, plan.cached, &outcomes, &model);
    join_task(
        "写入译名缓存",
        tauri::async_runtime::spawn_blocking(move || {
            in_db(|conn| ai_ledger::record(conn, &model, &outcomes).map(|_| ()))
        })
        .await,
    )?;
    Ok(rep)
}

/// 记一条「这个译名我不要」（`04 §7.7` 第 9 条）。写的只有拒绝表，**不改盘也不删缓存**：
/// 那一个名字仍然留在 `rename_name_cache` 里给别的条目用（同一片段可能出现在两个目录上），
/// 只是**在这个条目**上从此失效 —— 下一次预览与落地都会各自复核到它。
#[tauri::command]
pub async fn rename_reject_suggestion(path: String, suggestion: String) -> IpcResult<()> {
    if path.is_empty() || suggestion.is_empty() {
        return Err(IpcError::invalid_argument("要拒绝的译名或条目路径是空的"));
    }
    join_task(
        "记下这条拒绝",
        tauri::async_runtime::spawn_blocking(move || {
            in_db(|conn| ai_ledger::reject(conn, &path, &suggestion))
        })
        .await,
    )
}

/// 整批回滚：按库里那本账把名字搬回去（`04 §7.8`）。不重新预览 —— 回滚的依据是**当时**的映射。
#[tauri::command]
pub async fn rename_rollback(batch_id: String) -> IpcResult<Report> {
    join_task(
        "回滚改名",
        tauri::async_runtime::spawn_blocking(move || in_db(|conn| crate::rename::rollback::rollback(conn, &batch_id)))
            .await,
    )
}

/// 回滚的回滚：把这一批再执行一遍（`04 §7.8` 最后一条）。账还在，所以这一步不需要重新判定。
#[tauri::command]
pub async fn rename_redo(batch_id: String) -> IpcResult<Report> {
    join_task(
        "重做改名",
        tauri::async_runtime::spawn_blocking(move || in_db(|conn| crate::rename::rollback::redo(conn, &batch_id)))
            .await,
    )
}

/// 最近的改名批次（UI 靠它挑哪一批回滚/重做）。按创建时间倒序，一次最多 50 条 ——
/// 这个列表是给人挑批次用的，不是导出通道。
#[tauri::command]
pub async fn rename_batches(limit: u32) -> IpcResult<Vec<Batch>> {
    join_task(
        "读改名批次",
        tauri::async_runtime::spawn_blocking(move || {
            in_db(|conn| crate::rename::batch::list(conn, i64::from(limit.clamp(1, 50))))
        })
        .await,
    )
}

/// 拿一把串行化的连接跑一段「错误文案是给用户看的中文」的逻辑。
/// 改名会连着写盘与写库，锁着跑完可以排除两批交错在同一本账上的可能。
fn in_db<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> IpcResult<T> {
    crate::db::with_db_text(&crate::paths::library_db_path(), f).map_err(IpcError::db)
}

//! 译名那本**账**：`rename_name_cache`（过了九道校验的译名）与 `rejected_suggestions`（拒过的）。
//!
//! 与 [`ai_ask`](super::ai_ask) 的分界：那边只管「怎么把一个片段问成一个名字」，这一格只管
//! 「问出来的东西存哪、谁能取出来」。落地只认这一格 —— `apply` 要一个名字，唯一的来路是
//! [`suggestion`]，而它只回过完校验的那一行（`04 §7.7`）。
//!
//! 三条不外溢的口径：
//! 1. 入库只有 [`record`] 一处，且只写 `dst` 有值的那些：把 `Review` 的也写进去，等于给
//!    一个没核准的名字发了通行证。
//! 2. 键是**内容**键（`blake3(提示词版本 + 片段)`，`04 §7.6.2`）：换模型不作废，涨提示词版本
//!    整体作废，所以同一批名字再生成一次一个 token 都不再花。
//! 3. 「拒过」是**生效**的：[`suggestion`] 每取一次都复核一遍拒绝集，于是用户按了「不要这个
//!    译名」之后，预览（[`annotate`]）与落地（`apply`）同时不再提它。

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::rename::ai_ask::{Fragment, Outcome};
use crate::rename::ai_name;
use crate::rename::batch::now_ms;
use crate::rename::preview::{Item, Preview};

/// 一次请求的上限。这条命令没有任务与事件通道（与字幕链路的差别），量大了界面上只剩一颗
/// 转圈的按钮，所以宁可让用户把目录选小一点。
pub const MAX_NAMES: usize = 200;

/// 送给模型之前先要算清的事：多少条要翻、多少条库里有现成的、剩下的去重之后是哪些片段。
#[derive(Debug, Default)]
pub struct Plan {
    pub needed: usize,
    pub cached: usize,
    pub missing: Vec<Fragment>,
}

/// 规划：重算预览（与 `rename_preview`、`rename_apply` 用的是同一份纯函数），挑出要问模型的
/// 条目，命中缓存的直接算数。片段按「键 + 拒绝集」去重 —— 同目录几十条常是同一个词根。
pub fn plan(conn: &Connection, roots: &[String]) -> Result<Plan, String> {
    let preview = crate::rename::listing::preview_roots(roots);
    if preview.truncated {
        return Err(format!(
            "本批候选超过 {} 条预览上限，请减少选择的目录后分批处理",
            crate::rename::listing::MAX_ITEMS
        ));
    }
    let items: Vec<&Item> = preview.items.iter().filter(|i| i.needs_ai).collect();
    if items.len() > MAX_NAMES {
        return Err(format!("一次最多生成 {} 个译名，当前 {} 条，请减少选择的目录后分批", MAX_NAMES, items.len()));
    }
    let have = load_names(conn)?;
    let mut out = Plan::default();
    for it in items {
        let src = ai_name::fragment(it);
        if src.is_empty() {
            continue;
        }
        out.needed += 1;
        let key = ai_name::cache_key(&src);
        if have.contains_key(&key) {
            out.cached += 1;
            continue;
        }
        let refused = refused(conn, &it.path)?;
        match out.missing.iter_mut().find(|f| f.key == key && f.refused == refused) {
            // 同一片段落在不同条目上预算不同：取更严的那个，宁可少两个字也别写出放不下的名
            Some(known) => known.budget = known.budget.min(ai_name::body_budget(it)),
            None => out.missing.push(Fragment { key, src, refused, budget: ai_name::body_budget(it) }),
        }
    }
    Ok(out)
}

/// 这条名字在**这个条目**上被用户拒过哪些候选（`04 §7.7` 第 9 条的数据源）。
/// 键是条目的当前路径 —— 主干各处都以它为身份（`rename_items.old_path` 同一条），
/// 列名叫 `target_path` 是建表时的措辞，指的是「这一条」而不是「改完之后」。
/// 写它的那一侧是 [`reject`]（界面上「不要这个译名」），所以「拒过」是真能让一个已核准的
/// 名字失效的，不只是记一笔流水。
pub fn refused(conn: &Connection, path: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT suggestion FROM rejected_suggestions WHERE target_path = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![path], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 落地侧唯一的入口：按片段查已核准的译名，回（整名，当初用过的模型名 —— 它是 `04 §7.8`
/// 要的审计字段）。查不到就是没有：`apply` 因此把这条继续算进 `skipped`。
pub fn suggestion(conn: &Connection, item: &Item) -> Option<(String, Option<String>)> {
    let src = ai_name::fragment(item);
    if src.is_empty() {
        return None;
    }
    let (dst, model) = name_of(conn, &ai_name::cache_key(&src)).ok()??;
    let full = ai_name::rebuild(item, &dst);
    // 第 9 条在这里**再**判一次：库里那行是生成当时核准的，用户可能后来把这个名字拒了；
    // 而「拒绝集」读不出来时也不猜 —— 宁可不改，也不要改出一个刚被拒掉的名字。
    if refused(conn, &item.path).ok()?.iter().any(|r| r == &full) {
        return None;
    }
    Some((full, model))
}

/// 读库之后给预览**盖章**：每条 `needs_ai` 的条目带上「库里已核准的那个译名」。
/// [`suggestion`] 是落地侧的入口，这一条是**显示**侧的入口 —— 同一份判据，两处不会说两种话。
/// 与当前同名的不算：那种条目 `apply` 也不会动它，标出来只会多出一个假的「可应用」。
pub fn annotate(conn: &Connection, p: &mut Preview) {
    for it in &mut p.items {
        it.suggested = match it.needs_ai {
            true => suggestion(conn, it).map(|(n, _)| n).filter(|n| *n != it.name),
            false => None,
        };
    }
}

/// 记一条「这个译名我不要」：写进 `rejected_suggestions`，[`refused`] 立刻读得到，于是预览与
/// 落地**同时**不再用它 —— 拒绝是真生效，不是界面上藏起来。
/// 重复拒绝同一条不算失败（主键 `(target_path, suggestion)`，配 `OR IGNORE`）。
pub fn reject(conn: &Connection, path: &str, suggestion: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO rejected_suggestions (target_path, suggestion, rejected_at) VALUES (?1, ?2, ?3)",
        params![path, suggestion, now_ms()],
    )
    .map_err(|e| format!("记不下这条拒绝：{e}"))?;
    Ok(())
}

fn name_of(conn: &Connection, key: &str) -> Result<Option<(String, Option<String>)>, String> {
    let mut stmt = conn
        .prepare("SELECT dst, model FROM rename_name_cache WHERE cache_key = ?1")
        .map_err(|e| e.to_string())?;
    let first = stmt
        .query_map(params![key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
        .map_err(|e| e.to_string())?
        .next();
    first.transpose().map_err(|e| e.to_string())
}

fn load_names(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn.prepare("SELECT cache_key, dst FROM rename_name_cache").map_err(|e| e.to_string())?;
    let rows =
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    rows.collect::<Result<HashMap<_, _>, _>>().map_err(|e| e.to_string())
}

/// 把过完校验的结果写进库。只写有译文的那些 —— 键是内容键，所以同一批名字问第二次是覆盖
/// 而不是撞主键（换了模型也还是那一行，`model` 跟着换成最后一次的）。
pub fn record(conn: &Connection, model: &str, outcomes: &[Outcome]) -> Result<usize, String> {
    let mut n = 0usize;
    for o in outcomes.iter().filter(|o| o.dst.is_some()) {
        let notes = if o.notes.is_empty() { None } else { Some(o.notes.join("；")) };
        conn.execute(
            "INSERT INTO rename_name_cache (cache_key, src, dst, notes, model, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(cache_key) DO UPDATE SET src=excluded.src, dst=excluded.dst, \
             notes=excluded.notes, model=excluded.model, created_at=excluded.created_at",
            params![o.key.as_str(), o.src.as_str(), o.dst.as_deref(), notes, model, now_ms()],
        )
        .map_err(|e| format!("写译名缓存失败：{e}"))?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::{annotate, plan, record, reject, suggestion, Plan};
    use crate::rename::ai_name::cache_key;
    use crate::rename::judge::Verdict;
    use crate::rename::preview::{Item, Kind, Preview};
    use crate::rename::ai_ask::Outcome;
    use rusqlite::Connection;

    /// 表由**真迁移**建，不是手抄的 DDL —— 忘了往 `MIGRATIONS` 里加 006 那一行，这里就该红
    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::migrations::apply(&conn).expect("建表");
        conn
    }

    fn item(name: &str) -> Item {
        Item {
            path: format!("T:/x/{name}"),
            name: name.into(),
            target: name.into(),
            kind: Kind::Folder,
            verdict: Verdict::NeedsTranslation,
            reasons: vec![],
            changed: true,
            needs_ai: true,
            suggested: None,
            conflict: None,
        }
    }

    fn outcome(src: &str, dst: Option<&str>) -> Outcome {
        Outcome {
            key: cache_key(src),
            src: src.into(),
            dst: dst.map(Into::into),
            notes: vec![],
            blocked: vec![],
        }
    }

    /// 通行证只发给过了校验的那些：`dst = None`（含 `Review`/`Rejected`/请求失败）不入库，
    /// `apply` 因此一个字都不会替它改 —— 这一条就是「过不了校验就不写盘」的数据侧保证。
    #[test]
    fn only_checked_names_reach_the_ledger() {
        let conn = db();
        let outcomes = vec![outcome("静夜", Some("安宁")), outcome("深宵", None)];
        assert_eq!(record(&conn, "qwen", &outcomes).expect("写库"), 1);
        let got = suggestion(&conn, &item("静夜")).expect("过校验的那条该查得到");
        assert_eq!((got.0.as_str(), got.1.as_deref()), ("安宁", Some("qwen")));
        assert!(suggestion(&conn, &item("深宵")).is_none(), "没过校验的那条不许变成通行证");
    }

    /// 同一原文第二次生成要覆盖而不是撞主键（键是内容键，换模型也还是同一行）
    #[test]
    fn record_is_upsert_by_content_key() {
        let conn = db();
        record(&conn, "m1", &[outcome("丙", Some("丙一"))]).expect("首次");
        record(&conn, "m2", &[outcome("丙", Some("丙二"))]).expect("再来一次");
        let got = suggestion(&conn, &item("丙")).expect("查得到");
        assert_eq!((got.0.as_str(), got.1.as_deref()), ("丙二", Some("m2")));
    }

    /// 预览上那个「已核准」的标记与落地用的是同一份判据：库里没有、与原名同名、被拒过，
    /// 三种都不许标 —— 标了就是给用户一个「可应用」，而他按下之后什么都不会发生。
    #[test]
    fn annotate_marks_only_names_that_would_actually_be_applied() {
        let conn = db();
        let mut p = Preview { items: vec![item("静夜")], ..Default::default() };
        annotate(&conn, &mut p);
        assert_eq!(p.items[0].suggested, None, "库里什么都没标");

        record(&conn, "m", &[outcome("静夜", Some("静夜"))]).expect("写库");
        annotate(&conn, &mut p);
        assert_eq!(p.items[0].suggested, None, "译名与当前同名 = 这一条不会被动");

        record(&conn, "m", &[outcome("静夜", Some("安宁"))]).expect("换一个");
        annotate(&conn, &mut p);
        assert_eq!(p.items[0].suggested.as_deref(), Some("安宁"));

        reject(&conn, &p.items[0].path, "安宁").expect("记一条拒绝");
        annotate(&conn, &mut p);
        assert_eq!(p.items[0].suggested, None, "拒过的名字不许再出现在预览里");
        assert!(suggestion(&conn, &item("静夜")).is_none(), "落地侧也不同时再用它");
    }

    /// 规划只把**没命中缓存**的片段交出去，并且算清哪几条本来就有现成的（`04 §7.6.2` 省 token 的根据）。
    /// 目录不存在时预览是空的 —— 这条用例因此不需要碰盘。
    #[test]
    fn plan_asks_only_for_the_missing_fragments() {
        let conn = db();
        let Plan { needed, cached, .. } = plan(&conn, &["T:/不存在/的目录".into()]).expect("规划");
        assert_eq!((needed, cached), (0, 0), "没有目录就一条都不用问");
    }
}

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
//! 3. 「拒过」是**生效**的：[`ledger_state`] 每取一次都复核一遍拒绝集，于是用户按了「不要这个
//!    译名」之后，预览（[`annotate`]）与落地（`apply`）同时不再提它。
//! 4. 「拒过」是**名字级**的：`rejected_suggestions` 的主键带着 `suggestion`，拒掉的是那个候选名，
//!    不是这个路径。所以判「这条会不会被卡住」必须问到它**当下那个候选名**上（[`blocked_by_refusal`]），
//!    按路径一刀切会把「拒过一次」误算成「这条永久作废」—— 换个新译名本该照样能改。

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
/// `only` 非空时只清点**精确命中这些条目路径**的行（文件夹名翻译的「只生成选中的」）：
/// 预览仍按整根列一遍 —— 路径必须能在这一层里对上号才算数，账外的路径静默落空，
/// 不存在「前端指哪删哪」的口子。`needed` / `cached` / `missing` 一起收窄，
/// 报告的口径与实际问出去的请求一致。
pub fn plan_scoped(conn: &Connection, roots: &[String], only: &[String]) -> Result<Plan, String> {
    let preview = crate::rename::listing::preview_roots(roots);
    if preview.truncated {
        return Err(format!(
            "本批候选超过 {} 条预览上限，请减少选择的目录后分批处理",
            crate::rename::listing::MAX_ITEMS
        ));
    }
    let items: Vec<&Item> = preview
        .items
        .iter()
        .filter(|i| i.needs_ai)
        .filter(|i| only.is_empty() || only.iter().any(|p| p == &i.path))
        .collect();
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

/// 一个条目在译名账里的三种处境。`Missing` 与 `Blocked` 要分开：前者是「这个名字还没问过」，
/// 点一次「生成译名」就可能有救；后者是「问到了，但这个名字被用户拒了」，再问多少遍都是同一行，
/// 得先撤掉那条拒绝，或者手工改名（`04 §7.7` 第 9 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ledger {
    /// 库里有一个可用的整名（已经过完九道校验、且没被这个条目拒过）
    Ready { name: String, model: Option<String> },
    /// 库里有名字，但那正是这个条目拒过的那个
    Blocked,
    /// 片段为空、或库里还没有这个名字
    Missing,
}

/// 按条目**当下的片段**查一次账，再看这个名字在**这个条目**上被拒过没有 —— 第 9 条在这里
/// 复核一遍：库里那行是生成当时核准的，用户可能后来才把它拒了。
/// [`suggestion`]（落地要一个名字）与 [`blocked_by_refusal`]（预览要「是不是被拒卡住」）都从
/// 这里出发 —— 两边问的是同一件事的两种问法，不该有各说各话的余地。
pub fn ledger_state(conn: &Connection, item: &Item) -> Result<Ledger, String> {
    let src = ai_name::fragment(item);
    if src.is_empty() {
        return Ok(Ledger::Missing);
    }
    let Some((dst, model)) = name_of(conn, &ai_name::cache_key(&src))? else {
        return Ok(Ledger::Missing);
    };
    let full = ai_name::rebuild(item, &dst);
    if refused(conn, &item.path)?.iter().any(|r| r == &full) {
        return Ok(Ledger::Blocked);
    }
    Ok(Ledger::Ready { name: full, model })
}

/// 判定树那一侧（预览）用的判据：这一条会不会**只因为名字被拒**而卡住（`04 §7.2` 第 1 条）。
/// 与 [`suggestion`] 是同一条判据：那边答「有没有名字可用」，这边答「没有是不是因为被拒」。
///
/// 只对 `needs_ai` 的条目成立 —— 本地规则能定下来的动作根本不看译名账，拒过的候选也拦不着它，
/// 这与落地侧「`!needs_ai` 就直接用自己的目标名」是同一条分界。
/// 读库出错时答「没被卡住」：那样预览显示成待生成（落地那时也确实不会动它），而不是把它错记成
/// 跳过、让人以为这件事已经有人办过了。
pub fn blocked_by_refusal(conn: &Connection, item: &Item) -> bool {
    item.needs_ai && matches!(ledger_state(conn, item), Ok(Ledger::Blocked))
}

/// 落地侧唯一的入口：按片段查已核准的译名，回（整名，当初用过的模型名 —— 它是 `04 §7.8`
/// 要的审计字段）。查不到就是没有：`apply` 因此把这条继续算进 `skipped`。
pub fn suggestion(conn: &Connection, item: &Item) -> Option<(String, Option<String>)> {
    match ledger_state(conn, item) {
        Ok(Ledger::Ready { name, model }) => Some((name, model)),
        // `Missing` 本来就是没有；`Blocked` 是被拒；`Err` 是拒绝集读不出来 —— 读不出来也不猜，
        // 宁可不改，也不要改出一个刚被拒掉的名字。
        _ => None,
    }
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
    use super::{annotate, blocked_by_refusal, ledger_state, plan_scoped, record, reject, suggestion, Ledger, Plan};
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

    /// 同名的另一条（片段相同、路径不同）：验证「拒过」是挂在条目上而不是挂在片段上
    fn item_at(path: &str) -> Item {
        let mut it = item(path.rsplit('/').next().unwrap_or(path));
        it.path = path.to_string();
        it
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

    /// 拒绝是**名字级**的：拒掉「安宁」只让这一个条目上的这一个候选失效，同片段的另一个条目
    /// 照样能用它；同一个条目换个新译名也不再被它卡住。
    /// 这是「预览说的 == 落地做的」的根：判定树收到的 `rejected` 必须由 [blocked_by_refusal] 回答，
    /// 而不是「这个路径进过拒绝表」。
    #[test]
    fn a_refusal_only_blocks_the_name_it_was_aimed_at_on_that_one_item() {
        let conn = db();
        let here = item("静夜");
        let elsewhere = item_at("T:/y/静夜");
        assert_eq!(ledger_state(&conn, &here).expect("查账"), Ledger::Missing, "还没问过 = 只是缺");
        assert!(!blocked_by_refusal(&conn, &here), "缺名字不是被拒，点「生成译名」该有救");

        record(&conn, "m", &[outcome("静夜", Some("安宁"))]).expect("写库");
        reject(&conn, &here.path, "安宁").expect("记一条拒绝");
        assert_eq!(ledger_state(&conn, &here).expect("查账"), Ledger::Blocked);
        assert!(blocked_by_refusal(&conn, &here), "拒过的名字 = 预览该判跳过");
        assert!(suggestion(&conn, &here).is_none(), "落地侧同一条判据，不然就是各说各话");
        assert!(!blocked_by_refusal(&conn, &elsewhere), "另一个条目没拒过，不该跟着一起停");
        assert!(suggestion(&conn, &elsewhere).is_some());

        record(&conn, "m", &[outcome("静夜", Some("静谧"))]).expect("换个新译名");
        assert!(!blocked_by_refusal(&conn, &here), "拒的是旧名字，新名字不该被连坐");
        assert_eq!(
            suggestion(&conn, &here).map(|(n, _)| n).as_deref(),
            Some("静谧"),
            "按路径一刀切的话，这条就永久停在跳过了"
        );
    }

    /// 本地规则能定下来的条目不看译名账：落地那时用的是它自己的目标名，拒过的候选拦不着它
    #[test]
    fn a_settled_item_is_never_blocked_by_a_refusal() {
        let conn = db();
        let local = Item { needs_ai: false, ..item("静夜") };
        record(&conn, "m", &[outcome("静夜", Some("安宁"))]).expect("写库");
        reject(&conn, &local.path, "安宁").expect("记一条拒绝");
        assert!(!blocked_by_refusal(&conn, &local));
    }

    /// 规划只把**没命中缓存**的片段交出去，并且算清哪几条本来就有现成的（`04 §7.6.2` 省 token 的根据）。
    /// 目录不存在时预览是空的 —— 这条用例因此不需要碰盘。
    #[test]
    fn plan_asks_only_for_the_missing_fragments() {
        let conn = db();
        let Plan { needed, cached, .. } = plan_scoped(&conn, &["T:/不存在/的目录".into()], &[]).expect("规划");
        assert_eq!((needed, cached), (0, 0), "没有目录就一条都不用问");
    }
}

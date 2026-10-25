//! 已翻译文件的管理（AI 工作台「译文管理」页的事实源）。
//!
//! 翻译账（`ai_translate_cache`，kind='subtitle'）只记录「当时翻过什么」；这里逐条核对
//! 文件系统现状，把「现在还成立吗」算成四种状态交给界面，每种状态各给对应的处置：
//! - `ok`：源、译文都在且源没改过 → 打开 / 删除译文；
//! - `changed`：源还在但指纹（mtime/size）变了，译文已过时 → 重译 / 打开 / 删除；
//! - `out_missing`：源在、译文文件没了 → 重译 / 清账；
//! - `source_gone`：源被删了（译文可能还在，对歌词来说译文往往才是留下的那份）→ 清账 / 打开 / 删除。
//!
//! 删除永远以**账里记录的 out_path** 为准：命令只收源路径，译文路径不信任调用方传入。

use crate::ai_cache;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedEntry {
    pub source_path: String,
    pub source_name: String,
    pub out_path: String,
    /// 译文文件现在还在盘上吗（「打开译文」按钮的可用性跟着它走）
    pub out_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_path: Option<String>,
    /// 翻译完成时刻（epoch ms）
    pub translated_at: i64,
    /// ok | changed | out_missing | source_gone
    pub status: &'static str,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveReport {
    pub removed: usize,
    /// 逐条失败的原因（形状「路径：原因」），整体照常返回
    pub errors: Vec<String>,
}

/// 账里一行 → 现状分类。指纹判定与 `ai_cache::subtitle_exempt` 同一条口径（mtime 近似相等 + size 相等）。
fn classify(source: &str, out_path: &str, mtime: Option<f64>, size: Option<i64>) -> &'static str {
    let src = ai_cache::stat(source);
    if src.is_none() {
        return "source_gone";
    }
    if !Path::new(out_path).is_file() {
        return "out_missing";
    }
    let st = src.unwrap();
    let match_fingerprint = matches!((mtime, size), (Some(m), Some(s)) if ai_cache::mtime_close(m, st.mtime) && s == st.size as i64);
    if match_fingerprint {
        "ok"
    } else {
        "changed"
    }
}

/// 全部已翻译条目（含现状），按翻译时间倒序 —— 最近翻过的在最上面。
pub fn list(conn: &Connection) -> Vec<TranslatedEntry> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT source_path, out_path, audio_path, file_mtime, file_size, translated_at \
         FROM ai_translate_cache WHERE kind = 'subtitle' ORDER BY translated_at DESC",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<f64>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, i64>(5)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .map(|(source_path, out_path, audio_path, mtime, size, translated_at)| {
            let status = classify(&source_path, &out_path, mtime, size);
            let out_exists = Path::new(&out_path).is_file();
            let source_name = source_path.rsplit(['/', '\\']).next().unwrap_or(&source_path).to_string();
            TranslatedEntry { source_path, source_name, out_path, out_exists, audio_path, translated_at, status }
        })
        .collect()
}

/// 翻译账里记录过的**目录**（`source_path` 与 `out_path` 的去重父目录，不存在的不算）。
///
/// 这是 `PathScope` 的第四条授权来路（P0-1）：账是应用自己写入的，目录是用户在**某次
/// 会话里用对话框授权过的**——但 `PathScope` 是内存态，重启即失忆，于是译文管理页
/// 「打开译文」被闸门静默拒掉、重译整批拒（前端 fire-and-forget 连错误都看不见）。
/// 把账里的目录在启动时记回去，与 `folderPaths`（库里登记的根）同一个信任等级；
/// 不存在与读不出来的目录自然漏掉，`allow_root` 那边也会再 canonicalize 一次。
pub fn ledger_dirs(conn: &Connection) -> Vec<PathBuf> {
    let Ok(mut stmt) = conn
        .prepare("SELECT source_path, out_path FROM ai_translate_cache WHERE kind = 'subtitle'")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))) else {
        return Vec::new();
    };
    let mut dirs = std::collections::BTreeSet::new();
    for (source, out) in rows.flatten() {
        for p in [Some(source), out] {
            if let Some(parent) = p.as_deref().map(Path::new).and_then(Path::parent) {
                if parent.is_dir() {
                    dirs.insert(parent.to_path_buf());
                }
            }
        }
    }
    dirs.into_iter().collect()
}

/// 清理失效账：源、译文都不在的行是纯垃圾（留着只会让列表虚胖），一键删掉。
/// 源没了但译文还在的不动 —— 那份译文是用户的劳动成果，去留由人决定。
pub fn cleanup_gone(conn: &Connection) -> usize {
    let mut removed = 0usize;
    for entry in list(conn) {
        let pure_garbage = entry.status == "source_gone" && !Path::new(&entry.out_path).is_file();
        if pure_garbage
            && conn
                .execute("DELETE FROM ai_translate_cache WHERE kind='subtitle' AND source_path = ?1", [&entry.source_path])
                .is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// 删译文（`remove_file` = 连 `.zh` 文件一起删，路径以账里记录的为准）或只清账。
/// 返回实际处理的条数；逐条失败（删文件失败等）进 `errors`，不挡其余条目。
pub fn remove(conn: &Connection, sources: &[String], remove_file: bool) -> RemoveReport {
    let mut report = RemoveReport { removed: 0, errors: Vec::new() };
    for source in sources {
        let row = conn
            .query_row(
                "SELECT out_path FROM ai_translate_cache WHERE kind='subtitle' AND source_path = ?1",
                [source],
                |r| r.get::<_, String>(0),
            )
            .row_or_report(&mut report, source);
        let Some(out_path) = row else { continue };
        if remove_file && Path::new(&out_path).is_file() {
            if let Err(e) = std::fs::remove_file(&out_path) {
                report.errors.push(format!("{source}：删除译文失败 {e}"));
                continue;
            }
        }
        match conn.execute("DELETE FROM ai_translate_cache WHERE kind='subtitle' AND source_path = ?1", [source]) {
            Ok(1) => report.removed += 1,
            _ => report.errors.push(format!("{source}：清账失败")),
        }
    }
    report
}

/// 删**原文**：只删账里记录的那个源文件 —— 前端传什么就删什么是不存在的口子，
/// 路径必须精确命中一行账。译文与账都保留：条目转成「源已删」（`source_gone`），
/// 译文的去留另有「删除译文 / 清账」两个按钮管，不在这里替用户做决定。
pub fn remove_source(conn: &Connection, sources: &[String]) -> RemoveReport {
    let mut report = RemoveReport { removed: 0, errors: Vec::new() };
    for source in sources {
        let recorded = conn
            .query_row(
                "SELECT 1 FROM ai_translate_cache WHERE kind='subtitle' AND source_path = ?1",
                [source],
                |r| r.get::<_, i64>(0),
            )
            .row_or_report(&mut report, source);
        if recorded.is_none() {
            continue;
        }
        match std::fs::remove_file(source) {
            Ok(()) => report.removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.errors.push(format!("{source}：源文件已不在"));
            }
            Err(e) => report.errors.push(format!("{source}：删除原文失败 {e}")),
        }
    }
    report
}

/// query_row 的「没有这行不算错误」包装：查无此账静默跳过，真错误进报告。
/// 对行值的类型泛型（`remove` 取 out_path 的 String、`remove_source` 探账的 i64 共用一条口径）。
trait RowOrReport<T> {
    fn row_or_report(self, report: &mut RemoveReport, source: &str) -> Option<T>;
}
impl<T> RowOrReport<T> for Result<T, rusqlite::Error> {
    fn row_or_report(self, report: &mut RemoveReport, source: &str) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => {
                report.errors.push(format!("{source}：读账失败 {e}"));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::initialize_schema;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        initialize_schema(&conn).expect("建表");
        conn
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("kx-translated-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("临时目录");
        p
    }

    fn record(conn: &Connection, source: &str, out: &str) {
        let st = ai_cache::stat(source).expect("源存在");
        ai_cache::record_subtitle(conn, source, out, &st, "hash", None).unwrap();
    }

    #[test]
    fn statuses_follow_the_filesystem() {
        let dir = tmpdir("status");
        let conn = db();
        // ok：源、译文都在且指纹没变
        std::fs::write(dir.join("a.lrc"), "x").unwrap();
        let a_out = dir.join("a.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("a.lrc").to_str().unwrap(), &a_out);
        std::fs::write(&a_out, "译文").unwrap();
        // changed：翻完之后源被改过
        std::fs::write(dir.join("b.lrc"), "x").unwrap();
        let b_out = dir.join("b.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("b.lrc").to_str().unwrap(), &b_out);
        std::fs::write(&b_out, "译文").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("b.lrc"), "x2").unwrap();
        // out_missing：译文被手动删了
        std::fs::write(dir.join("c.lrc"), "x").unwrap();
        let c_out = dir.join("c.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("c.lrc").to_str().unwrap(), &c_out);
        std::fs::write(&c_out, "译文").unwrap();
        let _ = std::fs::remove_file(&c_out);
        // source_gone：源被删了，译文还在
        std::fs::write(dir.join("d.lrc"), "x").unwrap();
        let d_out = dir.join("d.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("d.lrc").to_str().unwrap(), &d_out);
        std::fs::write(&d_out, "译文").unwrap();
        let _ = std::fs::remove_file(dir.join("d.lrc"));

        let by_name = |name: &str| {
            list(&conn)
                .into_iter()
                .find(|e| e.source_name == name)
                .unwrap_or_else(|| panic!("{name} 应在列表里"))
        };
        assert_eq!(by_name("a.lrc").status, "ok");
        assert_eq!(by_name("b.lrc").status, "changed");
        assert_eq!(by_name("c.lrc").status, "out_missing");
        assert_eq!(by_name("d.lrc").status, "source_gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_only_removes_rows_with_neither_source_nor_output() {
        let dir = tmpdir("cleanup");
        let conn = db();
        // 纯垃圾：源和译文都没了
        std::fs::write(dir.join("gone.lrc"), "x").unwrap();
        let gone_out = dir.join("gone.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("gone.lrc").to_str().unwrap(), &gone_out);
        let _ = std::fs::remove_file(&gone_out);
        let _ = std::fs::remove_file(dir.join("gone.lrc"));
        // 源没了但译文还在：用户的劳动成果，不自动清
        std::fs::write(dir.join("keep.lrc"), "x").unwrap();
        let keep_out = dir.join("keep.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("keep.lrc").to_str().unwrap(), &keep_out);
        std::fs::write(&keep_out, "译文").unwrap();
        let _ = std::fs::remove_file(dir.join("keep.lrc"));

        assert_eq!(cleanup_gone(&conn), 1);
        let names: Vec<String> = list(&conn).into_iter().map(|e| e.source_name).collect();
        assert_eq!(names, vec!["keep.lrc"], "只剩源没译文在的那条");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_deletes_the_recorded_output_and_the_row() {
        let dir = tmpdir("remove");
        let conn = db();
        std::fs::write(dir.join("a.lrc"), "x").unwrap();
        let a_out = dir.join("a.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("a.lrc").to_str().unwrap(), &a_out);
        std::fs::write(&a_out, "译文").unwrap();

        // 只清账：译文文件留下
        let r = remove(&conn, &[dir.join("a.lrc").to_str().unwrap().to_string()], false);
        assert_eq!((r.removed, r.errors.len()), (1, 0));
        assert!(Path::new(&a_out).is_file(), "清账不动译文文件");
        assert!(list(&conn).is_empty());

        // 重新记账后删译文：文件与账一起消失
        record(&conn, dir.join("a.lrc").to_str().unwrap(), &a_out);
        let r = remove(&conn, &[dir.join("a.lrc").to_str().unwrap().to_string()], true);
        assert_eq!((r.removed, r.errors.len()), (1, 0));
        assert!(!Path::new(&a_out).is_file(), "删译文要真删文件");

        // 账里没有的路径静默跳过，不报错也不误删
        let r = remove(&conn, &["/nowhere/x.lrc".to_string()], true);
        assert_eq!((r.removed, r.errors.len()), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 删原文只删账里记录的那个源文件，译文与账都留着 —— 条目转为「源已删」，
    /// 译文的去留由别的按钮管；账外路径一个字节都不许碰。
    #[test]
    fn remove_source_deletes_only_the_ledgered_source_file() {
        let dir = tmpdir("remove-source");
        let conn = db();
        std::fs::write(dir.join("a.lrc"), "原文").unwrap();
        let a_out = dir.join("a.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("a.lrc").to_str().unwrap(), &a_out);
        std::fs::write(&a_out, "译文").unwrap();

        // 账外路径不删（哪怕它真实存在）
        std::fs::write(dir.join("outsider.lrc"), "别删我").unwrap();
        let r = remove_source(&conn, &[dir.join("outsider.lrc").to_str().unwrap().to_string()]);
        assert_eq!((r.removed, r.errors.len()), (0, 0));
        assert!(Path::new(&dir.join("outsider.lrc")).is_file(), "账外文件必须原样留着");

        // 账内路径：删的是源文件，译文与账都还在
        let r = remove_source(&conn, &[dir.join("a.lrc").to_str().unwrap().to_string()]);
        assert_eq!((r.removed, r.errors.len()), (1, 0));
        assert!(!Path::new(&dir.join("a.lrc")).is_file(), "原文要真删");
        assert!(Path::new(&a_out).is_file(), "译文不动");
        assert_eq!(list(&conn)[0].status, "source_gone", "条目转为源已删，译文的处置入口还在");

        // 源已经没了的条目再删一次：报错说明，不静默装成功
        let r = remove_source(&conn, &[dir.join("a.lrc").to_str().unwrap().to_string()]);
        assert_eq!(r.removed, 0);
        assert!(r.errors[0].contains("已不在"), "{}", r.errors[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 授权来路的第四条（PathScope 重启失忆的解药）：账里记录的目录要去重、
    /// 只算真实存在的；源没了（source_gone）的条目靠 out_path 的父目录也能认回来。
    #[test]
    fn ledger_dirs_lists_distinct_existing_parents() {
        let dir = tmpdir("ledger-dirs");
        let other = tmpdir("ledger-dirs-out");
        let conn = db();

        // 正常条目：源与译文在同一目录 → 只记一个
        std::fs::write(dir.join("a.lrc"), "x").unwrap();
        let a_out = dir.join("a.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("a.lrc").to_str().unwrap(), &a_out);
        std::fs::write(&a_out, "译文").unwrap();

        // 译文写在另一个目录（异常但合法）→ 两个都记
        std::fs::write(dir.join("b.lrc"), "x").unwrap();
        let b_out = other.join("b.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("b.lrc").to_str().unwrap(), &b_out);
        std::fs::write(&b_out, "译文").unwrap();

        // source_gone：源没了，靠 out_path 的父目录照样能认回来
        std::fs::write(dir.join("c.lrc"), "x").unwrap();
        let c_out = dir.join("c.zh.lrc").to_str().unwrap().to_string();
        record(&conn, dir.join("c.lrc").to_str().unwrap(), &c_out);
        std::fs::write(&c_out, "译文").unwrap();
        let _ = std::fs::remove_file(dir.join("c.lrc"));

        let dirs = ledger_dirs(&conn);
        assert!(dirs.contains(&dir), "源与译文的共同父目录必须在");
        assert!(dirs.contains(&other), "译文独占的父目录也必须");
        assert_eq!(dirs.len(), 2, "去重后只有这两个目录：{dirs:?}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }
}

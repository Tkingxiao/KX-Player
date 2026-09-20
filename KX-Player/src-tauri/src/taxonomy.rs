//! 分类树 + 标签 + 自动打标（P0-11/12/13）。
//! 自动打标只产生建议，绝不直接写库（宪法禁止项 24）。

use crate::db::with_db;
use crate::paths::library_db_path;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

// ── DTO ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub sort_index: i64,
    pub track_count: i64,
    pub children: Vec<CategoryDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub kind: String,
    pub use_count: i64,
    pub auto_generated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSuggestionDto {
    pub name: String,
    pub score: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ── 分类树 ──────────────────────────────────────────────────────

pub fn list_categories() -> Vec<CategoryDto> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.parent_id, c.name, c.icon, c.color, c.sort_index,
                    (SELECT COUNT(*) FROM category_items ci WHERE ci.category_id = c.id) AS track_count
             FROM categories c ORDER BY c.sort_index, c.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;
        let mut flat: Vec<CategoryDto> = Vec::new();
        for row in rows {
            let (id, parent_id, name, icon, color, sort_index, track_count) = row?;
            flat.push(CategoryDto { id, parent_id, name, icon, color, sort_index, track_count, children: vec![] });
        }
        Ok(flat)
    })
    .unwrap_or_default()
}

/// 由扁平列表组树（parent 在后者挂接；孤儿当根）
pub fn build_category_tree(mut flat: Vec<CategoryDto>) -> Vec<CategoryDto> {
    flat.sort_by_key(|c| c.id);
    let mut map: std::collections::HashMap<i64, CategoryDto> = flat.into_iter().map(|c| (c.id, c)).collect();
    let mut roots: Vec<CategoryDto> = Vec::new();
    let ids: Vec<i64> = map.keys().cloned().collect();
    for id in ids {
        if let Some(node) = map.remove(&id) {
            match node.parent_id {
                Some(pid) if map.contains_key(&pid) => {
                    if let Some(p) = map.get_mut(&pid) {
                        p.children.push(node);
                    } else {
                        roots.push(node);
                    }
                }
                _ => roots.push(node),
            }
        }
    }
    fn sort_rec(nodes: &mut Vec<CategoryDto>) {
        nodes.sort_by(|a, b| (a.sort_index, a.id).cmp(&(b.sort_index, b.id)));
        for n in nodes {
            sort_rec(&mut n.children);
        }
    }
    sort_rec(&mut roots);
    roots
}

pub fn create_category(parent_id: Option<i64>, name: &str) -> Result<i64, String> {
    if name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO categories (parent_id, name, created_at) VALUES (?1, ?2, ?3)",
            params![parent_id, name.trim(), now_ms()],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .map_err(|e| e.to_string())
}

pub fn rename_category(id: i64, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    with_db(Path::new(&library_db_path()), |conn| {
        conn.execute("UPDATE categories SET name = ?1 WHERE id = ?2", params![name.trim(), id])?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

pub fn delete_category(id: i64) -> Result<(), String> {
    with_db(Path::new(&library_db_path()), |conn| {
        // 级联删除子分类与关联（ON DELETE CASCADE）
        conn.execute("DELETE FROM categories WHERE id = ?1", [id])?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

pub fn move_category(id: i64, parent_id: Option<i64>, sort_index: i64) -> Result<(), String> {
    if parent_id == Some(id) {
        return Err("不能移动到自己".into());
    }
    with_db(Path::new(&library_db_path()), |conn| {
        conn.execute(
            "UPDATE categories SET parent_id = ?1, sort_index = ?2 WHERE id = ?3",
            params![parent_id, sort_index, id],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

pub fn assign_category(category_id: i64, track_ids: &[String]) -> Result<i64, String> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut changed: i64 = 0;
        for tid in track_ids {
            let n = conn.execute(
                "INSERT OR IGNORE INTO category_items (category_id, track_id, added_at) VALUES (?1, ?2, ?3)",
                params![category_id, tid, now_ms()],
            )?;
            changed += n as i64;
        }
        Ok(changed)
    })
    .map_err(|e| e.to_string())
}

pub fn unassign_category(category_id: i64, track_ids: &[String]) -> Result<i64, String> {
    with_db(Path::new(&library_db_path()), |conn| {
        let mut changed: i64 = 0;
        for tid in track_ids {
            changed += conn
                .execute("DELETE FROM category_items WHERE category_id = ?1 AND track_id = ?2", params![category_id, tid])? as i64;
        }
        Ok(changed)
    })
    .map_err(|e| e.to_string())
}

pub fn category_track_ids(category_id: i64, include_children: bool) -> Vec<String> {
    let cat_ids = if include_children {
        let flat = list_categories();
        collect_descendants(&flat, category_id)
    } else {
        vec![category_id]
    };
    let mut out = Vec::new();
    for cid in cat_ids {
        let ids = with_db(Path::new(&library_db_path()), |conn| {
            let mut stmt = conn.prepare("SELECT track_id FROM category_items WHERE category_id = ?1")?;
            let rows = stmt.query_map([cid], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
        out.extend(ids);
    }
    out
}

fn collect_descendants(flat: &[CategoryDto], root: i64) -> Vec<i64> {
    let mut out = vec![root];
    let children_of = |pid: i64| -> Vec<i64> { flat.iter().filter(|c| c.parent_id == Some(pid)).map(|c| c.id).collect() };
    let mut i = 0;
    while i < out.len() {
        let kids = children_of(out[i]);
        out.extend(kids);
        i += 1;
    }
    out
}

// ── 标签 ────────────────────────────────────────────────────────

pub fn list_tags() -> Vec<TagDto> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.kind, t.use_count, t.auto_generated,
                    (SELECT COUNT(*) FROM track_tags tt WHERE tt.tag_id = t.id) AS use_count
             FROM tags t ORDER BY use_count DESC, t.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(TagDto {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                kind: r.get(3)?,
                use_count: r.get(6)?,
                auto_generated: r.get::<_, i64>(5).unwrap_or(0) == 1,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .unwrap_or_default()
}

pub fn upsert_tag(name: &str, color: Option<&str>, kind: &str) -> Result<i64, String> {
    if name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        conn.execute(
            "INSERT INTO tags (name, color, kind) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET color = COALESCE(excluded.color, tags.color)",
            params![name.trim(), color, kind],
        )?;
        Ok(conn.query_row("SELECT id FROM tags WHERE name = ?1", [name.trim()], |r| r.get(0))?)
    })
    .map_err(|e| e.to_string())
}

pub fn rename_tag(id: i64, name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("名称不能为空".into());
    }
    with_db(Path::new(&library_db_path()), |conn| {
        let changed = conn.execute("UPDATE tags SET name = ?1 WHERE id = ?2", params![trimmed, id])?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    })
    .map_err(|e| {
        if e.to_string().contains("UNIQUE constraint failed") {
            "已存在同名标签".to_string()
        } else if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
            "标签不存在".to_string()
        } else {
            e.to_string()
        }
    })
}

pub fn delete_tag(id: i64) -> Result<(), String> {
    with_db(Path::new(&library_db_path()), |conn| {
        conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 打标（add=追加；replace=先清空这些条目的手动标签）
pub fn tag_tracks(tag_ids: &[i64], track_ids: &[String], mode: &str) -> Result<i64, String> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut changed: i64 = 0;
        if mode == "replace" {
            for tid in track_ids {
                conn.execute("DELETE FROM track_tags WHERE track_id = ?1 AND source = 'manual'", [tid])?;
            }
        }
        for tag in tag_ids {
            for tid in track_ids {
                changed += conn
                    .execute("INSERT OR IGNORE INTO track_tags (track_id, tag_id, source) VALUES (?1, ?2, 'manual')", params![tid, tag])? as i64;
            }
        }
        // 维护 use_count
        conn.execute(
            "UPDATE tags SET use_count = (SELECT COUNT(*) FROM track_tags WHERE tag_id = tags.id)",
            [],
        )?;
        Ok(changed)
    })
    .map_err(|e| e.to_string())
}

pub fn untag_tracks(tag_id: i64, track_ids: &[String]) -> Result<i64, String> {
    with_db(Path::new(&library_db_path()), |conn| {
        let mut changed: i64 = 0;
        for tid in track_ids {
            changed += conn.execute("DELETE FROM track_tags WHERE tag_id = ?1 AND track_id = ?2", params![tag_id, tid])? as i64;
        }
        conn.execute("UPDATE tags SET use_count = (SELECT COUNT(*) FROM track_tags WHERE tag_id = tags.id)", [])?;
        Ok(changed)
    })
    .map_err(|e| e.to_string())
}

pub fn tags_of_tracks(track_ids: &[String]) -> std::collections::HashMap<String, Vec<TagDto>> {
    let mut out: std::collections::HashMap<String, Vec<TagDto>> = std::collections::HashMap::new();
    if track_ids.is_empty() {
        return out;
    }
    let all = list_tags();
    let tag_by_id: std::collections::HashMap<i64, TagDto> = all.into_iter().map(|t| (t.id, t)).collect();
    let _ = with_db(Path::new(&library_db_path()), |conn| {
        let mut stmt = conn.prepare("SELECT track_id, tag_id FROM track_tags")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (tid, tag_id) = row?;
            if track_ids.contains(&tid) {
                if let Some(tag) = tag_by_id.get(&tag_id) {
                    out.entry(tid).or_default().push(tag.clone());
                }
            }
        }
        Ok(())
    });
    out
}

/// 全量 track→tag 关联（筛选用）：(trackId, tagId) 对
pub fn all_track_tags() -> Vec<(String, i64)> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut stmt = conn.prepare("SELECT track_id, tag_id FROM track_tags")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .unwrap_or_default()
}

/// 全量分类条目关联：(categoryId, trackId) 对
pub fn all_category_items() -> Vec<(i64, String)> {
    with_db(Path::new(&library_db_path()), |conn| {
        crate::db::initialize_schema(conn)?;
        let mut stmt = conn.prepare("SELECT category_id, track_id FROM category_items")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .unwrap_or_default()
}

// ── 自动打标（只建议，不写库）──────────────────────────────────

/// ASMR 关键词表（中/日/英 → 中文标签名），P0-13
const KEYWORDS: &[(&str, &str)] = &[
    ("耳搔", "耳搔"), ("掏耳", "掏耳"), ("采耳", "掏耳"), ("低语", "低语"), ("耳语", "低语"),
    ("舔耳", "舔耳"), ("敲击", "敲击"), ("咀嚼", "咀嚼"), ("雨声", "雨声"), ("雨音", "雨声"),
    ("风声", "风声"), ("火声", "火声"), ("焚き火", "火声"), ("篝火", "火声"), ("白噪音", "白噪音"),
    ("心跳", "心跳"), ("角色扮演", "角色扮演"), ("朗读", "朗读"), ("双耳", "双耳录音"), ("binaural", "双耳录音"),
    ("睡眠", "睡眠"), ("助眠", "助眠"), ("添い寝", "助眠"), ("专注", "专注"),
    ("耳かき", "掏耳"), ("ささやき", "低语"), ("whisper", "低语"), ("吐息", "吐息"), ("ear cleaning", "掏耳"),
    ("tapping", "敲击"), ("rain", "雨声"), ("asmr", "ASMR"),
];

fn match_keywords(text: &str) -> Vec<TagSuggestionDto> {
    let lower = text.to_lowercase();
    let mut out: Vec<TagSuggestionDto> = Vec::new();
    for (kw, tag) in KEYWORDS {
        // 词边界匹配英文关键词，避免 rain 命中 brain
        let hit = if kw.is_ascii() {
            let padded = format!(" {} ", &lower);
            let kw_padded = format!(" {} ", kw);
            padded.contains(&kw_padded)
                || lower.starts_with(&format!("{kw} "))
                || lower.ends_with(&format!(" {kw}"))
                || lower == *kw
        } else {
            lower.contains(kw)
        };
        if hit {
            let entry = out.iter_mut().find(|s| s.name == *tag);
            match entry {
                Some(e) => e.score += 1,
                None => out.push(TagSuggestionDto { name: tag.to_string(), score: 1 }),
            }
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score));
    out
}

/// 为条目生成打标建议（文件名 + 父目录路径，不含盘符）
pub fn suggest_tags(track_ids: &[String]) -> Vec<(String, Vec<TagSuggestionDto>)> {
    let all = crate::db::library::load_snapshot(&library_db_path(), true);
    let tracks = match all {
        Some(s) => s.all_tracks,
        None => return vec![],
    };
    let by_id: std::collections::HashMap<String, &crate::model::Track> = tracks.iter().map(|t| (t.id.clone(), t)).collect();
    let existing: std::collections::HashSet<String> = tags_of_tracks(track_ids)
        .values()
        .flat_map(|v| v.iter().map(|t| t.name.clone()))
        .collect();
    let mut out = Vec::new();
    for tid in track_ids {
        let Some(t) = by_id.get(tid) else { continue };
        let mut text = t.path.replace('\\', "/");
        // 去掉盘符
        if let Some(pos) = text.find(":/") {
            text = text[pos + 2..].to_string();
        }
        let suggestions = match_keywords(&text)
            .into_iter()
            .filter(|s| !existing.contains(&s.name))
            .collect();
        out.push((tid.clone(), suggestions));
    }
    out
}

//! 命令层：分类与标签（taxonomy）。
//!
//! 从 `commands/library.rs` 拆出来（账本 §8 第 3 档 **S5**）：曲库那一半是扫描/进度/书签/监听，
//! 这一半是 19 条分类标签命令，两者的依赖面完全不同（前者要 `AppHandle`/`AppWindowsState`，
//! 后者只要一次 `spawn_blocking`），挤在一个 400+ 行文件里只是历史顺序，不是同一种职责。
//! 业务实现在 `crate::taxonomy`，这里只做参数整形与错误收口。

use crate::error::IpcResult;

use super::join_task;
// ── 分类与标签（同样全部走阻塞池）──────────────────────────────

macro_rules! blocking {
    ($expr:expr) => {
        tauri::async_runtime::spawn_blocking(move || $expr).await
    };
}

#[tauri::command]
pub async fn taxonomy_list_categories() -> Vec<crate::taxonomy::CategoryDto> {
    blocking!(crate::taxonomy::build_category_tree(crate::taxonomy::list_categories())).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_create_category(parent_id: Option<i64>, name: String) -> IpcResult<crate::taxonomy::CategoryDto> {
    let trimmed = name.trim().to_string();
    let id = join_task("新建分类", blocking!(crate::taxonomy::create_category(parent_id, &name)))?;
    Ok(crate::taxonomy::CategoryDto {
        id,
        parent_id,
        name: trimmed,
        icon: None,
        color: None,
        sort_index: 0,
        track_count: 0,
        children: vec![],
    })
}

#[tauri::command]
pub async fn taxonomy_rename_category(id: i64, name: String) -> IpcResult<()> {
    join_task("分类改名", blocking!(crate::taxonomy::rename_category(id, &name)))
}

#[tauri::command]
pub async fn taxonomy_delete_category(id: i64) -> IpcResult<()> {
    join_task("删除分类", blocking!(crate::taxonomy::delete_category(id)))
}

#[tauri::command]
pub async fn taxonomy_move_category(id: i64, parent_id: Option<i64>, sort_index: i64) -> IpcResult<()> {
    join_task("移动分类", blocking!(crate::taxonomy::move_category(id, parent_id, sort_index)))
}

#[tauri::command]
pub async fn taxonomy_assign_category(category_id: i64, track_ids: Vec<String>) -> IpcResult<i64> {
    join_task("归入分类", blocking!(crate::taxonomy::assign_category(category_id, &track_ids)))
}

#[tauri::command]
pub async fn taxonomy_unassign_category(category_id: i64, track_ids: Vec<String>) -> IpcResult<i64> {
    join_task("移出分类", blocking!(crate::taxonomy::unassign_category(category_id, &track_ids)))
}

#[tauri::command]
pub async fn taxonomy_list_tags() -> Vec<crate::taxonomy::TagDto> {
    blocking!(crate::taxonomy::list_tags()).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_upsert_tag(name: String, color: Option<String>, kind: Option<String>) -> IpcResult<crate::taxonomy::TagDto> {
    let trimmed = name.trim().to_string();
    let kind_final = kind.unwrap_or_else(|| "topic".into());
    let color2 = color.clone();
    let kind2 = kind_final.clone();
    let id = join_task("保存标签", blocking!(crate::taxonomy::upsert_tag(&name, color2.as_deref(), &kind2)))?;
    Ok(crate::taxonomy::TagDto {
        id,
        name: trimmed,
        color,
        kind: kind_final,
        use_count: 0,
        auto_generated: false,
    })
}

#[tauri::command]
pub async fn taxonomy_rename_tag(id: i64, name: String) -> IpcResult<()> {
    join_task("标签改名", blocking!(crate::taxonomy::rename_tag(id, &name)))
}

#[tauri::command]
pub async fn taxonomy_delete_tag(id: i64) -> IpcResult<()> {
    join_task("删除标签", blocking!(crate::taxonomy::delete_tag(id)))
}

#[tauri::command]
pub async fn taxonomy_tag_tracks(tag_ids: Vec<i64>, track_ids: Vec<String>, mode: Option<String>) -> IpcResult<i64> {
    join_task(
        "批量打标",
        blocking!(crate::taxonomy::tag_tracks(&tag_ids, &track_ids, mode.as_deref().unwrap_or("add"))),
    )
}

#[tauri::command]
pub async fn taxonomy_untag_tracks(tag_id: i64, track_ids: Vec<String>) -> IpcResult<i64> {
    join_task("取消打标", blocking!(crate::taxonomy::untag_tracks(tag_id, &track_ids)))
}

#[tauri::command]
pub async fn taxonomy_suggest_tags(track_ids: Vec<String>) -> Vec<(String, Vec<crate::taxonomy::TagSuggestionDto>)> {
    blocking!(crate::taxonomy::suggest_tags(&track_ids)).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_apply_suggested_tags(accepted: Vec<(String, String)>) -> IpcResult<i64> {
    // accepted: (trackId, tagName) — 先建 tag 再打标
    join_task(
        "应用建议标签",
        tauri::async_runtime::spawn_blocking(move || -> IpcResult<i64> {
            let mut tag_ids: Vec<i64> = Vec::new();
            let names: Vec<String> = accepted.iter().map(|(_, n)| n.clone()).collect();
            for name in &names {
                let id = crate::taxonomy::upsert_tag(name, None, "topic")?;
                if !tag_ids.contains(&id) {
                    tag_ids.push(id);
                }
            }
            let mut track_ids: Vec<String> = accepted.iter().map(|(t, _)| t.clone()).collect();
            track_ids.dedup();
            crate::taxonomy::tag_tracks(&tag_ids, &track_ids, "add")
        })
        .await,
    )
}

#[tauri::command]
pub async fn taxonomy_category_track_ids(category_id: i64, include_children: Option<bool>) -> Vec<String> {
    blocking!(crate::taxonomy::category_track_ids(category_id, include_children.unwrap_or(true))).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_track_tags(track_ids: Vec<String>) -> std::collections::HashMap<String, Vec<crate::taxonomy::TagDto>> {
    blocking!(crate::taxonomy::tags_of_tracks(&track_ids)).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_all_track_tags() -> Vec<(String, i64)> {
    blocking!(crate::taxonomy::all_track_tags()).unwrap_or_default()
}

#[tauri::command]
pub async fn taxonomy_all_category_items() -> Vec<(i64, String)> {
    blocking!(crate::taxonomy::all_category_items()).unwrap_or_default()
}

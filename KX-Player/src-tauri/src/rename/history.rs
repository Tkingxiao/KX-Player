//! `rename_history.json` —— 「原始名 → 新名」的账，**形状是用户脚本的契约**（`04 §7.1`）。
//!
//! ```jsonc
//! { "nor": { "【耳かきASMR】親切なお姉さん…": { "_": "【掏耳ASMR】温柔姐姐…",   // "_" = 文件夹自身
//!                                             "01_xxx.mp3": "01_yyy.mp3" } } } // 其余键 = 文件名
//! ```
//!
//! 分界线与 `preview`/`listing` 一样：本文件的**合并逻辑全是纯函数**（收 `serde_json::Value`，
//! 回 `serde_json::Value`），列目录与读写盘只有最后那三个小函数。批量改名没有第二次机会，
//! 而这份文件是用户跑过 354 个作品攒下来的 —— 「不许把别人的记录挤掉」这件事只能靠用例证明。
//!
//! 三条硬约束（`04 §7.1` 第 2/4/6 条）：
//! 1. 顶层键 = **实际根目录名**，不硬编码 `nor`/`unnor`；
//! 2. 只做「追加/改值」，**不认识的结构原样保留**（用户脚本往里塞的别的键一个都不丢）；
//! 3. 同一个作品多轮改名要**折叠进同一条记录**（键始终是最初那个原始名），否则第 6 条的
//!    「上下文取最终名」就没有唯一答案，用户脚本也会看到一个作品两条互相矛盾的映射。
//!
//! 写盘走 tmp + rename 原子替换（同 `settingsio::save`）。`serde_json` 默认用 `BTreeMap`，
//! 所以重写后键按字典序排 —— JSON 对象的键序不是语义，脚本按名字取值不受影响。

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 用户既有的文件名（`04 §5:13` 之外唯一由用户持有的账本）。
pub const HISTORY_FILE: &str = "rename_history.json";

/// 一条要落进 history 的映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    /// 顶层键：实际根目录名
    pub root: String,
    /// 二级键：作品文件夹的**原始名**（多轮改名后仍是第一次那个名字）
    pub group: String,
    /// `_` = 文件夹自身，其余 = 文件名
    pub key: String,
    /// 新名（不含路径）
    pub to: String,
    /// 本批之前这一格里是什么。`None` = 原来是空的 —— 回滚时**回填这个值**而不是删掉它，
    /// 否则第二轮改名的回滚会把第一轮的记录一起抹掉。
    pub prev: Option<String>,
}

/// 一个作品文件夹（或根目录自身）当前叫 `current`，找出它在 `doc[root]` 里**已有**的组键。
///
/// 命中规则：某组的 `_` 等于 `current`（它被改过名，`current` 是新名），或组键本身就等于
/// `current`（已有一条只登文件的组）。都不命中才用 `current` 开新组 —— 这就是折叠，
/// 也是第 6 条「已有记录就认那条的最终名」的落点。
pub fn group_key(doc: &Value, root: &str, current: &str) -> String {
    let Some(groups) = doc.get(root).and_then(Value::as_object) else {
        return current.to_string();
    };
    for (name, entry) in groups {
        let Some(v) = entry.get("_").and_then(Value::as_str) else {
            continue;
        };
        if v == current {
            return name.clone();
        }
    }
    if groups.contains_key(current) {
        return current.to_string();
    }
    current.to_string()
}

/// 把 deltas 合并进 `doc`。返回**实际写入**的条数（值本来就等于它的算没变）。
///
/// 冲突一律拒绝而不是覆盖：`doc[root]` 或 `doc[root][group]` 已存在却不是对象，说明那是
/// 用户自己的结构，改它 = 破坏脚本。
pub fn merge(doc: &mut Value, deltas: &[Delta]) -> Result<usize, String> {
    let mut written = 0usize;
    for d in deltas {
        let obj = doc.as_object_mut().ok_or("history 顶层不是对象")?;
        let root = obj
            .entry(d.root.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let groups = root
            .as_object_mut()
            .ok_or_else(|| format!("顶层键 {} 不是对象，拒绝覆盖", d.root))?;
        let entry = groups
            .entry(d.group.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let files = entry
            .as_object_mut()
            .ok_or_else(|| format!("{} 下的 {} 不是对象，拒绝覆盖", d.root, d.group))?;
        let prev = files.insert(d.key.clone(), Value::String(d.to.clone()));
        if prev.as_ref().and_then(Value::as_str) != Some(d.to.as_str()) {
            written += 1;
        }
    }
    Ok(written)
}

/// 回滚的 history 侧：**只撤本批写进去、且还没被别人改走的值**。
///
/// 撤的动作看 `prev`：本批之前那格有值就**回填它**（第二轮改名的回滚要回到第一轮的结果，
/// 不能把第一轮的记录一起抹掉），原来是空的才删键。组被删空就删组、根被删空就删根 ——
/// 留下的空壳对用户脚本没有信息量。
/// 返回实际撤销的条数。
pub fn revert(doc: &mut Value, deltas: &[Delta]) -> usize {
    let mut undone = 0usize;
    let mut empty_roots: Vec<String> = Vec::new();
    for d in deltas {
        let Some(root) = doc.get_mut(&d.root).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(entry) = root.get_mut(&d.group).and_then(Value::as_object_mut) else {
            continue;
        };
        if !matches!(entry.get(&d.key), Some(Value::String(v)) if *v == d.to) {
            // 值已经不是本批写的那个（后续批次又改过名）：那条记录现在是别人写的，删它是伪造历史
            continue;
        }
        let mut drop_group = false;
        match &d.prev {
            Some(prev) => {
                entry.insert(d.key.clone(), Value::String(prev.clone()));
            }
            None => {
                entry.remove(&d.key);
                drop_group = entry.is_empty();
            }
        }
        if drop_group {
            root.remove(&d.group);
        }
        if root.is_empty() {
            empty_roots.push(d.root.clone());
        }
        undone += 1;
    }
    if let Some(obj) = doc.as_object_mut() {
        for k in empty_roots {
            obj.remove(&k);
        }
    }
    undone
}

/// 多个根的**共同父目录**下有没有这份文件。没有就不写 —— 到别处凭空造一个同名文件，
/// 比不追加更容易让人以为「我的那份已经更新过了」。
///
/// 根分属不同父目录时也返回 None：一份文件装不下两个平级的账本。
pub fn locate(roots: &[PathBuf]) -> Option<PathBuf> {
    let parent = roots.first()?.parent()?;
    if roots.iter().skip(1).any(|r| r.parent() != Some(parent)) {
        return None;
    }
    let p = parent.join(HISTORY_FILE);
    p.is_file().then_some(p)
}

pub fn load(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("解析失败：{e}"))?;
            if !v.is_object() {
                return Err("内容不是 JSON 对象".into());
            }
            Ok(v)
        }
        // 文件不存在 = 还没有这份账，交出去由调用方决定建不建（不在这儿静默造空档）
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(e) => Err(format!("读取失败：{e}")),
    }
}

/// pretty + tmp/rename 原子替换。写坏用户的账本不在可接受的失败模式里。
pub fn store(path: &Path, doc: &Value) -> Result<(), String> {
    let pretty = serde_json::to_string_pretty(doc).map_err(|e| format!("序列化失败：{e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &pretty).map_err(|e| format!("写入临时文件失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换失败：{e}")
    })
}

#[cfg(test)]
mod tests {
    use super::{group_key, load, locate, merge, revert, store, Delta};
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn d(root: &str, group: &str, key: &str, to: &str) -> Delta {
        Delta { root: root.into(), group: group.into(), key: key.into(), to: to.into(), prev: None }
    }
    fn dp(root: &str, group: &str, key: &str, to: &str, prev: Option<&str>) -> Delta {
        Delta {
            root: root.into(),
            group: group.into(),
            key: key.into(),
            to: to.into(),
            prev: prev.map(str::to_string),
        }
    }

    /// 用户既有的那份（两个作品、一条只有文件夹改名、一条连文件一起）
    fn existing() -> Value {
        json!({ "nor": {
            "旧甲": { "_": "新甲", "01_a~.mp3": "01_a.mp3" },
            "旧乙": { "_": "新乙" }
        } } )
    }

    #[test]
    fn appends_without_disturbing_other_records() {
        let mut doc = existing();
        let before = doc["nor"]["旧乙"].clone();
        assert_eq!(merge(&mut doc, &[d("nor", "旧丙", "_", "新丙")]).unwrap(), 1);
        assert_eq!(doc["nor"]["旧乙"], before, "没参与本批的记录要逐字节等价");
        assert_eq!(doc["nor"]["旧丙"], json!({ "_": "新丙" }));
        assert_eq!(doc["nor"]["旧甲"]["_"], "新甲");
    }

    #[test]
    fn second_round_folds_into_the_original_key() {
        // 新甲 又改成 更甲：不许开出第二条 新甲，用户脚本要的是一个作品一条记录
        let mut doc = existing();
        let group = group_key(&doc, "nor", "新甲");
        assert_eq!(group, "旧甲", "已有一条 _ == 新甲，组键仍是原始名");
        merge(&mut doc, &[d("nor", &group, "_", "更甲")]).unwrap();
        assert_eq!(doc["nor"].as_object().unwrap().len(), 2, "组数不增");
        assert_eq!(doc["nor"]["旧甲"]["_"], "更甲");
        assert_eq!(doc["nor"]["旧甲"]["01_a~.mp3"], "01_a.mp3", "同组里的文件映射不能被挤掉");
    }

    #[test]
    fn file_entries_hang_under_a_group_that_has_no_underscore_yet() {
        // 根目录本身就是作品名（04 §7.1 第 4 条：顶层键取实际根目录名），组键同名但没有 _
        let mut doc = json!({ "nor": {} });
        let group = group_key(&doc, "nor", "nor");
        assert_eq!(group, "nor");
        merge(&mut doc, &[d("nor", &group, "01_x~.mp3", "01_x.mp3")]).unwrap();
        assert_eq!(doc["nor"]["nor"], json!({ "01_x~.mp3": "01_x.mp3" }), "不该凭空造一个 _");
    }

    #[test]
    fn an_unknown_group_name_is_not_folded() {
        let doc = existing();
        assert_eq!(group_key(&doc, "nor", "从没见过"), "从没见过");
        // 换根的账本互不干扰
        assert_eq!(group_key(&doc, "unnor", "新甲"), "新甲");
    }

    #[test]
    fn refuses_to_clobber_a_shape_it_does_not_own() {
        // 用户脚本往顶层塞了个数组：位置被占了，报错而不是覆盖
        let mut doc = json!({ "notes": ["手工记的"] });
        let err = merge(&mut doc, &[d("notes", "甲", "_", "乙")]).unwrap_err();
        assert!(err.contains("不是对象"), "{err}");
        assert_eq!(doc["notes"], json!(["手工记的"]));
    }

    #[test]
    fn rewriting_the_same_value_is_not_counted_as_a_change() {
        let mut doc = existing();
        assert_eq!(merge(&mut doc, &[d("nor", "旧乙", "_", "新乙")]).unwrap(), 0);
    }

    #[test]
    fn revert_removes_only_what_this_batch_wrote() {
        let mut doc = existing();
        merge(&mut doc, &[d("nor", "旧丙", "_", "新丙"), d("nor", "旧丙", "01_c~.mp3", "01_c.mp3")])
            .unwrap();
        let before = doc["nor"]["旧甲"].clone();
        assert_eq!(revert(&mut doc, &[d("nor", "旧丙", "_", "新丙"), d("nor", "旧丙", "01_c~.mp3", "01_c.mp3")]), 2);
        assert!(doc["nor"].get("旧丙").is_none(), "撤空的组要删掉，留空壳没信息量");
        assert_eq!(doc["nor"]["旧甲"], before, "别人的记录一个字都不动");
    }

    #[test]
    fn revert_leaves_a_record_that_someone_else_has_moved_on_from() {
        // 本批写了 新丙，之后又有一批改成 新丁 —— 回滚本批不该把 新丁 一起删了
        let mut doc = json!({ "nor": {} });
        merge(&mut doc, &[d("nor", "旧丙", "_", "新丙")]).unwrap();
        merge(&mut doc, &[d("nor", "旧丙", "_", "新丁")]).unwrap();
        assert_eq!(revert(&mut doc, &[d("nor", "旧丙", "_", "新丙")]), 0);
        assert_eq!(doc["nor"]["旧丙"]["_"], "新丁");
    }

    #[test]
    fn an_emptied_root_is_dropped_too() {
        let mut doc = json!({ "temp": { "甲": { "_": "乙" } } });
        revert(&mut doc, &[d("temp", "甲", "_", "乙")]);
        assert_eq!(doc, json!({}), "本批造出来的顶层键也不该留着");
    }

    #[test]
    fn revert_restores_the_previous_round_instead_of_deleting_it() {
        // 旧甲→新甲 是第一轮的账，本批把 新甲 改成 更甲；回滚要回到 新甲，而不是把记录整条删掉
        let mut doc = existing();
        let prev = doc["nor"]["旧甲"]["_"].as_str().map(str::to_string);
        assert_eq!(prev.as_deref(), Some("新甲"));
        merge(&mut doc, &[dp("nor", "旧甲", "_", "更甲", prev.as_deref())]).unwrap();
        assert_eq!(doc["nor"]["旧甲"]["_"], "更甲");
        assert_eq!(revert(&mut doc, &[dp("nor", "旧甲", "_", "更甲", prev.as_deref())]), 1);
        assert_eq!(doc["nor"]["旧甲"]["_"], "新甲", "回到第一轮的结果，记录还在");
        assert_eq!(doc["nor"]["旧甲"]["01_a~.mp3"], "01_a.mp3");
    }

    #[test]
    fn locate_needs_one_common_parent_and_a_real_file() {
        let dir = std::env::temp_dir().join(format!("kx-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nor")).unwrap();
        std::fs::create_dir_all(dir.join("unnor")).unwrap();
        let roots: Vec<PathBuf> =
            ["nor", "unnor"].iter().map(|n| dir.join(n)).collect();

        assert_eq!(locate(&roots), None, "还没有这份账就不该指着一个不存在的路径写");
        std::fs::write(dir.join("rename_history.json"), "{}").unwrap();
        assert_eq!(locate(&roots), Some(dir.join("rename_history.json")));
        // 根分属两个父目录：一份文件装不下两个平级账本
        assert_eq!(locate(&[dir.join("nor"), dir.join("unnor").join("x")]), None);
        assert_eq!(locate(&[]), None);
        let file = dir.join("rename_history.json");
        assert_eq!(load(&file).unwrap(), json!({}));
        // 读写往返：留 tmp 不留半成品，且第二次写不挤掉第一次的账
        let mut doc = load(&file).unwrap();
        merge(&mut doc, &[d("nor", "旧甲", "_", "新甲")]).unwrap();
        store(&file, &doc).unwrap();
        assert_eq!(load(&file).unwrap(), doc);
        assert!(!dir.join("rename_history.json.tmp").exists(), "临时文件要被 rename 掉");
        let mut again = load(&file).unwrap();
        merge(&mut again, &[d("unnor", "旧乙", "01_b~.mp3", "01_b.mp3")]).unwrap();
        store(&file, &again).unwrap();
        assert_eq!(load(&file).unwrap(), again);
        assert_eq!(load(&file).unwrap()["nor"]["旧甲"]["_"], "新甲", "重写不能丢上一批");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! 未完成动作的**外账**：一个目录、一份 JSON、一次原子写（P1-11）。
//!
//! 改名和旧 ID 迁移都要跨「文件系统 + 曲库 + 设置」三处动东西，而这三处之间没有共同的事务。
//! 崩在中间时，下一次启动看到的是一半改完的盘和一本没写完的账，**事后推不出当时打算做什么**：
//! 盘上少了一个名字、库里多了一行，谁也说不清那是「本来就没改」还是「改了没记账」。
//! 这个目录就是那份线索 —— 动手之前先把意图写下来，收尾时删掉；启动时扫一遍，还在的每一份
//! 都代表一次没有正常结束的操作，交给对应的模块去核对。
//!
//! 只放**机械**（目录、读写、列举）。意图的内容与「怎么核对」属于各自的模块
//! （改名的在 `rename::journal`，旧 ID 迁移的在 `db`）—— 恢复要在各自的数据集上做判断，
//! 塞进一个大 match 只会让两边都不敢改。

use std::path::{Path, PathBuf};

/// 数据目录下的子目录名。放数据目录而不是临时目录：临时目录会被清理程序扫掉，
/// 而这份文件正是崩溃之后唯一还在的线索。
const DIR: &str = "journal";

/// 意图文件归谁看管由调用方决定（改名 / 旧 ID 迁移各用各的前缀）。
pub fn dir() -> PathBuf {
    crate::paths::data_dir().join(DIR)
}

/// 写下意图。**写不进去就不许开工**：没有这份文件的那一批，崩溃后没有任何线索，
/// 调用方拿到 `Err` 就直接退出、一个字节都不动。
pub fn put(dir: &Path, name: &str, body: &str) -> Result<(), String> {
    crate::atomic::write_str(&dir.join(name), body).map_err(|e| format!("写意图文件失败：{e}"))
}

/// 正常收尾后收掉意图。删不掉只是残留：下一次启动会再核对一遍，而核对是幂等的。
pub fn remove(dir: &Path, name: &str) {
    let path = dir.join(name);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            crate::paths::append_log(&format!("[journal] 意图文件删不掉：{e}"));
        }
    }
}

/// 目录里以 `prefix` 开头的意图文件。按文件名排序 —— 恢复顺序稳定，日志也好读。
pub fn list(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(prefix)))
        .collect();
    out.sort();
    out
}

pub fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读取失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::{list, put, read, remove};
    use std::path::PathBuf;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!("kx-journal-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).expect("建测试目录");
            Self(p)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_reads_lists_and_removes_by_prefix() {
        let d = Dir::new("basic");
        put(&d.0, "rename-rb1.json", r#"{"a":1}"#).expect("写");
        put(&d.0, "legacy-ids.json", "{}").expect("写");
        assert_eq!(read(&d.0.join("rename-rb1.json")).unwrap(), r#"{"a":1}"#);

        let only_rename = list(&d.0, "rename-");
        assert_eq!(only_rename.len(), 1, "{only_rename:?}");
        assert!(only_rename[0].ends_with("rename-rb1.json"));
        assert_eq!(list(&d.0, "legacy-").len(), 1);
        assert_eq!(list(&d.0, "没有这个前缀").len(), 0);

        remove(&d.0, "rename-rb1.json");
        assert!(list(&d.0, "rename-").is_empty());
        remove(&d.0, "rename-rb1.json"); // 再删一次不该炸（NotFound 静默）
        assert_eq!(list(&d.0, "").len(), 1, "别人的意图文件一个字都不动");

        // 目录不存在 = 空列表，而不是 panic（首次启动时这个目录还没有）
        assert!(list(&d.0.join("还没有"), "rename-").is_empty());
    }
}

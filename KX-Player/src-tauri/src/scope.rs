//! 路径能力（P0-1）：前端只说「路径」，能不能碰由这里判。
//!
//! 授权的三个来路都在 Rust 侧，JS 伪造不了：
//! 1. 用户在原生对话框里点出来的（`open_*` 当场登记，见 [`crate::commands::dialog_fs`]）；
//! 2. 应用自己的数据目录与封面目录（`lib.rs` 装配时登记）；
//! 3. 曲库里已登记的根目录（从库的 `folderPaths` 元数据读出来 —— 是应用自己写进去的，不是前端递进来的）。
//!
//! 判据落在**规范化后的真路径**上：`canonicalize` 会把 `..` 与软链接/联接点一并解掉，
//! 于是「根/../别人的目录」解完就不在根里，不必再堆一串字符串规则（字符串规则永远是漏的）。
//! 目标还不存在时（新建文件、探测存在性）退一步：向上找**最长的已存在祖先**做规范化，
//! 再把剩余组件接回去 —— 于是「还不存在的路径」也拿得到判据。
//!
//! `..` 一律直接拒，不交给 OS 去词法折叠：折叠规则分平台（Windows 会先把 `..` 折算掉再交给
//! 文件系统，Linux 不会），与其赌它折得对，不如让调用方拿真路径来说话 —— 应用里没有一处需要 `..`。

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

/// 为什么没通过。给日志与错误信封用：能回 `IpcResult` 的命令（改名三条）直接回
/// [`crate::error::IpcError::outside_scope`]，回不了旧形状的那几个（`read_text_file` 这类）
/// 记一行日志 —— 统一错误语义是 P1-6 那一批的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// 不是绝对路径，或含 `..` 的那一截压根不存在 —— 解析不出真路径
    Unresolvable,
    /// 规范化之后不在任何授权根/授权文件里
    Outside,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::Unresolvable => write!(f, "路径无法解析（相对路径、含 `..` 或只剩盘符）"),
            Denied::Outside => write!(f, "不在任何已授权目录里"),
        }
    }
}

/// 已授权的路径集合。读多写少（登记只发生在对话框、装配与扫描时），用 `RwLock` 而不是 `Mutex`。
#[derive(Default)]
pub struct PathScope {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    /// 目录：其下（含子目录）全部可读
    roots: Vec<PathBuf>,
    /// 单个文件：只有它自己可读（对话框多选文件走这条）
    files: HashSet<PathBuf>,
}

impl PathScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个目录。目录必须已存在（对话框刚选中的目录必然存在）。
    pub fn allow_root(&self, dir: &Path) -> bool {
        let Ok(canon) = std::fs::canonicalize(dir) else { return false };
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if !inner.roots.contains(&canon) {
            inner.roots.push(canon);
        }
        true
    }

    /// 撤销一个目录（从曲库移除文件夹时同步收权）。`false` = 它本来就不在集合里。
    pub fn revoke_root(&self, dir: &Path) -> bool {
        let Ok(canon) = std::fs::canonicalize(dir) else { return false };
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let before = inner.roots.len();
        inner.roots.retain(|r| r != &canon);
        inner.roots.len() != before
    }

    /// 登记单个文件（对话框选中的文件：译文导入、图片预览这类只该放行它自己）。
    pub fn allow_file(&self, file: &Path) -> bool {
        let Ok(canon) = std::fs::canonicalize(file) else { return false };
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.files.insert(canon);
        true
    }

    /// 这个路径许不许碰。用于「不存在不算错」的探测类命令（`file_exists`）与只需放行的场景。
    pub fn allows(&self, raw: &str) -> bool {
        resolve_loose(raw).map(|p| self.contains(&p)).unwrap_or(false)
    }

    /// 解析成真路径。命令层拿它去读文件/列目录 —— 过不了就什么都不做。
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, Denied> {
        let p = resolve_loose(raw).ok_or(Denied::Unresolvable)?;
        if self.contains(&p) {
            Ok(p)
        } else {
            Err(Denied::Outside)
        }
    }

    /// 一批路径的整批判决（扫描、改名这类「一次处理一整个列表」的入口用）：
    /// 任一条不过就整批不过 —— 半批放行会让「哪几条真被处理了」变得说不清。
    pub fn check_all(&self, raws: &[String]) -> Result<(), (String, Denied)> {
        for raw in raws {
            self.resolve(raw).map_err(|d| (raw.clone(), d))?;
        }
        Ok(())
    }

    fn contains(&self, p: &Path) -> bool {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        if inner.files.contains(p) {
            return true;
        }
        inner.roots.iter().any(|root| p.starts_with(root))
    }
}

/// 解析成真路径：存在的部分交给 `canonicalize`（顺手解掉软链接/联接点），不存在的尾巴按名字接回去。
/// 接不上（相对路径、`..`、盘符根、空串）返回 `None` —— 一律当作「不可解析」拒掉，不猜。
fn resolve_loose(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let p = Path::new(raw);
    if !p.is_absolute() {
        return None;
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    let mut tail: Vec<OsString> = Vec::new();
    let mut cur: &Path = p;
    loop {
        if let Ok(canon) = std::fs::canonicalize(cur) {
            let mut out = canon;
            for name in tail.iter().rev() {
                out.push(name);
            }
            return Some(out);
        }
        // 走到盘符根还解析不出（理论上到不了：非存在段已被挡掉），或者碰上没名字的组件
        let name = cur.file_name()?.to_os_string();
        tail.push(name);
        cur = cur.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立临时目录树，退出时整体删掉（用例之间不共享路径）
    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let root = std::env::temp_dir().join(format!("kx-scope-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&root).expect("建临时目录");
            Tree(root)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).expect("建子目录");
            p
        }

        fn file(&self, rel: &str, body: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).expect("建父目录");
            }
            std::fs::write(&p, body).expect("写文件");
            p
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_root_opens_itself_and_its_children_only() {
        let t = Tree::new("root");
        let root = t.dir("music");
        let inside = t.file("music/album/a.flac", "x");
        let sibling = t.file("music2/b.flac", "x");
        let outside = t.file("other/c.txt", "x");

        let scope = PathScope::new();
        assert!(scope.allow_root(&root));

        assert!(scope.allows(&inside.to_string_lossy()));
        assert!(scope.allows(&root.to_string_lossy()));
        // `music2` 与 `music` 只差一个字符，按组件比不会误判在里面
        assert!(!scope.allows(&sibling.to_string_lossy()));
        assert!(!scope.allows(&outside.to_string_lossy()));
        assert_eq!(Some(Denied::Outside), scope.resolve(&outside.to_string_lossy()).err());
    }

    #[test]
    fn an_empty_scope_denies_everything() {
        let t = Tree::new("empty");
        let f = t.file("a.txt", "x");
        let scope = PathScope::new();
        assert!(!scope.allows(&f.to_string_lossy()));
    }

    #[test]
    fn traversal_out_of_a_root_is_denied() {
        let t = Tree::new("dotdot");
        let root = t.dir("music");
        let secret = t.file("secret.txt", "别人的文件");

        let scope = PathScope::new();
        assert!(scope.allow_root(&root));

        // 带 `..` 的一律不可解析 —— 不去赌 OS 会不会把它折回根内
        let escape = root.join("..").join("secret.txt");
        assert_eq!(Some(Denied::Unresolvable), scope.resolve(&escape.to_string_lossy()).err());
        // 根外那一份自己（不带 `..`）走的是另一条判据：解析得出来，但不在根里
        assert_eq!(Some(Denied::Outside), scope.resolve(&secret.to_string_lossy()).err());
    }

    #[test]
    fn a_granted_file_does_not_open_its_siblings() {
        let t = Tree::new("file");
        let picked = t.file("inbox/译文.txt", "x");
        let sibling = t.file("inbox/别的.txt", "x");

        let scope = PathScope::new();
        assert!(scope.allow_file(&picked));

        assert!(scope.allows(&picked.to_string_lossy()));
        assert!(!scope.allows(&sibling.to_string_lossy()));
    }

    #[test]
    fn a_target_that_does_not_exist_yet_judges_by_its_parent() {
        let t = Tree::new("new");
        let root = t.dir("music");

        let scope = PathScope::new();
        assert!(scope.allow_root(&root));

        let new_inside = root.join("album").join("还没建的文件.flac");
        assert!(scope.allows(&new_inside.to_string_lossy()), "根内的新路径该放行（转换输出、改名目标都靠它）");
        let new_outside = t.path("outside").join("还没建的文件.flac");
        assert!(!scope.allows(&new_outside.to_string_lossy()));
    }

    #[test]
    fn relative_and_odd_paths_are_unresolvable() {
        let scope = PathScope::new();
        for raw in ["", ".", "..", "music/a.flac", "\\\\?\\", "C:", "nul\0x"] {
            assert!(!scope.allows(raw), "{raw:?} 不该被当成可解析的路径");
        }
        // 盘符根是绝对路径但解析不出（`C:` 的 parent 是 None）
        assert_eq!(Some(Denied::Unresolvable), scope.resolve("C:").err());
    }

    #[test]
    fn a_dotdot_anywhere_is_unresolvable_even_inside_a_root() {
        let t = Tree::new("tail");
        let root = t.dir("music");
        let scope = PathScope::new();
        assert!(scope.allow_root(&root));

        // 尾巴里带 `..`：即使折完还落在根内也不放行 —— 这条规则要的是**判断只看字面量**，
        // 不随平台折叠行为变化（Windows 折、Linux 不折，赌哪个都是在给自己埋雷）
        let raw = root.join("不存在").join("..").join("..").join("secret.txt");
        assert_eq!(Some(Denied::Unresolvable), scope.resolve(&raw.to_string_lossy()).err());
    }

    #[test]
    fn check_all_rejects_the_whole_batch_when_one_path_is_out() {
        let t = Tree::new("batch");
        let root = t.dir("music");
        let inside = t.file("music/a.flac", "x");
        let outside = t.file("other/b.flac", "x");

        let scope = PathScope::new();
        assert!(scope.allow_root(&root));

        assert!(scope.check_all(&[inside.to_string_lossy().into_owned()]).is_ok());
        let bad = scope
            .check_all(&[inside.to_string_lossy().into_owned(), outside.to_string_lossy().into_owned()])
            .unwrap_err();
        assert_eq!(outside.to_string_lossy(), bad.0);
        assert_eq!(Denied::Outside, bad.1);
    }

    #[test]
    fn revoking_a_root_closes_it_again() {
        let t = Tree::new("revoke");
        let root = t.dir("music");
        let inside = t.file("music/a.flac", "x");

        let scope = PathScope::new();
        assert!(scope.allow_root(&root));
        assert!(scope.allows(&inside.to_string_lossy()));

        assert!(scope.revoke_root(&root));
        assert!(!scope.allows(&inside.to_string_lossy()));
        // 再撤一次是「本来就不在」，不是错误
        assert!(!scope.revoke_root(&root));
    }
}

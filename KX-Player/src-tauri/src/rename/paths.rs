//! 动手之前的**路径机械**：执行顺序、现名换算、一次不会覆盖任何东西的改名。
//!
//! 三件事合在一个文件里，因为它们回答的是同一个问题 ——「预览那一刻记下的绝对路径，现在还算
//! 不算数」。所选根相互嵌套时父目录会先被改掉，于是子项的两端都变了；换算只需要知道本批**已经
//! 改成**的那些，不必回头问磁盘。`apply.rs`（正向）与 `rollback.rs`（反向）共用这一套，区别只在方向。

use std::path::{Path, PathBuf};

use crate::rename::batch::{Pair, FOLDER};

/// `04 §7.7` 的执行顺序：文件夹在前、文件在后。应用时文件夹**自底向上**（深目录先，免得父目录
/// 一改子路径全失效）；回滚时**自顶向下**（浅的先回）—— 父目录先搬回原始名，它下面每一条存的
/// `old_path`/`new_path` 就正好是**此刻真实**的路径，不需要拿同批的其它行去换算现名。
/// 两种顺序的**终局完全一样**，选这条是因为账本只有两列路径，换算出来的东西不该靠推测。
pub(crate) fn sort_pairs(v: &mut [Pair], undo: bool) {
    v.sort_by(|a, b| {
        let folders_first = (b.level == FOLDER).cmp(&(a.level == FOLDER));
        let (da, db) = (depth(&a.old), depth(&b.old));
        folders_first.then_with(|| if undo { da.cmp(&db) } else { db.cmp(&da) }).then_with(|| a.old.cmp(&b.old))
    });
}

fn depth(p: &str) -> usize {
    Path::new(p).components().count()
}

/// 把「预览时的绝对路径」按已发生的改名换算成**现在**的路径（按路径分量比，
/// 所以 `nor` 不会命中 `northern`）。
pub(crate) fn resolve(path: &str, moved: &[(String, String)]) -> String {
    let mut cur = PathBuf::from(path);
    for (from, to) in moved {
        if let Ok(rest) = cur.strip_prefix(from) {
            cur = Path::new(to).join(rest);
        }
    }
    cur.to_string_lossy().into_owned()
}

/// 回滚/重做没有「用户选了哪些根」这个输入（只有库里的路径），用条目的父目录代替。
pub(crate) fn parents_of(pairs: &[Pair]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pairs {
        if let Some(dir) = Path::new(&p.old).parent() {
            let s = dir.to_string_lossy().into_owned();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// 一次改名。**动手前重新检测冲突**：预览之后盘上可能又多出一个同名的来路。
pub(crate) fn move_one(from: &str, to: &str) -> Result<(), String> {
    if !Path::new(from).exists() {
        return Err("原路径已不存在".into());
    }
    if Path::new(to).exists() {
        return Err("目标名已被占用，未覆盖".into());
    }
    rename_noreplace(Path::new(from), Path::new(to)).map_err(|e| {
        // 目标在这一次 check 与 rename 之间被别人占住：报成「已占用」而不是一句笼统的改名失败
        if Path::new(to).exists() {
            "目标名已被占用，未覆盖".to_string()
        } else {
            format!("改名失败：{e}")
        }
    })
}

/// 搬一个名字，**目标已存在就失败**（P1-11）。
///
/// `std::fs::rename` 在 Windows 上是带 `MOVEFILE_REPLACE_EXISTING` 的 `MoveFileExW`：
/// 目标存在就**覆盖**。`move_one` 上面那两句 `exists` 只是快速拒绝 —— 检查和改名的空隙里
/// 被人塞进一个同名文件，旧实现就把那个文件无声吞掉了（TOCTOU）。所以真正的保证在这一层：
/// 走不带 REPLACE 的 `MoveFileW`，目标存在即失败，检查与动作之间的空隙就不再有后果。
#[cfg(windows)]
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }
    let (f, t) = (wide(from), wide(to));
    // SAFETY: 两个指针都指向以 NUL 结尾的、在调用期间存活的宽字符串
    let ok = unsafe { MoveFileW(f.as_ptr(), t.as_ptr()) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 非 Windows 构建不发布（本产品只有 Windows 版），这里退回普通 `rename` ——
/// 调用方那两句 `exists` 已经是这一层能做的最好，POSIX 没有「不覆盖」的标准 rename 入口。
#[cfg(not(windows))]
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::{move_one, rename_noreplace, resolve, sort_pairs};
    use crate::rename::batch::{Pair, FILE, FOLDER};
    use std::path::{Path, PathBuf};

    /// 三种 separator 混用的显示串，比对前统一成 `/`
    fn sl(p: &str) -> String {
        p.replace('\\', "/")
    }

    fn pair(old: &str, new: &str, level: &'static str) -> Pair {
        Pair { old: old.into(), new: new.into(), level }
    }

    fn tree(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("kx-paths-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("临时目录");
        p
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .expect("列目录")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn resolve_follows_folder_renames_component_by_component() {
        let moved = vec![("L:/nor/甲".to_string(), "L:/nor/乙".to_string())];
        assert_eq!(sl(&resolve("L:/nor/甲/f~.mp3", &moved)), "L:/nor/乙/f~.mp3");
        assert_eq!(
            sl(&resolve("L:/northern/甲/f~.mp3", &moved)),
            "L:/northern/甲/f~.mp3",
            "按路径分量比：nor 不能命中 northern"
        );
        assert_eq!(sl(&resolve("L:/其它/甲/f~.mp3", &moved)), "L:/其它/甲/f~.mp3", "不相干的根不动");
    }

    /// `move_one` 上面那两句 `exists` 挡不住「检查之后、改名之前」塞进来的同名文件 ——
    /// 真正的保证来自底层那一下不带 REPLACE 的 `MoveFileW`。这里绕过上层检查直接钉住它：
    /// 目标已存在时必须失败，且目标的内容一个字都不能变（旧实现会把它覆盖掉）。
    #[test]
    fn the_move_primitive_never_replaces_an_existing_target() {
        let base = tree("noreplace");
        std::fs::create_dir(base.join("占位的目录")).unwrap();
        std::fs::create_dir(base.join("要搬的目录")).unwrap();
        std::fs::write(base.join("占用者.txt"), "别人的内容").unwrap();
        std::fs::write(base.join("源.txt"), "我的内容").unwrap();

        assert!(rename_noreplace(&base.join("源.txt"), &base.join("占用者.txt")).is_err());
        assert_eq!(std::fs::read_to_string(base.join("占用者.txt")).unwrap(), "别人的内容", "目标不许被覆盖");
        assert_eq!(std::fs::read_to_string(base.join("源.txt")).unwrap(), "我的内容", "源还在原地");

        // 目录同理（改名主链里一半的条目是文件夹）
        assert!(rename_noreplace(&base.join("要搬的目录"), &base.join("占位的目录")).is_err());
        assert!(base.join("要搬的目录").is_dir() && base.join("占位的目录").is_dir());

        // 目标不在时才搬得动
        rename_noreplace(&base.join("源.txt"), &base.join("新名.txt")).expect("正常改名");
        assert_eq!(names(&base), ["占位的目录", "占用者.txt", "新名.txt", "要搬的目录"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn move_one_renames_but_never_overwrites_or_forges() {
        let base = tree("move");
        std::fs::write(base.join("占用者.txt"), "x").unwrap();
        std::fs::write(base.join("源.txt"), "y").unwrap();
        let p = |n: &str| base.join(n).to_string_lossy().into_owned();

        let e = move_one(&p("不存在.txt"), &p("目标.txt")).unwrap_err();
        assert!(e.contains("原路径已不存在"), "{e}");
        let e = move_one(&p("源.txt"), &p("占用者.txt")).unwrap_err();
        assert!(e.contains("目标名已被占用"), "{e}");
        assert!(Path::new(&p("源.txt")).exists(), "被拒的那次必须一个字都没动，而不是把源文件吞了");

        move_one(&p("源.txt"), &p("目标.txt")).expect("正常改名");
        // 期望值也排序：文件系统不保证顺序，断言里写的是「有哪些」
        assert_eq!(names(&base), ["占用者.txt", "目标.txt"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 回滚的排序为什么是「文件夹自顶向下、文件最后」：这样每一条动手时，它自己那两列路径里的
    /// 现名就是**此刻盘上的名字**，不需要拿同批的其它行去换算。顺序错了的表现是「现名已不存在」。
    #[test]
    fn undo_order_puts_shallow_folders_first_and_files_last() {
        let mut v = vec![
            pair("R:/甲/乙/歌.srt", "R:/甲/乙/歌.srt.new", FILE),
            pair("R:/甲/乙", "R:/甲/乙名", FOLDER),
            pair("R:/甲", "R:/改名", FOLDER),
        ];
        sort_pairs(&mut v, true);
        assert_eq!(
            v.iter().map(|p| sl(&p.old)).collect::<Vec<_>>(),
            ["R:/甲", "R:/甲/乙", "R:/甲/乙/歌.srt"],
            "浅的先回，文件垫底"
        );
        sort_pairs(&mut v, false);
        assert_eq!(
            v.iter().map(|p| sl(&p.old)).collect::<Vec<_>>(),
            ["R:/甲/乙", "R:/甲", "R:/甲/乙/歌.srt"],
            "应用时反过来：深的先改，文件仍然最后"
        );
    }
}
